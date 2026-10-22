//! Choosing the folder `settings.toml` lives in: the default
//! (`%APPDATA%\desktop-assistant`), or a synced one such as OneDrive so
//! your PCs share settings. Each PC remembers its choice in the local data
//! folder (`AppPaths::settings_pointer_file`), which never syncs.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::paths::{pointer_text, AppPaths, SETTINGS_FILE};

/// Environment variables the OneDrive app sets to its folders, the main
/// one first.
const ONEDRIVE_VARS: [&str; 3] = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"];

/// The folder made inside a synced folder for the settings.
pub const SYNCED_SUBFOLDER: &str = "desktop-assistant";

#[derive(Debug, thiserror::Error)]
pub enum FolderError {
    #[error("Type the folder's full path, like C:\\Users\\you\\OneDrive\\desktop-assistant")]
    NotAbsolute,
    #[error("{0} doesn't exist")]
    NoParent(String),
    #[error("{0} is a file, not a folder")]
    NotAFolder(String),
    #[error("{action} {path}: {source}")]
    Io {
        action: &'static str,
        path: String,
        source: io::Error,
    },
}

impl FolderError {
    pub(super) fn io(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.display().to_string(),
            source,
        }
    }
}

/// The OneDrive folder to suggest: `desktop-assistant` inside the first
/// OneDrive the environment names that exists on this PC.
pub fn onedrive_suggestion(
    var: impl Fn(&str) -> Option<OsString>,
    is_dir: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    ONEDRIVE_VARS
        .iter()
        .filter_map(|name| var(name))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .find(|root| is_dir(root))
        .map(|root| root.join(SYNCED_SUBFOLDER))
}

/// A folder typed in the menu: quotes from "Copy as path" dropped, `~` is
/// the user folder, and it must be a full path.
pub fn parse_folder(input: &str, home: Option<&Path>) -> Result<PathBuf, FolderError> {
    let text = input.trim().trim_matches('"').trim();
    let path = match text.strip_prefix('~') {
        Some(rest) => home
            .ok_or(FolderError::NotAbsolute)?
            .join(rest.trim_start_matches(['\\', '/'])),
        None => PathBuf::from(text),
    };
    if text.is_empty() || !path.is_absolute() {
        return Err(FolderError::NotAbsolute);
    }
    Ok(path.components().collect())
}

/// What's at a folder the settings could move to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub folder: PathBuf,
    /// It already has a `settings.toml` (from another PC, or an earlier
    /// move).
    pub has_settings: bool,
}

/// Checks a folder before moving there. A folder that doesn't exist yet is
/// fine if its parent does (it's made on the move).
pub fn probe(folder: &Path) -> Result<Probe, FolderError> {
    if folder.is_dir() {
        return Ok(Probe {
            folder: folder.to_owned(),
            has_settings: folder.join(SETTINGS_FILE).is_file(),
        });
    }
    if folder.exists() {
        return Err(FolderError::NotAFolder(folder.display().to_string()));
    }
    match folder.parent() {
        Some(parent) if parent.is_dir() => Ok(Probe {
            folder: folder.to_owned(),
            has_settings: false,
        }),
        _ => Err(FolderError::NoParent(
            folder.parent().unwrap_or(folder).display().to_string(),
        )),
    }
}

/// How a switch to another folder treats the settings there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// Use the settings already there (saved by another PC).
    Adopt,
    /// Write this PC's settings there (backing up any file it replaces).
    Write,
}

/// A folder with settings in it is adopted unless you asked to replace
/// them; an empty one gets this PC's settings.
pub fn switch_for(probe: &Probe, replace: bool) -> Switch {
    if probe.has_settings && !replace {
        Switch::Adopt
    } else {
        Switch::Write
    }
}

/// Whether two paths name the same folder (Windows paths ignore case).
pub fn same_folder(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Remembers `folder` as this PC's settings folder; the default folder
/// means forgetting the choice.
pub fn remember(paths: &AppPaths, folder: &Path) -> Result<(), FolderError> {
    let pointer = paths.settings_pointer_file();
    if same_folder(folder, paths.default_settings_dir()) {
        return match fs::remove_file(&pointer) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                Err(FolderError::io("could not remove", &pointer, e))
            }
            _ => Ok(()),
        };
    }
    if let Some(dir) = pointer.parent() {
        fs::create_dir_all(dir).map_err(|e| FolderError::io("could not create", dir, e))?;
    }
    fs::write(&pointer, pointer_text(folder))
        .map_err(|e| FolderError::io("could not write", &pointer, e))
}

