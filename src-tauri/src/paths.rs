use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

const APP_DIR: &str = "desktop-assistant";
/// The settings file's name, in whichever folder holds it.
pub const SETTINGS_FILE: &str = "settings.toml";
/// Per-PC settings, in the local data folder.
pub const LOCAL_SETTINGS_FILE: &str = "settings.local.toml";
/// One line: the folder `settings.toml` is in, when it isn't the default.
pub const POINTER_FILE: &str = "settings-folder.txt";

/// Where the app keeps its files. On Windows:
/// config = %APPDATA%\desktop-assistant, data = %LOCALAPPDATA%\desktop-assistant.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> AppResult<Self> {
        let config = dirs::config_dir().ok_or(AppError::MissingDir("config"))?;
        let data = dirs::data_local_dir().ok_or(AppError::MissingDir("local data"))?;
        Ok(Self::with_roots(&config, &data))
    }

    pub fn with_roots(config_root: &Path, data_root: &Path) -> Self {
        Self {
            config_dir: config_root.join(APP_DIR),
            data_dir: data_root.join(APP_DIR),
        }
    }

    /// The settings file in use: `settings.toml` in the folder chosen in
    /// the settings menu (a synced folder such as OneDrive), else in the
    /// config folder. Read fresh each time, since the folder can change
    /// while the app runs.
    pub fn settings_file(&self) -> PathBuf {
        self.settings_dir().folder.join(SETTINGS_FILE)
    }

    /// Where `settings.toml` lives, and whether a chosen folder is missing.
    pub fn settings_dir(&self) -> SettingsDir {
        let chosen = std::fs::read_to_string(self.settings_pointer_file())
            .ok()
            .and_then(|text| parse_pointer(&text));
        resolve_settings_dir(chosen, &self.config_dir, |p| p.is_dir())
    }

    /// Where settings live unless another folder is chosen.
    pub fn default_settings_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Remembers the chosen settings folder. It's in the local data folder,
    /// so it never syncs: each PC picks its own.
    pub fn settings_pointer_file(&self) -> PathBuf {
        self.data_dir.join(POINTER_FILE)
    }

    /// Settings for this PC only, layered over `settings.toml`.
    pub fn local_settings_file(&self) -> PathBuf {
        self.data_dir.join(LOCAL_SETTINGS_FILE)
    }

    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn database_file(&self) -> PathBuf {
        self.data_dir.join("data.db")
    }

    pub fn icon_cache_dir(&self) -> PathBuf {
        self.data_dir.join("icons")
    }

    pub fn file_index_file(&self) -> PathBuf {
        self.data_dir.join("file-index.txt")
    }

    /// PNGs the UI shows as result icons via `thumb/<name>` keys (clipboard
    /// images and the like).
    pub fn thumbnails_dir(&self) -> PathBuf {
        self.data_dir.join("thumbnails")
    }
}

/// The settings folder in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsDir {
    pub folder: PathBuf,
    /// A folder was chosen but isn't there (OneDrive not set up on this PC,
    /// a drive that isn't connected): the default folder is used meanwhile.
    pub missing: Option<PathBuf>,
}

/// The folder named in the pointer file: its first non-empty line, with
/// quotes from "Copy as path" dropped.
pub fn parse_pointer(text: &str) -> Option<PathBuf> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches('"').trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

/// What the pointer file holds for `folder`.
pub fn pointer_text(folder: &Path) -> String {
    format!("{}\n", folder.display())
}

/// The chosen folder when it exists, else the default one. The default
/// folder never counts as missing: it's made when settings are saved.
pub fn resolve_settings_dir(
    chosen: Option<PathBuf>,
    default: &Path,
    is_dir: impl Fn(&Path) -> bool,
) -> SettingsDir {
    match chosen {
        Some(folder) if folder == default || is_dir(&folder) => SettingsDir {
            folder,
            missing: None,
        },
        Some(folder) => SettingsDir {
            folder: default.to_owned(),
            missing: Some(folder),
        },
        None => SettingsDir {
            folder: default.to_owned(),
            missing: None,
        },
    }
}

