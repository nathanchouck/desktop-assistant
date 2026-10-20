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
use std::sync::{Arc, RwLock};

use crate::platform::FileHit;

/// Stop reading after this many entries, to keep memory bounded.
const MAX_ENTRIES: usize = 2_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub name: Box<str>,
    pub is_folder: bool,
    /// Last modified, Unix seconds.
    pub modified: Option<i64>,
}

/// Folder path → its direct children.
pub type Tree = BTreeMap<String, Vec<IndexEntry>>;

pub struct FileIndex {
    roots: Vec<String>,
    rules: Rules,
    snapshot_path: PathBuf,
    tree: RwLock<Tree>,
    started: AtomicBool,
    dirty: AtomicBool,
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
        }
    }

    /// Loads, scans and starts watching on a background thread, once. Later
    /// calls do nothing, so it's cheap to call on every search.
    pub fn ensure_started(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let index = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("file-index".into())
            .spawn(move || watch::run(index));
        if let Err(e) = spawned {
            tracing::error!("could not start the file index: {e}");
            self.started.store(false, Ordering::Release);
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

    /// The `max` best entries by `score` (`None` = not a match), best first.
    pub fn search(
        &self,
        mut score: impl FnMut(&IndexEntry) -> Option<f32>,
        max: usize,
    ) -> Vec<FileHit> {
        if max == 0 {
            return Vec::new();
        }
        let Ok(tree) = self.tree.read() else {
            return Vec::new();
        };
        // Min-heap of the best so far: the worst kept hit is on top.
        let mut best: BinaryHeap<Ranked<'_>> = BinaryHeap::with_capacity(max + 1);
        for (dir, entries) in tree.iter() {
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
        let mut ranked = best.into_vec();
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
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
            tracing::warn!("could not save the file index: {e}");
        }
    }
}

struct Ranked<'a> {
    score: f32,
    dir: &'a str,
    entry: &'a IndexEntry,
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
        let names: Vec<String> = index.search(score, 2).into_iter().map(|h| h.name).collect();
        assert_eq!(names, ["aaaa", "aaa"]);
        let hit = &index.search(score, 1)[0];
        assert_eq!(hit.path, Path::new("A").join("aaaa").to_string_lossy());
        assert_eq!(index.len(), 4);
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
            .search(|e| (&*e.name == name).then_some(1.0), 1)
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
}
