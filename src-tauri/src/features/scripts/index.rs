//! Reading the scripts folder: each script's header, and why the files
//! that aren't commands aren't.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::header::{self, Header, HEADER_BYTES};
use super::language::{Interpreters, Language, Runner};

/// More files than this in the folder are left out (it's a folder of
/// commands, not a project).
const MAX_FILES: usize = 1000;

/// A script that's a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    /// Its file name, which is also its id ("hello.ps1").
    pub file: String,
    pub path: PathBuf,
    pub language: Language,
    pub header: Header,
    pub runner: Runner,
}

/// A script file that isn't a command, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub file: String,
    pub message: String,
}

/// What the scripts folder holds, as last read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    pub folder: PathBuf,
    /// Why the folder couldn't be read.
    pub folder_error: Option<String>,
    /// Sorted by file name.
    pub scripts: Vec<Script>,
    pub problems: Vec<Problem>,
}

/// Reads every script directly in `folder` (not in subfolders, which is
/// where helper files can go). Other files are ignored.
pub fn scan(folder: &Path, interpreters: &Interpreters) -> Index {
    let mut index = Index {
        folder: folder.to_path_buf(),
        ..Index::default()
    };
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            index.folder_error = Some("The folder doesn't exist.".into());
            return index;
        }
        Err(e) => {
            index.folder_error = Some(format!("The folder can't be read: {e}"));
            return index;
        }
    };
    let mut files: Vec<(String, PathBuf, Language)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            let path = entry.path();
            let language = Language::from_path(&path)?;
            let file = entry.file_name().to_str()?.to_owned();
            // Ids put a script's argument after a ':' (which Windows
            // never allows in a file name anyway).
            (!file.contains(':')).then_some((file, path, language))
        })
        .take(MAX_FILES)
        .collect();
    files.sort_by_key(|(file, _, _)| file.to_lowercase());

    for (file, path, language) in files {
        match read(&path, language, interpreters) {
            Ok((header, runner)) => index.scripts.push(Script {
                file,
                path,
                language,
                header,
                runner,
            }),
            Err(message) => index.problems.push(Problem { file, message }),
        }
    }
    index
}

fn read(
    path: &Path,
    language: Language,
    interpreters: &Interpreters,
) -> Result<(Header, Runner), String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(HEADER_BYTES as u64).read_to_end(&mut bytes))
        .map_err(|e| format!("Couldn't read it: {e}"))?;
    let header = header::parse(&header::decode(&bytes), language.comment_style())
        .map_err(|e| e.to_string())?;
    let runner = interpreters
        .runner(language)
        .ok_or_else(|| language.missing().to_owned())?;
    Ok((header, runner.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::scripts::header::ScriptMode;

    fn all_found() -> Interpreters {
        let program = |name: &str| {
            Some(Runner::Program {
                program: name.into(),
                args: Vec::new(),
            })
        };
        Interpreters {
            powershell: program("powershell"),
            python: program("py"),
            batch: Some(Runner::Itself),
            node: program("node"),
            bash: None,
        }
    }

    #[test]
    fn finds_commands_and_explains_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
        write("b.py", "# @title Bee\n# @mode copy\n");
        write("a.ps1", "# @title Ay\n");
        write("c.cmd", "@echo off\r\n:: @title Sea\r\n");
        write("d.js", "// @title Dee\n");
        write("helper.py", "import os\n");
        write("bad.ps1", "# @title Bad\n# @mode loud\n");
        write("e.sh", "# @title Ee\n");
        write("README.txt", "# @title Not a script\n");
        std::fs::create_dir(dir.path().join("lib.ps1")).unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("deep.ps1"), "# @title Deep\n").unwrap();

        let index = scan(dir.path(), &all_found());
        let files: Vec<&str> = index.scripts.iter().map(|s| s.file.as_str()).collect();
        assert_eq!(files, ["a.ps1", "b.py", "c.cmd", "d.js"]);
        let bee = &index.scripts[1];
        assert_eq!(bee.header.title, "Bee");
        assert_eq!(bee.header.mode, ScriptMode::Copy);
        assert_eq!(bee.language, Language::Python);
        assert_eq!(bee.path, dir.path().join("b.py"));
        assert_eq!(index.scripts[2].runner, Runner::Itself);

        let problems: Vec<(&str, &str)> = index
            .problems
            .iter()
            .map(|p| (p.file.as_str(), p.message.as_str()))
            .collect();
        assert_eq!(
            problems,
            [
                ("bad.ps1", "@mode “loud” isn't one of silent, output, copy or paste."),
                ("e.sh", "Bash scripts need Git Bash, which wasn't found."),
                ("helper.py", "No @title in the comment at the top, so it isn't a command."),
            ]
        );
        assert_eq!(index.folder_error, None);
    }

    #[test]
    fn a_missing_folder_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let index = scan(&dir.path().join("nope"), &all_found());
        assert!(index.scripts.is_empty());
        assert_eq!(index.folder_error.as_deref(), Some("The folder doesn't exist."));
    }
}
