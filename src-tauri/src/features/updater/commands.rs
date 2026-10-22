//! What the settings menu calls (registered in `ipc/mod.rs`).

use tauri::State;

use super::{updates, UpdateStatus};
use crate::error::IpcError;
use crate::state::AppState;

/// Where updating stands. Changes arrive as `update-status-changed`.
#[tauri::command]
#[specta::specta]
pub fn update_status(state: State<'_, AppState>) -> UpdateStatus {
    updates(&state.paths.data_dir).status()
}

/// Checks for an update now; a newer version starts downloading. Returns
/// the status once the check is done.
#[tauri::command]
#[specta::specta]
pub async fn update_check(state: State<'_, AppState>) -> Result<UpdateStatus, IpcError> {
    let updates = updates(&state.paths.data_dir);
    updates.check().await;
    Ok(updates.status())
}

/// Installs the downloaded update and restarts into it. Only returns if
/// that couldn't happen.
#[tauri::command]
#[specta::specta]
pub async fn update_install(state: State<'_, AppState>) -> Result<(), IpcError> {
    updates(&state.paths.data_dir)
        .install()
        .await
        .map_err(|e| IpcError {
            message: format!("Couldn't install the update: {e}"),
        })
}
