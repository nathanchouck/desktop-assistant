//! The launcher's own file index: the names of files and folders under the
//! folders you choose (default: your user folder), held in memory, saved to
//! disk between runs, and kept current by watching for changes. File search
//! uses it whenever Everything isn't running.
//!
//! Layout: folder path → its direct children. A change only ever re-reads
//! the folder it happened in.

mod rules;
mod scan;
mod snapshot;
mod watch;

pub use rules::Rules;

use std::cmp::Ordering as CmpOrdering;
use std::collections::{BTreeMap, BinaryHeap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::platform::FileHit;

/// Stop reading after this many entries, to keep memory bounded.
const MAX_ENTRIES: usize = 2_000_000;
/// Below this many entries one thread searches faster than several.
const PARALLEL_FROM: usize = 20_000;
const MAX_THREADS: usize = 8;
/// How many entries a search scores between checks for cancellation.
const CANCEL_CHECK_EVERY: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub name: Box<str>,
    pub is_folder: bool,
    /// Last modified, Unix seconds.
    pub modified: Option<i64>,
}

/// Folder path → its direct children.
pub type Tree = BTreeMap<String, Vec<IndexEntry>>;

/// How far along the index is, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct IndexStatus {
    /// A full scan is reading the disk.
    pub scanning: bool,
    /// Some files can't be found yet: the index is loading, or the first
    /// scan is still running with no saved index to start from.
    pub incomplete: bool,
    /// Files and folders indexed so far.
    pub entries: u32,
}

type StatusListener = Box<dyn Fn(IndexStatus) + Send + Sync>;

pub struct FileIndex {
    roots: Vec<String>,
    rules: Rules,
    snapshot_path: PathBuf,
    tree: RwLock<Tree>,
    started: AtomicBool,
    dirty: AtomicBool,
    scanning: AtomicBool,
    incomplete: AtomicBool,
    listener: OnceLock<StatusListener>,
}

impl FileIndex {
    pub fn new(roots: &[PathBuf], exclusions: &[String], snapshot_path: PathBuf) -> Self {
        Self {
            roots: roots
                .iter()
                .map(|r| r.to_string_lossy().into_owned())
                .collect(),
            rules: Rules::new(exclusions),
            snapshot_path,
            tree: RwLock::new(Tree::new()),
            started: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            scanning: AtomicBool::new(false),
            incomplete: AtomicBool::new(false),
            listener: OnceLock::new(),
        }
    }

    /// Calls `listener` whenever the status changes (at most a few times a
    /// second during a scan). Set once, before the index starts.
    pub fn on_status(&self, listener: impl Fn(IndexStatus) + Send + Sync + 'static) {
        let _ = self.listener.set(Box::new(listener));
    }

    pub fn status(&self) -> IndexStatus {
        IndexStatus {
            scanning: self.scanning.load(Ordering::Acquire),
            incomplete: self.incomplete.load(Ordering::Acquire),
            entries: u32::try_from(self.len()).unwrap_or(u32::MAX),
        }
    }

    fn report(&self) {
        if let Some(listener) = self.listener.get() {
            listener(self.status());
        }
    }

    fn set_scanning(&self, scanning: bool) {
        self.scanning.store(scanning, Ordering::Release);
        self.report();
    }

    fn set_incomplete(&self, incomplete: bool) {
        self.incomplete.store(incomplete, Ordering::Release);
        self.report();
    }

    /// Loads, scans and starts watching on a background thread, once. Later
    /// calls do nothing, so it's cheap to call on every search.
    pub fn ensure_started(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        // Until the saved index is loaded or the first scan ends.
        self.set_incomplete(true);
        let index = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("file-index".into())
            .spawn(move || watch::run(index));
        if let Err(e) = spawned {
            crate::toast::report(format!("Couldn't start the file index: {e}"));
            self.started.store(false, Ordering::Release);
            self.set_incomplete(false);
        }
    }

