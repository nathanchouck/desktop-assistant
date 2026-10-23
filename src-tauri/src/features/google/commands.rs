//! What Settings → Google calls (registered in `ipc/mod.rs` under
//! `// [google]`). The client secret only goes in; replies say whether
//! it's saved.

use serde::Serialize;
use tauri::AppHandle;

use super::account::{GoogleStatus, CLIENT_SECRET};
use super::{connect, shared};
use crate::error::IpcError;
use crate::secrets::{self, Secret};

/// The connection changed: connecting finished or failed, a refresh hit
/// a problem or recovered.
#[derive(Debug, Clone, Serialize, specta::Type, tauri_specta::Event)]
pub struct GoogleStatusChanged {
    pub status: GoogleStatus,
}

fn err(message: impl ToString) -> IpcError {
    IpcError {
        message: message.to_string(),
    }
}

fn google() -> Result<std::sync::Arc<super::hub::Google>, IpcError> {
    shared().ok_or_else(|| err("the app is still starting up"))
}

#[tauri::command]
#[specta::specta]
pub fn google_status() -> Result<GoogleStatus, IpcError> {
    Ok(google()?.status())
}

/// Saves the OAuth client's secret in Credential Manager.
#[tauri::command]
#[specta::specta]
pub async fn google_secret_set(value: String) -> Result<GoogleStatus, IpcError> {
    let google = google()?;
    let value = value.trim();
    if value.is_empty() {
        return Err(err("Paste the client secret first."));
    }
    secrets::set(CLIENT_SECRET, &Secret::new(value)).map_err(err)?;
    tracing::info!("saved the Google client secret");
    Ok(google.status())
}

/// Opens Google's consent page; the outcome follows as `GoogleStatusChanged`.
#[tauri::command]
#[specta::specta]
pub async fn google_connect(app: AppHandle) -> Result<GoogleStatus, IpcError> {
    let google = google()?;
    connect::start(std::sync::Arc::clone(&google), app)
        .await
        .map_err(err)?;
    Ok(google.status())
}

/// Revokes the sign-in and removes the refresh token and the client
/// secret from this PC.
#[tauri::command]
#[specta::specta]
pub async fn google_disconnect() -> Result<GoogleStatus, IpcError> {
    let google = google()?;
    connect::cancel(&google);
    google.account.disconnect().await;
    google.clear();
    tracing::info!("disconnected from Google");
    let status = google.status();
    google.announce();
    Ok(status)
}
