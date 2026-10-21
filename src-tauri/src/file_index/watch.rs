//! The index's background thread: load the saved copy, rescan, then keep the
//! index current from file system change events.
//!
//! Each root is watched on its own (not recursively), and each indexed folder
//! directly inside it is watched recursively. Hidden folders like AppData are
//! never indexed, so their constant churn never reaches us.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use super::{scan, snapshot, FileIndex, Tree, MAX_ENTRIES};

/// Wait for this much quiet before applying a burst of changes.
const DEBOUNCE: Duration = Duration::from_millis(500);
/// ...but never hold changes back longer than this.
const MAX_BATCH: Duration = Duration::from_secs(3);
/// Save to disk at most this often when something changed.
const SAVE_EVERY: Duration = Duration::from_secs(300);
/// When the OS says events were lost, rescan everything, but not more often.
const MIN_RESCAN_GAP: Duration = Duration::from_secs(600);
/// How often a scan reports progress (and, on the first scan, adds results).
const PROGRESS_EVERY: Duration = Duration::from_millis(300);

enum Change {
    Paths(Vec<PathBuf>),
    /// Events were dropped (buffer overflow): the index may be out of date.
    Rescan,
}

pub(super) fn run(index: Arc<FileIndex>) {
    if let Some(tree) = snapshot::load(&index.snapshot_path, &index.roots) {
        tracing::info!("file index: {} entries loaded from disk", count(&tree));
        index.replace_all(tree);
        index
            .dirty
            .store(false, std::sync::atomic::Ordering::Release);
        index.set_incomplete(false);
    }

    // Watch before scanning so changes made during the scan queue up.
    let (tx, rx) = mpsc::channel();
    let mut watches = Watches::new(tx);
    full_scan(&index);
    let mut last_full_scan = Instant::now();
    if let Some(w) = watches.as_mut() {
        w.sync(&index);
    }

    let mut last_save = Instant::now();
    loop {
        match next_batch(&rx) {
            Ok(batch) => {
                if batch.iter().any(|c| matches!(c, Change::Rescan)) {
                    if last_full_scan.elapsed() > MIN_RESCAN_GAP {
                        full_scan(&index);
                        last_full_scan = Instant::now();
                    }
                } else {
                    apply(&index, batch);
                }
                if let Some(w) = watches.as_mut() {
                    w.sync(&index);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if last_save.elapsed() >= SAVE_EVERY {
            index.save_if_dirty();
            last_save = Instant::now();
        }
    }
    index.save_if_dirty();
}

/// Waits for a change, then gathers the rest of the burst.
fn next_batch(rx: &Receiver<Change>) -> Result<Vec<Change>, RecvTimeoutError> {
    let first = rx.recv_timeout(SAVE_EVERY)?;
    let started = Instant::now();
    let mut batch = vec![first];
    while started.elapsed() < MAX_BATCH {
        match rx.recv_timeout(DEBOUNCE) {
            Ok(change) => batch.push(change),
            Err(_) => break,
        }
    }
    Ok(batch)
}

fn full_scan(index: &FileIndex) {
    let started = Instant::now();
    index.set_scanning(true);
    // With nothing indexed yet, add folders as they're read so search works
    // during the first scan. Otherwise build the new tree on the side and
    // swap it in at the end, so deleted files drop out without a gap.
    let fill_live = index.is_empty();
    let mut tree = Tree::new();
    let mut budget = MAX_ENTRIES;
    let mut total = 0;
    for root in &index.roots {
        let read = scan::walk_in_chunks(
            Path::new(root),
            &index.rules,
            budget,
            PROGRESS_EVERY,
            |chunk| {
                if fill_live {
                    index.insert_tree(chunk);
                    index.report();
                } else {
                    tree.extend(chunk);
                }
            },
        );
        budget = budget.saturating_sub(read);
        total += read;
    }
    if !fill_live {
        index.replace_all(tree);
    }
    index.save_if_dirty();
    index
        .scanning
        .store(false, std::sync::atomic::Ordering::Release);
    index.set_incomplete(false);
    tracing::info!(
        "file index: {total} entries scanned in {:?}",
        started.elapsed()
    );
}

/// Re-reads the folders the changes happened in, then indexes new folders
/// and drops deleted ones.
fn apply(index: &FileIndex, batch: Vec<Change>) {
    let paths: BTreeSet<PathBuf> = batch
        .into_iter()
        .flat_map(|c| match c {
            Change::Paths(paths) => paths,
            Change::Rescan => Vec::new(),
        })
        .collect();

    // Only folders already in the index matter; this filters out everything
    // inside hidden or excluded folders.
    let parents: BTreeSet<PathBuf> = paths
        .iter()
        .filter_map(|p| p.parent())
        .filter(|parent| index.has_dir(&parent.to_string_lossy()))
        .map(Path::to_path_buf)
        .collect();
    for parent in &parents {
        let key = parent.to_string_lossy().into_owned();
        match scan::list_dir(parent, &index.rules) {
            Ok(entries) => index.set_dir(key, entries),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => index.remove_subtree(&key),
            Err(e) => tracing::debug!("could not re-read {key}: {e}"),
        }
    }

    for path in &paths {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            continue;
        };
        let parent_key = parent.to_string_lossy();
        // Only judge paths inside indexed folders. A root's own parent isn't
        // indexed, so a change to a root never reads as "root deleted".
        if !index.has_dir(&parent_key) {
            continue;
        }
        let key = path.to_string_lossy().into_owned();
        let listed = index.lists_folder(&parent_key, &name.to_string_lossy());
        if listed && !index.has_dir(&key) && !index.rules.excludes_dir(&key) {
            // A new (or moved-in) folder: index everything inside it.
            index.insert_tree(scan::walk(path, &index.rules, MAX_ENTRIES));
        } else if !listed && index.has_dir(&key) {
            // Deleted or moved away.
            index.remove_subtree(&key);
        }
    }
}

fn count(tree: &Tree) -> usize {
    tree.values().map(Vec::len).sum()
}

/// The set of OS watches, kept matching the index's roots and top folders.
struct Watches {
    watcher: RecommendedWatcher,
    watched: BTreeSet<(PathBuf, bool)>,
}

impl Watches {
    fn new(tx: Sender<Change>) -> Option<Self> {
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let change = match event {
                Ok(e) if e.need_rescan() => Change::Rescan,
                Ok(e) => Change::Paths(e.paths),
                Err(e) => {
                    tracing::debug!("file watch error: {e}");
                    return;
                }
            };
            let _ = tx.send(change);
        });
        match watcher {
            Ok(watcher) => Some(Self {
                watcher,
                watched: BTreeSet::new(),
            }),
            Err(e) => {
                tracing::warn!("file index won't update live: {e}");
                None
            }
        }
    }

    /// Roots non-recursively; each indexed folder directly inside a root
    /// recursively.
    fn sync(&mut self, index: &FileIndex) {
        let mut wanted = BTreeSet::new();
        for root in &index.roots {
            wanted.insert((PathBuf::from(root), false));
            for child in index.child_folders(root) {
                wanted.insert((PathBuf::from(child), true));
            }
        }
        for (path, recursive) in self.watched.difference(&wanted) {
            let _ = self.watcher.unwatch(path);
            tracing::debug!(
                "stopped watching {} (recursive: {recursive})",
                path.display()
            );
        }
        for (path, recursive) in wanted.difference(&self.watched) {
            let mode = if *recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            if let Err(e) = self.watcher.watch(path, mode) {
                tracing::debug!("could not watch {}: {e}", path.display());
            }
        }
        self.watched = wanted;
    }
}