    /// Number of files and folders indexed.
    pub fn len(&self) -> usize {
        self.tree
            .read()
            .map_or(0, |t| t.values().map(Vec::len).sum())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `max` best entries by score (`None` = not a match), best first.
    /// `make_scorer` builds one scorer per thread: big indexes are split
    /// across CPU cores. `cancelled` is checked as it goes, so a search
    /// for a query you've already typed past stops early.
    pub fn search<S>(
        &self,
        make_scorer: impl Fn() -> S + Sync,
        max: usize,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Vec<FileHit>
    where
        S: FnMut(&IndexEntry) -> Option<f32>,
    {
        if max == 0 {
            return Vec::new();
        }
        let Ok(tree) = self.tree.read() else {
            return Vec::new();
        };
        let dirs: Vec<(&str, &[IndexEntry])> = tree
            .iter()
            .map(|(dir, entries)| (dir.as_str(), entries.as_slice()))
            .collect();
        let total: usize = dirs.iter().map(|(_, e)| e.len()).sum();
        let threads = if total < PARALLEL_FROM {
            1
        } else {
            // Leave a core for the UI and the keyboard hook.
            std::thread::available_parallelism()
                .map_or(1, std::num::NonZeroUsize::get)
                .saturating_sub(1)
                .clamp(1, MAX_THREADS)
        };

        let mut ranked: Vec<Ranked<'_>> = if threads == 1 {
            best_of(&dirs, make_scorer(), max, cancelled)
        } else {
            let parts = split_evenly(&dirs, threads, total);
            std::thread::scope(|scope| {
                let runs: Vec<_> = parts
                    .iter()
                    .map(|part| scope.spawn(|| best_of(part, make_scorer(), max, cancelled)))
                    .collect();
                runs.into_iter()
                    .filter_map(|run| run.join().ok())
                    .flatten()
                    .collect()
            })
        };
        if cancelled() {
            return Vec::new();
        }
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
        ranked.truncate(max);
        ranked
            .into_iter()
            .map(|r| FileHit {
                path: Path::new(r.dir)
                    .join(&*r.entry.name)
                    .to_string_lossy()
                    .into_owned(),
                name: r.entry.name.to_string(),
                dir: r.dir.to_owned(),
                is_folder: r.entry.is_folder,
                modified: r.entry.modified,
            })
            .collect()
    }

    fn replace_all(&self, tree: Tree) {
        if let Ok(mut t) = self.tree.write() {
            *t = tree;
        }
        self.dirty.store(true, Ordering::Release);
    }

    fn set_dir(&self, dir: String, entries: Vec<IndexEntry>) {
        if let Ok(mut t) = self.tree.write() {
            t.insert(dir, entries);
        }
        self.dirty.store(true, Ordering::Release);
    }

    fn insert_tree(&self, tree: Tree) {
        if let Ok(mut t) = self.tree.write() {
            t.extend(tree);
        }
        self.dirty.store(true, Ordering::Release);
    }

    /// Drops a folder and everything indexed under it.
    fn remove_subtree(&self, dir: &str) {
        let Ok(mut t) = self.tree.write() else { return };
        let prefix = Path::new(dir).join("").to_string_lossy().into_owned();
        let doomed: Vec<String> = t
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(k, _)| k.clone())
            .collect();
        for key in doomed {
            t.remove(&key);
        }
        t.remove(dir);
        self.dirty.store(true, Ordering::Release);
    }

    fn has_dir(&self, dir: &str) -> bool {
        self.tree.read().is_ok_and(|t| t.contains_key(dir))
    }

    /// Whether `parent`'s listing has a folder called `name`.
    fn lists_folder(&self, parent: &str, name: &str) -> bool {
        self.tree.read().is_ok_and(|t| {
            t.get(parent)
                .is_some_and(|entries| entries.iter().any(|e| e.is_folder && &*e.name == name))
        })
    }

    /// Indexed folders directly inside `dir`.
    fn child_folders(&self, dir: &str) -> Vec<String> {
        let Ok(t) = self.tree.read() else {
            return Vec::new();
        };
        t.get(dir)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.is_folder)
                    .map(|e| Path::new(dir).join(&*e.name).to_string_lossy().into_owned())
                    .filter(|p| t.contains_key(p))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn save_if_dirty(&self) {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return;
        }
        let Ok(tree) = self.tree.read() else { return };
        if let Err(e) = snapshot::save(&self.snapshot_path, &self.roots, &tree) {
            crate::toast::report(format!("Couldn't save the file index: {e}"));
        }
    }
}

struct Ranked<'a> {
    score: f32,
    dir: &'a str,
    entry: &'a IndexEntry,
}

