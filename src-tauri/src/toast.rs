//! Short messages in the launcher's footer, for things that happen away
//! from the result you picked (a background task failed, an update is
//! ready). Errors offer to open the log.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_specta::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ToastLevel {
    Info,
    Error,
}

/// Shown in the footer for a few seconds.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, tauri_specta::Event,
)]
pub struct Toast {
    pub level: ToastLevel,
    pub message: String,
}

pub fn info(app: &AppHandle, message: impl Into<String>) {
    send(app, ToastLevel::Info, message.into());
}

/// Also logged, so "Open log" has the details.
pub fn error(app: &AppHandle, message: impl Into<String>) {
    let message = message.into();
    tracing::warn!("{message}");
    send(app, ToastLevel::Error, message);
}

fn send(app: &AppHandle, level: ToastLevel, message: String) {
    if let Err(e) = (Toast { level, message }).emit(app) {
        tracing::debug!("could not show a toast: {e}");
    }
}
