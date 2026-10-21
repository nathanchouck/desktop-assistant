//! Reading folders from disk into index entries.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use super::rules::Rules;
use super::{IndexEntry, Tree};

/// The direct children of `dir` that the rules let through.
pub fn list_dir(dir: &Path, rules: &Rules) -> io::Result<Vec<IndexEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        // Symlinks and junctions can loop or point outside the roots.
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // On Windows this metadata comes from the directory listing itself,
        // so it costs no extra disk access.
        let metadata = entry.metadata().ok();
        if rules.skip_entry(&name, metadata.as_ref().is_some_and(is_hidden_or_system)) {
            continue;
        }
        let modified = metadata
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_secs()).ok());
        entries.push(IndexEntry {
            name: name.into_boxed_str(),
            is_folder: file_type.is_dir(),
            modified,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Every indexable folder under `root` (inclusive), stopping once `limit`
/// entries have been read. Unreadable folders are skipped.
pub fn walk(root: &Path, rules: &Rules, limit: usize) -> Tree {
    let mut tree = Tree::new();
    walk_in_chunks(root, rules, limit, Duration::MAX, |chunk| {
        tree.extend(chunk)
    });
    tree
}

/// Like `walk`, but hands over what it has read so far every `every` (and
/// the rest at the end), so a long scan can show progress and results.
/// Returns the number of entries read.
pub fn walk_in_chunks(
    root: &Path,
    rules: &Rules,
    limit: usize,
    every: Duration,
    mut emit: impl FnMut(Tree),
) -> usize {
    let mut tree = Tree::new();
    let mut count = 0usize;
    let mut last_emit = Instant::now();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match list_dir(&dir, rules) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::debug!("skipping {}: {e}", dir.display());
                continue;
            }
        };
        for entry in entries.iter().filter(|e| e.is_folder) {
            let child = dir.join(&*entry.name);
            if !rules.excludes_dir(&child.to_string_lossy()) {
                stack.push(child);
            }
        }
        count += entries.len();
        tree.insert(dir.to_string_lossy().into_owned(), entries);
        if count >= limit {
            tracing::warn!(
                "file index stopped at {limit} entries; add exclusions to index the rest"
            );
            break;
        }
        if last_emit.elapsed() >= every {
            emit(std::mem::take(&mut tree));
            last_emit = Instant::now();
        }
    }
    if !tree.is_empty() {
        emit(tree);
    }
    count
}

#[cfg(windows)]
fn is_hidden_or_system(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    metadata.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
fn is_hidden_or_system(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(entries: &[IndexEntry]) -> Vec<&str> {
        entries.iter().map(|e| &*e.name).collect()
    }

    #[test]
    fn walks_and_applies_rules() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("docs/school")).unwrap();
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::create_dir_all(root.join("site/node_modules/react")).unwrap();
        fs::write(root.join("docs/report.pdf"), b"x").unwrap();
        fs::write(root.join("docs/school/hw1.txt"), b"x").unwrap();
        fs::write(root.join("site/node_modules/react/index.js"), b"x").unwrap();
        fs::write(root.join(".env"), b"x").unwrap();

        let rules = Rules::new(&["node_modules".into()]);
        let tree = walk(root, &rules, usize::MAX);
        // Join segment by segment so keys use the platform separator, like
        // the index does (a "a/b" literal would mix separators on Windows).
        let key = |p: &str| {
            p.split('/')
                .filter(|s| !s.is_empty())
                .fold(root.to_path_buf(), |path, segment| path.join(segment))
                .to_string_lossy()
                .into_owned()
        };

        assert_eq!(names(&tree[&key("")]), ["docs", "site"]);
        assert_eq!(names(&tree[&key("docs")]), ["report.pdf", "school"]);
        assert_eq!(names(&tree[&key("docs/school")]), ["hw1.txt"]);
        // The excluded folder is listed but not entered.
        assert_eq!(names(&tree[&key("site")]), ["node_modules"]);
        assert!(!tree.contains_key(&key("site/node_modules")));
        assert!(!tree.contains_key(&key(".git")));
    }

    #[test]
    fn records_kind_and_modified_time() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();
        fs::write(dir.path().join("file.txt"), b"x").unwrap();
        let entries = list_dir(dir.path(), &Rules::default()).unwrap();
        assert!(!entries[0].is_folder && entries[1].is_folder);
        assert!(entries.iter().all(|e| e.modified.is_some()));
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("real/loop")).unwrap();
        let tree = walk(dir.path(), &Rules::default(), usize::MAX);
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn hands_over_everything_in_chunks() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            fs::create_dir_all(dir.path().join(format!("d{i}"))).unwrap();
            fs::write(dir.path().join(format!("d{i}/f.txt")), b"x").unwrap();
        }
        let rules = Rules::default();
        let mut chunks = 0;
        let mut joined = Tree::new();
        let count = walk_in_chunks(dir.path(), &rules, usize::MAX, Duration::ZERO, |chunk| {
            chunks += 1;
            joined.extend(chunk);
        });
        assert_eq!(count, 10);
        assert!(chunks > 1, "a zero interval hands over every folder");
        assert_eq!(joined, walk(dir.path(), &rules, usize::MAX));
    }

    #[test]
    fn stops_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            fs::create_dir_all(dir.path().join(format!("d{i}/inner"))).unwrap();
        }
        let tree = walk(dir.path(), &Rules::default(), 3);
        assert!(tree.len() < 11);
    }
}
