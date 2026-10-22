//! System commands: lock, sleep, hibernate, restart, shut down, sign out,
//! empty the Recycle Bin, toggle dark mode, mute; and Windows Settings
//! pages (Display, Sound, Bluetooth, ...), opened as `ms-settings:` links.
//!
//! - `commands.rs`: names, keywords, which commands ask first.
//! - `provider.rs`: search rows and running them.
//! - `win.rs` (Windows): the system calls; `fallback.rs` elsewhere.
//! - `matching.rs`, `units.rs`: finding commands by name, sizes and counts
//!   (window layouts, media, color and processes use them too).

mod commands;
#[cfg(not(windows))]
mod fallback;
pub mod matching;
mod provider;
pub mod units;
#[cfg(windows)]
mod win;

#[cfg(not(windows))]
use fallback as os;
#[cfg(windows)]
use win as os;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::ProviderContext;
use crate::search::SearchProvider;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SystemSettings {
    /// System commands and Settings pages in search.
    pub enabled: bool,
    /// Restart, shut down, sign out, hibernate and emptying the Recycle Bin
    /// ask in the footer first; Enter again runs them.
    pub confirm: bool,
}

impl Default for SystemSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            confirm: true,
        }
    }
}

/// System commands in search.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(provider::SystemProvider::new(Arc::clone(
        &ctx.settings,
    ))))
}

/// Nothing to start: the Recycle Bin is looked at when a search needs it.
pub fn start(_app: &AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_ask_first() {
        let s = SystemSettings::default();
        assert!(s.enabled && s.confirm);
    }

    #[test]
    fn older_settings_files_get_the_new_field() {
        // A file from before `confirm` existed still loads (layered over
        // the defaults by `settings::load_or_create`).
        let text = "enabled = false\n";
        let mut merged = toml::Table::try_from(SystemSettings::default()).unwrap();
        merged.extend(text.parse::<toml::Table>().unwrap());
        let s: SystemSettings = merged.try_into().unwrap();
        assert!(!s.enabled && s.confirm);
    }
}