/// Copies a settings file about to be replaced to `settings.toml.bak` next
/// to it. Returns the copy, if there was a file.
pub fn back_up(file: &Path) -> Result<Option<PathBuf>, FolderError> {
    if !file.is_file() {
        return Ok(None);
    }
    let mut name = file.file_name().unwrap_or_default().to_owned();
    name.push(".bak");
    let backup = file.with_file_name(name);
    fs::copy(file, &backup).map_err(|e| FolderError::io("could not back up", file, e))?;
    Ok(Some(backup))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn suggests_the_first_onedrive_that_exists() {
        let env: HashMap<&str, &str> = [
            ("OneDrive", ""),
            ("OneDriveConsumer", "/home/me/OneDrive - Personal"),
            ("OneDriveCommercial", "/home/me/OneDrive - Work"),
        ]
        .into();
        let var = |name: &str| env.get(name).map(OsString::from);
        let all = |_: &Path| true;
        assert_eq!(
            onedrive_suggestion(var, all),
            Some(PathBuf::from("/home/me/OneDrive - Personal").join(SYNCED_SUBFOLDER))
        );
        let only_work = |p: &Path| p.ends_with("OneDrive - Work");
        assert_eq!(
            onedrive_suggestion(var, only_work),
            Some(PathBuf::from("/home/me/OneDrive - Work").join(SYNCED_SUBFOLDER))
        );
        assert_eq!(onedrive_suggestion(|_| None, all), None);
        assert_eq!(onedrive_suggestion(var, |_| false), None);
    }

    #[test]
    fn typed_folders_are_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        let full = dir.path().join("Sync");
        let typed = format!("  \"{}\"  ", full.display());
        assert_eq!(parse_folder(&typed, None).unwrap(), full);
        let home = dir.path();
        assert_eq!(parse_folder("~/Sync", Some(home)).unwrap(), full);
        assert!(matches!(parse_folder("Sync", Some(home)), Err(FolderError::NotAbsolute)));
        assert!(matches!(parse_folder("  ", Some(home)), Err(FolderError::NotAbsolute)));
        assert!(matches!(parse_folder("~/x", None), Err(FolderError::NotAbsolute)));
    }

    #[test]
    fn probing_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let empty = probe(dir.path()).unwrap();
        assert!(!empty.has_settings);
        assert_eq!(switch_for(&empty, false), Switch::Write);

        fs::write(dir.path().join(SETTINGS_FILE), "version = 1\n").unwrap();
        let taken = probe(dir.path()).unwrap();
        assert!(taken.has_settings);
        assert_eq!(switch_for(&taken, false), Switch::Adopt);
        assert_eq!(switch_for(&taken, true), Switch::Write);

        // Not there yet, but its parent is: made on the move.
        let new = probe(&dir.path().join("new")).unwrap();
        assert!(!new.has_settings);
        assert!(matches!(
            probe(&dir.path().join("a").join("b")),
            Err(FolderError::NoParent(_))
        ));
        assert!(matches!(
            probe(&dir.path().join(SETTINGS_FILE)),
            Err(FolderError::NotAFolder(_))
        ));
    }

    #[test]
    fn remembering_and_forgetting_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_roots(&dir.path().join("cfg"), &dir.path().join("data"));
        let synced = dir.path().join("OneDrive").join(SYNCED_SUBFOLDER);
        fs::create_dir_all(&synced).unwrap();
        remember(&paths, &synced).unwrap();
        assert_eq!(paths.settings_file(), synced.join(SETTINGS_FILE));
        remember(&paths, paths.default_settings_dir()).unwrap();
        assert!(!paths.settings_pointer_file().exists());
        assert_eq!(paths.settings_file(), paths.config_dir.join(SETTINGS_FILE));
        // Forgetting twice is fine.
        remember(&paths, &paths.config_dir.clone()).unwrap();
    }

    #[test]
    fn backups_copy_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SETTINGS_FILE);
        assert_eq!(back_up(&file).unwrap(), None);
        fs::write(&file, "old").unwrap();
        let backup = back_up(&file).unwrap().unwrap();
        assert_eq!(backup, dir.path().join("settings.toml.bak"));
        assert_eq!(fs::read_to_string(backup).unwrap(), "old");
        assert!(file.exists());
    }

    #[test]
    fn same_folder_sees_through_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("x");
        fs::create_dir_all(&a).unwrap();
        assert!(same_folder(&a, &dir.path().join("x").join("..").join("x")));
        assert!(!same_folder(&a, dir.path()));
        assert!(same_folder(Path::new("/not/there"), Path::new("/not/there")));
    }
}
