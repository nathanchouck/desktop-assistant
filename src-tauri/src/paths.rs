use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

const APP_DIR: &str = "desktop-assistant";

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

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
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
    fn no_home_means_no_tilde_folders() {
        assert!(resolve_index_folders(&["~".to_owned()], None).is_empty());
    }
}
