//! What the settings menu calls about where settings are kept (registered
//! in `ipc/mod.rs`). File work runs off the async threads: a synced folder
//! can be slow.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use super::files::{Moved, SettingsFiles};
use super::{folder, live};
use crate::error::IpcError;
use crate::state::AppState;

/// Where settings are kept, for the Advanced section.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct SettingsFolder {
    /// The folder `settings.toml` is read from now.
    pub folder: String,
    pub is_default: bool,
    pub default_folder: String,
    /// This PC's own settings (`settings.local.toml`).
    pub local_file: String,
    /// A chosen folder that isn't there now; the default one is used
    /// until it is.
    pub missing: Option<String>,
    /// `settings.toml` couldn't be read; only this PC's settings are saved
    /// until it can be.
    pub unreadable: Option<String>,
    /// OneDrive's folder for the settings, when OneDrive is set up on this
    /// PC and it isn't the folder in use.
    pub suggestion: Option<FolderCheck>,
}

/// A folder settings could move to.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct FolderCheck {
    /// The folder, written out in full.
    pub folder: String,
    /// It has settings already (saved by another PC).
    pub has_settings: bool,
}

/// What a move did.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct FolderMoved {
    pub location: SettingsFolder,
    /// The settings already there are used now (and the menu got them as
    /// `settings-changed`).
    pub adopted: bool,
    /// Where the file that was replaced was copied.
    pub backup: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn settings_folder_get(app: AppHandle) -> Result<SettingsFolder, IpcError> {
    blocking(move || Ok(location(&app))).await
}

/// Checks a folder typed in the menu before moving there.
#[tauri::command]
#[specta::specta]
pub async fn settings_folder_check(folder: String) -> Result<FolderCheck, IpcError> {
    blocking(move || {
        let folder = folder::parse_folder(&folder, dirs::home_dir().as_deref()).map_err(ipc)?;
        let probe = folder::probe(&folder).map_err(ipc)?;
        Ok(FolderCheck {
            folder: show(&probe.folder),
            has_settings: probe.has_settings,
        })
    })
    .await
}

/// Keeps settings in `folder` (OneDrive) from now on. If it has settings
/// already they're used, unless `replace`: then this PC's are written
/// there and the old file is kept as `settings.toml.bak`.
#[tauri::command]
#[specta::specta]
pub async fn settings_folder_move(
    app: AppHandle,
    folder: String,
    replace: bool,
) -> Result<FolderMoved, IpcError> {
    let target = folder::parse_folder(&folder, dirs::home_dir().as_deref()).map_err(ipc)?;
    move_to(app, target, replace).await
}

/// Back to `%APPDATA%\desktop-assistant`, with the settings in use now.
/// The synced copy is left as it is for the other PCs.
#[tauri::command]
#[specta::specta]
pub async fn settings_folder_reset(app: AppHandle) -> Result<FolderMoved, IpcError> {
    let target = app.state::<AppState>().paths.default_settings_dir().to_owned();
    move_to(app, target, true).await
}

/// Opens this PC's own settings file for hand edits.
#[tauri::command]
#[specta::specta]
pub fn settings_local_open(app: AppHandle) -> Result<(), IpcError> {
    let file = app.state::<AppState>().paths.local_settings_file();
    app.opener()
        .open_path(file.display().to_string(), None::<&str>)
        .map_err(|e| IpcError {
            message: format!("Couldn't open {}: {e}", file.display()),
        })
}

async fn move_to(app: AppHandle, target: PathBuf, replace: bool) -> Result<FolderMoved, IpcError> {
    let handle = app.clone();
    let moved = blocking(move || {
        let current = handle.state::<AppState>().settings();
        handle
            .state::<SettingsFiles>()
            .move_to(&target, &current, replace)
            .map_err(ipc)
    })
    .await?;
    let (adopted, backup) = match moved {
        Moved::Adopted(settings) => {
            live::adopted(&app, settings);
            (true, None)
        }
        Moved::Wrote { backup } => (false, backup.as_deref().map(show)),
    };
    let location = blocking(move || Ok(location(&app))).await?;
    Ok(FolderMoved {
        location,
        adopted,
        backup,
    })
}

fn location(app: &AppHandle) -> SettingsFolder {
    let state = app.state::<AppState>();
    let at = app.state::<SettingsFiles>().location();
    let suggestion = folder::onedrive_suggestion(|name| std::env::var_os(name), Path::is_dir)
        .filter(|s| !folder::same_folder(s, &at.folder))
        .and_then(|s| folder::probe(&s).ok())
        .map(|probe| FolderCheck {
            folder: show(&probe.folder),
            has_settings: probe.has_settings,
        });
    SettingsFolder {
        folder: show(&at.folder),
        is_default: at.is_default,
        default_folder: show(state.paths.default_settings_dir()),
        local_file: show(&state.paths.local_settings_file()),
        missing: at.missing.as_deref().map(show),
        unreadable: at.unreadable,
        suggestion,
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| IpcError {
            message: e.to_string(),
        })?
}

fn show(path: &Path) -> String {
    path.display().to_string()
}

fn ipc(e: impl std::fmt::Display) -> IpcError {
    IpcError {
        message: e.to_string(),
    }
}
