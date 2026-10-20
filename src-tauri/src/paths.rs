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
}
