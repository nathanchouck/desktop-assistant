//! Saves the index to disk between runs so file search works the moment the
//! launcher starts. A plain text file: one `D` line per folder followed by
//! one line per entry (`F` file, `d` folder) with its modified time.

use std::fs;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::Path;

use super::{IndexEntry, Tree};

const HEADER: &str = "desktop-assistant file index v1";

pub fn save(path: &Path, roots: &[String], tree: &Tree) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut out = BufWriter::new(fs::File::create(&tmp)?);
        writeln!(out, "{HEADER}")?;
        for root in roots {
            writeln!(out, "R\t{root}")?;
        }
        for (dir, entries) in tree {
            if !safe(dir) {
                continue;
            }
            writeln!(out, "D\t{dir}")?;
            for e in entries.iter().filter(|e| safe(&e.name)) {
                let kind = if e.is_folder { 'd' } else { 'F' };
                match e.modified {
                    Some(m) => writeln!(out, "{kind}\t{m}\t{}", e.name)?,
                    None => writeln!(out, "{kind}\t-\t{}", e.name)?,
                }
            }
        }
        out.flush()?;
    }
    fs::rename(&tmp, path)
}

/// The saved tree, or `None` if there isn't one, it's unreadable, or it was
/// built for different folders.
pub fn load(path: &Path, roots: &[String]) -> Option<Tree> {
    let file = fs::File::open(path).ok()?;
    let mut lines = io::BufReader::new(file).lines().map_while(Result::ok);
    if lines.next()? != HEADER {
        return None;
    }
    let mut tree = Tree::new();
    let mut saved_roots = Vec::new();
    let mut current: Option<String> = None;
    for line in lines {
        let mut parts = line.splitn(3, '\t');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("R"), Some(root), None) => saved_roots.push(root.to_owned()),
            (Some("D"), Some(dir), None) => {
                tree.insert(dir.to_owned(), Vec::new());
                current = Some(dir.to_owned());
            }
            (Some(kind @ ("F" | "d")), Some(modified), Some(name)) => {
                let dir = current.as_ref()?;
                tree.get_mut(dir)?.push(IndexEntry {
                    name: name.into(),
                    is_folder: kind == "d",
                    modified: modified.parse().ok(),
                });
            }
            _ => return None,
        }
    }
    (saved_roots == roots).then_some(tree)
}

/// Names with tabs or newlines would break the line format (Windows doesn't
/// allow them anyway).
fn safe(text: &str) -> bool {
    !text.contains(['\t', '\n', '\r'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let mut tree = Tree::new();
        tree.insert(
            r"C:\Users\n".into(),
            vec![
                IndexEntry {
                    name: "Documents".into(),
                    is_folder: true,
                    modified: Some(10),
                },
                IndexEntry {
                    name: "notes.txt".into(),
                    is_folder: false,
                    modified: None,
                },
            ],
        );
        tree.insert(r"C:\Users\n\Documents".into(), Vec::new());
        tree
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.txt");
        let roots = vec![r"C:\Users\n".to_owned()];
        save(&path, &roots, &sample()).unwrap();
        assert_eq!(load(&path, &roots), Some(sample()));
    }

    #[test]
    fn ignores_a_snapshot_for_other_folders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.txt");
        save(&path, &["A".to_owned()], &sample()).unwrap();
        assert_eq!(load(&path, &["B".to_owned()]), None);
    }

    #[test]
    fn rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.txt");
        fs::write(&path, "not an index\n").unwrap();
        assert_eq!(load(&path, &[]), None);
        fs::write(&path, format!("{HEADER}\nF\t1\torphan.txt\n")).unwrap();
        assert_eq!(load(&path, &[]), None);
    }
}
