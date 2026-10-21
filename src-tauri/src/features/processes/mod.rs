//! Process killer: `kill chrome` lists matching processes; Enter ends one.
//!
//! Not built yet: no results, nothing at startup.

use std::sync::Arc;

use tauri::AppHandle;

use super::ProviderContext;
use crate::search::SearchProvider;

/// This feature's search results, if it has any.
pub fn provider(_ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    None
}

/// Background work once the app is running.
pub fn start(_app: &AppHandle) {}
