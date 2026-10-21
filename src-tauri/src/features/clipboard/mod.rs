//! Clipboard history: text, images and copied files, searchable with
//! `clip`, pinnable, pasted back into the window you were in with Enter.
//!
//! - `win.rs` (Windows): a listener thread hears every copy, skips private
//!   ones (password managers mark theirs; the ignore list covers the rest)
//!   and hands the copy to a recorder thread.
//! - `recorder.rs`: decodes images, spots repeats, saves image files.
//! - `history.rs`: the list, in its own SQLite file, mirrored in memory.
//! - `provider.rs`: search rows and what Enter, pin and delete do.
//! - `system.rs`: reading and writing the clipboard (snippets use it too).

pub mod content;
pub mod history;
pub mod image;
mod provider;
pub mod recorder;
pub mod system;
#[cfg(windows)]
mod win;

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::ProviderContext;
use crate::paths::AppPaths;
use crate::search::SearchProvider;
use crate::state::AppState;
use history::History;
pub use provider::history_filter;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ClipboardSettings {
    /// Record what you copy.
    pub enabled: bool,
    /// Entries kept, not counting pinned ones.
    pub max_items: u32,
    /// Copies made in these programs are never recorded: program names
    /// ("KeePassXC" for KeePassXC.exe).
    pub ignore_apps: Vec<String>,
    /// Record copied images too (kept as PNG files).
    pub keep_images: bool,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_items: 200,
            ignore_apps: [
                "1Password",
                "Bitwarden",
                "KeePass",
                "KeePassXC",
                "LastPass",
                "Dashlane",
            ]
            .map(String::from)
            .to_vec(),
            keep_images: true,
        }
    }
}

/// The history, shared by the search provider (made while the app state
/// is built) and the listener (started once the app runs). `None` if not
/// even an in-memory database would open.
static HISTORY: OnceLock<Option<Arc<History>>> = OnceLock::new();

fn history(paths: &AppPaths) -> Option<Arc<History>> {
    HISTORY
        .get_or_init(|| {
            let images = paths.data_dir.join("clipboard");
            let thumbs = paths.thumbnails_dir();
            History::open(
                &paths.data_dir.join("clipboard.db"),
                images.clone(),
                thumbs.clone(),
            )
            .or_else(|e| {
                tracing::error!("{e}; clipboard history won't be saved this session");
                History::in_memory(images, thumbs)
            })
            .map_err(|e| tracing::error!("clipboard history is off: {e}"))
            .ok()
            .map(Arc::new)
        })
        .clone()
}

/// Clipboard history in search (`clip`).
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    let history = history(&ctx.paths)?;
    Some(Arc::new(provider::ClipboardProvider::new(
        history,
        Arc::clone(&ctx.settings),
    )))
}

/// Starts listening for copies (Windows).
pub fn start(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some(history) = history(&state.paths) else {
        return;
    };
    // The cap may have been lowered by hand in the settings file.
    let max_items = state.settings().clipboard.max_items;
    if let Err(e) = history.trim(max_items) {
        tracing::warn!("could not trim clipboard history: {e}");
    }
    #[cfg(windows)]
    win::start(history, Arc::clone(&state.settings));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_ignore_password_managers() {
        let s = ClipboardSettings::default();
        assert!(s.enabled && s.keep_images);
        assert_eq!(s.max_items, 200);
        assert!(content::is_ignored("KeePassXC.exe", &s.ignore_apps));
        assert!(content::is_ignored("1Password.exe", &s.ignore_apps));
        assert!(!content::is_ignored("notepad.exe", &s.ignore_apps));
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let s = ClipboardSettings {
            enabled: false,
            max_items: 50,
            ignore_apps: vec!["Secret".into()],
            keep_images: false,
        };
        let text = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<ClipboardSettings>(&text).unwrap(), s);
    }
}
