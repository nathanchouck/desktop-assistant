//! Google Calendar, Gmail and Drive without the AI, through your own Google OAuth client (D-008).
//!
//! Not built yet: no results, nothing at startup.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::ProviderContext;
use crate::search::SearchProvider;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct GoogleSettings {
    pub enabled: bool,
}

impl Default for GoogleSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// This feature's search results, if it has any.
pub fn provider(_ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    None
}

/// Background work once the app is running.
pub fn start(_app: &AppHandle) {}
