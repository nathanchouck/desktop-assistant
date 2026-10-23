//! Snippets: saved text you paste by name or keyword from the launcher,
//! and (when turned on) expanded as you type anywhere: type `;sig` and it
//! becomes your signature.
//!
//! - `provider.rs`: search rows; Enter pastes the snippet.
//! - `expand.rs`: placeholders (`{clipboard}`, `{date}`, `{time}`, `{datetime}`).
//! - `typing.rs`: spotting a keyword in what's typed (pure, tested).
//! - `hook.rs` (Windows): the keyboard hook and the expansion itself.

pub mod expand;
#[cfg(windows)]
mod hook;
mod provider;
pub mod typing;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::ProviderContext;
use crate::search::SearchProvider;
use typing::Expansion;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SnippetSettings {
    /// Snippets show up in search.
    pub enabled: bool,
    pub items: Vec<Snippet>,
    /// Typing a keyword in any app replaces it with the snippet.
    pub expand_as_you_type: bool,
    /// Only keywords starting with this expand as you type, so ordinary
    /// words never do.
    pub trigger: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Snippet {
    pub name: String,
    /// Typed to find or expand it, e.g. ";sig". May be empty.
    pub keyword: String,
    /// The text, with optional placeholders.
    pub text: String,
}

impl Default for SnippetSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            items: vec![Snippet {
                name: "Today's date".into(),
                keyword: ";date".into(),
                text: "{date}".into(),
            }],
            expand_as_you_type: false,
            trigger: ";".into(),
        }
    }
}

/// The keywords that should expand as you type right now (none when it's off).
pub fn wanted_expansions(settings: &SnippetSettings) -> Vec<Expansion> {
    if settings.enabled && settings.expand_as_you_type {
        typing::expansions(settings)
    } else {
        Vec::new()
    }
}

/// Snippets in search.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(provider::SnippetProvider::new(Arc::clone(
        &ctx.settings,
    ))))
}

/// Starts expansion as you type (Windows), following the settings.
pub fn start(app: &AppHandle) {
    #[cfg(windows)]
    {
        use tauri::Manager;
        if let Some(state) = app.try_state::<crate::state::AppState>() {
            let settings = Arc::clone(&state.settings);
            tauri::async_runtime::spawn(follow_settings(settings));
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// Turns the keyboard hook on and off with the settings, and hands it new
/// keywords. Wakes only when settings change; the hook only runs while
/// needed.
#[cfg(windows)]
async fn follow_settings(settings: crate::settings::SharedSettings) {
    // Subscribed before the first look, so no change is missed.
    let mut changes = crate::settings::changes();
    let mut running: Option<hook::Expander> = None;
    let mut current: Vec<Expansion> = Vec::new();
    loop {
        let wanted = wanted_expansions(&crate::settings::current(&settings).snippets);
        if wanted != current {
            running = hook::apply(running.take(), &wanted);
            current = wanted;
        }
        if changes.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = SnippetSettings::default();
        assert!(s.enabled);
        assert!(!s.expand_as_you_type, "typing expansion is opt-in");
        assert_eq!(s.trigger, ";");
        assert_eq!(s.items[0].keyword, ";date");
        assert!(wanted_expansions(&s).is_empty());
    }

    #[test]
    fn expansion_follows_the_switches() {
        let mut s = SnippetSettings {
            expand_as_you_type: true,
            ..SnippetSettings::default()
        };
        assert_eq!(wanted_expansions(&s).len(), 1);
        s.enabled = false;
        assert!(wanted_expansions(&s).is_empty());
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let s = SnippetSettings {
            enabled: true,
            items: vec![Snippet {
                name: "Sig".into(),
                keyword: ";sig".into(),
                text: "Best,\n\"Me\"".into(),
            }],
            expand_as_you_type: true,
            trigger: "//".into(),
        };
        let text = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<SnippetSettings>(&text).unwrap(), s);
    }
}
