//! Process killer: `kill chrome` lists running programs matching "chrome"
//! (all their processes, memory and window titles); Enter, asked twice,
//! ends them. `kill` alone lists everything, apps with windows first.
//!
//! - `list.rs`: grouping, what's never offered (Windows' own processes,
//!   the launcher), the order to end them in, what to say after (pure).
//! - `provider.rs`: the `kill` prefix, rows, and ending.
//! - `win.rs` (Windows): the process list and ending; `fallback.rs` elsewhere.

#[cfg(not(windows))]
mod fallback;
mod list;
mod provider;
#[cfg(windows)]
mod win;

use std::sync::{Arc, OnceLock};

use tauri::AppHandle;

use super::ProviderContext;
use crate::search::SearchProvider;

/// For the "Ended 3 chrome.exe processes." toast (providers are made
/// before the app runs).
static APP: OnceLock<AppHandle> = OnceLock::new();

/// The process list in search (`kill`).
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(provider::ProcessProvider::new(Arc::clone(
        &ctx.icons,
    ))))
}

/// Nothing runs in the background: processes are listed when you type
/// `kill`.
pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

fn announce(message: &str) {
    if let Some(app) = APP.get() {
        crate::toast::info(app, message);
    }
}