/// The `max` best of `dirs`, unordered (empty once `cancelled`).
fn best_of<'a>(
    dirs: &[(&'a str, &'a [IndexEntry])],
    mut score: impl FnMut(&IndexEntry) -> Option<f32>,
    max: usize,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Vec<Ranked<'a>> {
    // Min-heap of the best so far: the worst kept hit is on top.
    let mut best: BinaryHeap<Ranked<'a>> = BinaryHeap::with_capacity(max + 1);
    let mut scored = 0usize;
    for &(dir, entries) in dirs {
        scored += entries.len();
        if scored >= CANCEL_CHECK_EVERY {
            scored = 0;
            if cancelled() {
                return Vec::new();
            }
        }
        for entry in entries {
            let Some(s) = score(entry) else { continue };
            if best.len() == max && best.peek().is_some_and(|worst| s <= worst.score) {
                continue;
            }
            best.push(Ranked {
                score: s,
                dir,
                entry,
            });
            if best.len() > max {
                best.pop();
            }
        }
    }
    best.into_vec()
}

/// Splits `dirs` into `parts` runs of about the same number of entries.
fn split_evenly<'s, 'a>(
    dirs: &'s [(&'a str, &'a [IndexEntry])],
    parts: usize,
    total: usize,
) -> Vec<&'s [(&'a str, &'a [IndexEntry])]> {
    let target = total.div_ceil(parts.max(1)).max(1);
    let mut out = Vec::with_capacity(parts);
    let mut start = 0;
    let mut size = 0;
    for (i, (_, entries)) in dirs.iter().enumerate() {
        size += entries.len();
        if size >= target {
            out.push(&dirs[start..=i]);
            start = i + 1;
            size = 0;
        }
    }
    if start < dirs.len() {
        out.push(&dirs[start..]);
    }
    out
}

// Reversed, so `BinaryHeap` (a max-heap) keeps the lowest score on top.
impl Ord for Ranked<'_> {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other.score.total_cmp(&self.score)
    }
}

impl PartialOrd for Ranked<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Ranked<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == CmpOrdering::Equal
    }
}

