//! What the UI calls (registered in `ipc/mod.rs`): the settings menu's
//! count and Clear button, and the chat's copy / paste of the last answer.

use tauri::AppHandle;

use super::chats::HistoryOverview;
use crate::actions;
use crate::error::IpcError;

fn ipc_err(message: impl ToString) -> IpcError {
    IpcError {
        message: message.to_string(),
    }
}

/// How many chats are saved. Applies the current limits first, so a
/// lowered cap or age shows at once.
#[tauri::command]
#[specta::specta]
pub async fn history_overview() -> Result<HistoryOverview, IpcError> {
    let history = super::get().ok_or_else(|| ipc_err("Chat history isn't available."))?;
    tokio::task::spawn_blocking(move || {
        history.prune().map_err(ipc_err)?;
        Ok(history.overview())
    })
    .await
    .map_err(ipc_err)?
}

/// Deletes every saved chat except pinned ones.
#[tauri::command]
#[specta::specta]
pub async fn history_clear() -> Result<HistoryOverview, IpcError> {
    let history = super::get().ok_or_else(|| ipc_err("Chat history isn't available."))?;
    tokio::task::spawn_blocking(move || {
        let removed = history.clear().map_err(ipc_err)?;
        tracing::info!("cleared {removed} saved chats");
        Ok(history.overview())
    })
    .await
    .map_err(ipc_err)?
}

/// Copies an answer (its Markdown) from the chat; the launcher stays open.
#[tauri::command]
#[specta::specta]
pub fn chat_copy_answer(app: AppHandle, text: String) -> Result<(), IpcError> {
    actions::copy_text(&app, &text)?;
    crate::toast::info(&app, "Copied the answer");
    Ok(())
}

/// Pastes an answer into the window the launcher was opened from (the
/// launcher hides).
#[tauri::command]
#[specta::specta]
pub async fn chat_paste_answer(app: AppHandle, text: String) -> Result<(), IpcError> {
    actions::paste(&app, Some(&text)).await.map_err(Into::into)
}
