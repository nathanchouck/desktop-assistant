use serde::Serialize;

use crate::settings::SettingsError;

/// Top-level error for the app. Module errors convert into this.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("could not find the {0} directory for this user")]
    MissingDir(&'static str),
    #[error("could not open {path}: {reason}")]
    Open { path: String, reason: String },
    #[error("the app is still starting up")]
    NotReady,
    #[error("could not open the app: {0}")]
    Launch(String),
    #[error("unknown result: {0}")]
    UnknownItem(String),
    #[error("could not copy to the clipboard: {0}")]
    Clipboard(String),
}

/// What the UI receives when a command fails. Keeps internal error types
/// out of the generated TypeScript bindings.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct IpcError {
    pub message: String,
}

impl From<AppError> for IpcError {
    fn from(e: AppError) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;
