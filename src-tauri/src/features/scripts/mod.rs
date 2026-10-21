//! Script commands: a script in the scripts folder whose first lines are
//! a header comment (`# @title Say Hello`) becomes a launcher command.
//!
//! - `header.rs`: reading the header (pure, tested).
//! - `language.rs`: .ps1/.py/.cmd/.bat/.js/.sh and the programs that run them.
//! - `index.rs`: reading the folder; `watch.rs` keeps it current.
//! - `library.rs`: the shared state (index, running scripts, the app).
//! - `rows.rs`: the rows a query finds (pure, tested); `provider.rs`:
//!   the search provider, which runs a script when it is picked.
//! - `run.rs`: running one (no window, capped output, time limit);
//!   `output.rs`: what it printed, as messages (pure, tested).
//! - `starter.rs`: the README and example a new folder starts with.
//! - `win.rs` (Windows): the console code page, stopping a process tree.

pub mod header;
mod index;
pub mod language;
pub mod library;
mod output;
mod provider;
mod rows;
mod run;
mod starter;
mod watch;
#[cfg(windows)]
mod win;

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::ProviderContext;
use crate::platform::Shell;
use crate::search::SearchProvider;
use library::{Library, ScriptsOverview};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ScriptSettings {
    /// Scripts in the folder show up in search.
    pub enabled: bool,
    /// Where scripts are read from. Empty: `scripts` in the settings
    /// folder (`%APPDATA%\desktop-assistant\scripts`). `~` is your user
    /// folder; `%NAME%` an environment variable.
    pub folder: String,
    /// Seconds a script may run before it's stopped.
    pub timeout_secs: u32,
}

impl Default for ScriptSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            folder: String::new(),
            timeout_secs: 60,
        }
    }
}

/// Shared by the provider (made while the app state is built), the folder
/// watcher (started once the app runs) and the settings menu.
static LIBRARY: OnceLock<Arc<Library>> = OnceLock::new();

/// Script commands in search.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    let library = LIBRARY
        .get_or_init(|| Arc::new(Library::new(ctx.paths.clone(), Arc::clone(&ctx.settings))))
        .clone();
    Some(Arc::new(provider::ScriptProvider::new(
        library,
        Arc::clone(&ctx.shell),
        Arc::clone(&ctx.desktop),
        Arc::clone(&ctx.store),
    )))
}

/// Reads the scripts folder (making it the first time) and keeps
/// following it.
pub fn start(app: &AppHandle) {
    let Some(library) = LIBRARY.get().cloned() else {
        return;
    };
    library.set_app(app.clone());
    tauri::async_runtime::spawn(watch::follow(library));
}

/// The folder and its scripts for the settings menu, read again first
/// when `reload` is set.
pub async fn overview(reload: bool) -> Result<ScriptsOverview, String> {
    let library = LIBRARY
        .get()
        .cloned()
        .ok_or("Script commands aren't ready yet.")?;
    if reload {
        library.rescan().await;
    }
    Ok(library.overview())
}

/// Opens the scripts folder in File Explorer (making it if needed), for
/// the settings menu's button.
pub async fn open_folder(shell: &Shell) -> Result<(), String> {
    let library = LIBRARY
        .get()
        .cloned()
        .ok_or("Script commands aren't ready yet.")?;
    provider::open_folder(&library, shell).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = ScriptSettings::default();
        assert!(s.enabled);
        assert_eq!(s.folder, "", "empty means the default folder");
        assert_eq!(s.timeout_secs, 60);
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let s = ScriptSettings {
            enabled: false,
            folder: r"D:\Scripts".into(),
            timeout_secs: 300,
        };
        let text = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<ScriptSettings>(&text).unwrap(), s);
    }
}