impl Eq for Ranked<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};

    fn entry(name: &str, is_folder: bool) -> IndexEntry {
        IndexEntry {
            name: name.into(),
            is_folder,
            modified: None,
        }
    }

    fn never() -> bool {
        false
    }

    fn index_with(tree: Tree) -> FileIndex {
        let index = FileIndex::new(&[], &[], PathBuf::from("unused"));
        index.replace_all(tree);
        index
    }

    #[test]
    fn keeps_the_best_matches_in_order() {
        let mut tree = Tree::new();
        tree.insert(
            "A".into(),
            vec![entry("aa", false), entry("aaaa", false), entry("b", false)],
        );
        tree.insert("B".into(), vec![entry("aaa", true)]);
        let index = index_with(tree);
        // Score = number of a's; "b" doesn't match.
        let score = |e: &IndexEntry| {
            let n = e.name.matches('a').count();
            (n > 0).then_some(n as f32)
        };
        let names: Vec<String> = index
            .search(|| score, 2, &never)
            .into_iter()
            .map(|h| h.name)
            .collect();
        assert_eq!(names, ["aaaa", "aaa"]);
        let hit = &index.search(|| score, 1, &never)[0];
        assert_eq!(hit.path, Path::new("A").join("aaaa").to_string_lossy());
        assert_eq!(index.len(), 4);
    }

    /// Enough entries to take the multi-threaded path.
    fn big_tree() -> Tree {
        let mut tree = Tree::new();
        for d in 0..500 {
            let entries = (0..100)
                .map(|i| entry(&format!("file-{d}-{i}.txt"), false))
                .collect();
            tree.insert(format!("dir{d:03}"), entries);
        }
        tree
    }

    #[test]
    fn parallel_search_finds_the_same_best_hits() {
        let index = index_with(big_tree());
        // Higher numbers score higher; the best are in the last folders.
        let score = |e: &IndexEntry| {
            let n: f32 = e.name.split('-').nth(1)?.parse().ok()?;
            Some(n)
        };
        let hits = index.search(|| score, 3, &never);
        assert_eq!(hits.len(), 3);
        assert!(
            hits.iter().all(|h| h.name.starts_with("file-499-")),
            "{hits:?}"
        );
    }

    #[test]
    fn a_cancelled_search_returns_nothing() {
        let index = index_with(big_tree());
        let hits = index.search(|| |_: &IndexEntry| Some(1.0), 10, &|| true);
        assert!(hits.is_empty());
    }

    #[test]
    fn splits_work_evenly() {
        let a = [entry("x", false), entry("y", false)];
        let b = [entry("z", false)];
        let dirs: Vec<(&str, &[IndexEntry])> = vec![("a", &a), ("b", &b), ("c", &a), ("d", &b)];
        let parts = split_evenly(&dirs, 2, 6);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts.iter().map(|p| p.len()).sum::<usize>(), 4);
    }

    #[test]
    fn removes_whole_subtrees_only() {
        let sep = std::path::MAIN_SEPARATOR;
        let mut tree = Tree::new();
        for dir in [
            "r",
            &format!("r{sep}a"),
            &format!("r{sep}a{sep}b"),
            &format!("r{sep}ab"),
        ] {
            tree.insert(dir.to_string(), Vec::new());
        }
        let index = index_with(tree);
        index.remove_subtree(&format!("r{sep}a"));
        let left: Vec<String> = index.tree.read().unwrap().keys().cloned().collect();
        assert_eq!(left, ["r".to_string(), format!("r{sep}ab")]);
    }

    fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if check() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("timed out waiting for {what}");
    }

    fn find(index: &FileIndex, name: &str) -> bool {
        !index
            .search(
                || |e: &IndexEntry| (&*e.name == name).then_some(1.0),
                1,
                &never,
            )
            .is_empty()
    }

    /// End to end on the real file system: scan, then pick up new files,
    /// new folders and deletions through the watcher.
    #[test]
    fn follows_changes_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("home");
        fs::create_dir_all(root.join("Documents")).unwrap();
        fs::write(root.join("Documents/old.txt"), b"x").unwrap();
        let index = Arc::new(FileIndex::new(
            std::slice::from_ref(&root),
            &[],
            dir.path().join("index.txt"),
        ));
        index.ensure_started();
        wait_for("initial scan", || find(&index, "old.txt"));
        // Give the watcher a moment to arm after the scan.
        std::thread::sleep(Duration::from_millis(500));

        fs::write(root.join("Documents/new.txt"), b"x").unwrap();
        wait_for("new file", || find(&index, "new.txt"));

        fs::create_dir_all(root.join("Projects/site")).unwrap();
        fs::write(root.join("Projects/site/index.html"), b"x").unwrap();
        wait_for("new folder tree", || find(&index, "index.html"));

        fs::remove_dir_all(root.join("Documents")).unwrap();
        wait_for("deletion", || {
            !find(&index, "old.txt") && !find(&index, "new.txt")
        });
        assert!(
            dir.path().join("index.txt").exists(),
            "snapshot saved after scan"
        );
    }

    /// The UI's indicator: incomplete from the start, scanning while the
    /// disk is read, then done. A restart with a saved index is complete
    /// right away.
    #[test]
    fn reports_progress_until_the_first_scan_ends() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("home");
        fs::create_dir_all(root.join("Documents")).unwrap();
        fs::write(root.join("Documents/a.txt"), b"x").unwrap();
        let snapshot = dir.path().join("index.txt");

        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let index = Arc::new(FileIndex::new(
            std::slice::from_ref(&root),
            &[],
            snapshot.clone(),
        ));
        let log = Arc::clone(&seen);
        index.on_status(move |s| log.lock().unwrap().push(s));
        index.ensure_started();
        wait_for("first scan", || {
            let s = index.status();
            !s.incomplete && !s.scanning && s.entries == 2
        });
        let seen = seen.lock().unwrap().clone();
        assert!(seen.first().is_some_and(|s| s.incomplete), "{seen:?}");
        assert!(seen.iter().any(|s| s.scanning && s.incomplete), "{seen:?}");
        assert!(
            seen.last().is_some_and(|s| !s.incomplete && !s.scanning),
            "{seen:?}"
        );

        // Second run: the saved index makes it complete before rescanning.
        let again = Arc::new(FileIndex::new(std::slice::from_ref(&root), &[], snapshot));
        let first_complete = Arc::new(std::sync::Mutex::new(None));
        let record = Arc::clone(&first_complete);
        again.on_status(move |s| {
            let mut first = record.lock().unwrap();
            if first.is_none() && !s.incomplete {
                *first = Some(s);
            }
        });
        again.ensure_started();
        wait_for("saved index", || first_complete.lock().unwrap().is_some());
        let status = first_complete.lock().unwrap().unwrap();
        assert_eq!(
            status.entries, 2,
            "complete with the saved entries, before any scan"
        );
        assert!(!status.scanning);
    }
}
