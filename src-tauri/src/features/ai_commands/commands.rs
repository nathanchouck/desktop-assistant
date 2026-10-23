//! What the UI calls for AI inputs.

use tauri::{AppHandle, State};

use crate::error::IpcError;
use crate::state::AppState;

/// Starts Windows voice typing (Win+H) into the search box, which the UI
/// focuses first. Voice typing writes what you say there, like typing.
#[tauri::command]
#[specta::specta]
pub async fn ai_dictate(state: State<'_, AppState>) -> Result<(), IpcError> {
    if !cfg!(windows) {
        return Err(IpcError {
            message: "Voice typing needs Windows.".into(),
        });
    }
    super::provider::voice_typing(&state.shell)
        .await
        .map_err(|message| IpcError { message })
}

/// Pauses the "ask about the selection" shortcut while the settings menu
/// records a new one, so pressing the current one doesn't grab anything.
#[tauri::command]
#[specta::specta]
pub fn ai_selection_shortcut_pause(app: AppHandle, state: State<'_, AppState>, paused: bool) {
    let text = state.settings().ai_commands.selection_shortcut;
    super::shortcut::pause(&app, &text, paused);
}