/// Turns the `index_folders` setting into real folders: `~` becomes the
/// home folder, missing folders are dropped, and folders inside another
/// listed folder are dropped (the outer one already covers them).
pub fn resolve_index_folders(folders: &[String], home: Option<&Path>) -> Vec<PathBuf> {
    let mut resolved: Vec<PathBuf> = folders
        .iter()
        .filter_map(|f| {
            let f = f.trim();
            match f.strip_prefix('~') {
                Some(rest) => home.map(|h| h.join(rest.trim_start_matches(['\\', '/']))),
                None if f.is_empty() => None,
                None => Some(PathBuf::from(f)),
            }
        })
        .filter(|p| p.is_dir())
        .map(|p| p.components().collect::<PathBuf>())
        .collect();
    resolved.sort();
    resolved.dedup();
    let all = resolved.clone();
    resolved.retain(|p| !all.iter().any(|other| other != p && p.starts_with(other)));
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_home_missing_and_nested_folders() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        let folders = [
            "~".to_owned(),
            "~/Documents".to_owned(),
            dir.path().join("data").display().to_string(),
            dir.path().join("missing").display().to_string(),
            " ".to_owned(),
        ];
        let resolved = resolve_index_folders(&folders, Some(&home));
        assert_eq!(resolved, [dir.path().join("data"), home]);
    }

    #[test]
    fn pointer_is_the_first_line_without_quotes() {
        assert_eq!(parse_pointer(""), None);
        assert_eq!(parse_pointer("  \n\n"), None);
        assert_eq!(parse_pointer("\"\""), None);
        assert_eq!(
            parse_pointer("\n  \"C:\\Users\\me\\OneDrive\\desktop-assistant\"  \nignored\n"),
            Some(PathBuf::from(r"C:\Users\me\OneDrive\desktop-assistant"))
        );
        let folder = Path::new("/home/me/OneDrive/desktop-assistant");
        assert_eq!(
            parse_pointer(&pointer_text(folder)).as_deref(),
            Some(folder)
        );
    }

    #[test]
    fn settings_dir_falls_back_to_the_default() {
        let default = Path::new("/cfg/desktop-assistant");
        let synced = PathBuf::from("/onedrive/desktop-assistant");
        let there = |_: &Path| true;
        let gone = |_: &Path| false;
        assert_eq!(
            resolve_settings_dir(None, default, there),
            SettingsDir {
                folder: default.to_owned(),
                missing: None
            }
        );
        assert_eq!(
            resolve_settings_dir(Some(synced.clone()), default, there).folder,
            synced
        );
        let lost = resolve_settings_dir(Some(synced.clone()), default, gone);
        assert_eq!(lost.folder, default);
        assert_eq!(lost.missing, Some(synced));
        let same = resolve_settings_dir(Some(default.to_owned()), default, gone);
        assert_eq!(same.missing, None);
    }

    #[test]
    fn settings_file_follows_the_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_roots(&dir.path().join("cfg"), &dir.path().join("data"));
        assert_eq!(
            paths.settings_file(),
            dir.path().join("cfg").join(APP_DIR).join(SETTINGS_FILE)
        );
        let synced = dir.path().join("OneDrive").join(APP_DIR);
        std::fs::create_dir_all(&synced).unwrap();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        std::fs::write(paths.settings_pointer_file(), pointer_text(&synced)).unwrap();
        assert_eq!(paths.settings_file(), synced.join(SETTINGS_FILE));
        assert_eq!(
            paths.local_settings_file(),
            dir.path()
                .join("data")
                .join(APP_DIR)
                .join(LOCAL_SETTINGS_FILE)
        );
        // OneDrive gone (signed out on this PC): back to the default.
        std::fs::remove_dir(&synced).unwrap();
        let resolved = paths.settings_dir();
        assert_eq!(resolved.folder, paths.config_dir);
        assert_eq!(resolved.missing, Some(synced));
    }

    #[test]
    fn no_home_means_no_tilde_folders() {
        assert!(resolve_index_folders(&["~".to_owned()], None).is_empty());
    }
}
