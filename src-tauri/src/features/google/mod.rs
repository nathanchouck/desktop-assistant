//! Google Calendar, Gmail and Drive without the AI, through your own
//! Google OAuth client (D-008): `cal` lists the next events (Enter joins
//! the call), `mail` the unread count and threads, and Drive files join
//! normal search. Read-only scopes; nothing goes to Google until you
//! connect.
//!
//! - `oauth` (pure): PKCE, the consent page, the redirect, token answers.
//! - `loopback`: the one-shot listener on 127.0.0.1; `connect`: the flow.
//! - `account`: the client secret and refresh token (Credential Manager),
//!   access tokens (memory), the status line.
//! - `api`: authorized requests over `crate::net` (WinHTTP), errors.
//! - `calendar`, `meeting`, `mail`, `drive` (pure): requests, parsing, rows.
//! - `sync`: the calendar and inbox cache, refreshed in the background;
//!   `drive_search`: Drive per query, debounced and cached.
//! - `provider`: search and activation; `commands`: what Settings calls.

mod account;
mod api;
mod calendar;
pub mod commands;
mod connect;
mod drive;
mod drive_search;
mod hub;
mod loopback;
mod mail;
mod meeting;
mod oauth;
mod provider;
mod sync;

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_specta::Event;

use super::ProviderContext;
use crate::ipc::LauncherShown;
use crate::search::SearchProvider;
use hub::Google;
use provider::GoogleProvider;

pub use account::GoogleStatus;
pub use provider::keeps_order;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct GoogleSettings {
    /// `cal`, `mail` and Drive results (once connected).
    pub enabled: bool,
    /// Your OAuth client ("Desktop app") from Google Cloud Console. Not a
    /// secret, so it may sync; the client secret stays in Credential Manager.
    pub client_id: String,
    /// Drive files in normal search (what you type goes to Drive).
    pub drive_in_search: bool,
    /// Days `cal` covers, today included (1–7).
    pub calendar_days: u32,
}

impl Default for GoogleSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            client_id: String::new(),
            drive_in_search: true,
            calendar_days: 2,
        }
    }
}

/// Shared by the provider, the background refresh and the commands.
static GOOGLE: OnceLock<Arc<Google>> = OnceLock::new();

fn shared() -> Option<Arc<Google>> {
    GOOGLE.get().cloned()
}

/// This feature's search results.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    let google = GOOGLE
        .get_or_init(|| Arc::new(Google::new(&ctx.paths.data_dir, Arc::clone(&ctx.settings))))
        .clone();
    Some(Arc::new(GoogleProvider::new(google)))
}

/// Keeps the calendar and inbox fresh while connected, and checks again
/// when the launcher opens on data over a minute old.
pub fn start(app: &AppHandle) {
    let Some(google) = shared() else {
        return;
    };
    google.set_app(app);
    tauri::async_runtime::spawn(sync::run(Arc::clone(&google)));
    LauncherShown::listen_any(app, move |_| {
        if google.active() {
            google.cache.refresh_if_stale();
        }
    });
}

/// `text` with a capital first letter: error texts start lower case
/// ("the sign-in expired") and read as a sentence on their own.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences() {
        assert_eq!(sentence("the sign-in expired"), "The sign-in expired");
        assert_eq!(sentence("échec"), "Échec");
        assert_eq!(sentence(""), "");
    }

    #[test]
    fn settings_defaults() {
        let s = GoogleSettings::default();
        assert!(s.enabled && s.drive_in_search);
        assert_eq!(s.client_id, "");
        assert_eq!(s.calendar_days, 2);
    }
}
