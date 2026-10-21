//! Auto-update from GitHub Releases, through the Tauri updater plugin
//! driven from Rust.
//!
//! - `setup`: the public key and endpoint this build uses (a build without
//!   a key is "not set up" and never checks);
//! - `schedule`: a minute after start, then every 6 hours;
//! - `manager`: check, download in the background, install on request;
//! - `tracker`: where an update stands and how that's worded;
//! - `saved`: last check and pending install, remembered on this PC;
//! - `provider`: "Check for Updates" / "Install Update and Restart";
//! - `commands`: what the settings menu calls.

pub mod commands;
mod manager;
mod provider;
mod saved;
mod schedule;
mod setup;
mod tracker;

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Wry};

use super::ProviderContext;
use crate::search::SearchProvider;
use crate::state::AppState;
use crate::{settings, toast};
pub use manager::{UpdateStatus, UpdateStatusChanged, Updates};
pub use tracker::UpdatePhase;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct UpdateSettings {
    /// Look for a new version a minute after start and every 6 hours, and
    /// download it in the background. Installing always waits for you.
    pub check_automatically: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_automatically: true,
        }
    }
}

/// Shared by search, the settings menu and the background checks.
static UPDATES: OnceLock<Arc<Updates>> = OnceLock::new();
/// Whether this build can update (Windows, with a public key). Set by
/// `plugin`, which registers the plugin the manager relies on.
static SET_UP: OnceLock<bool> = OnceLock::new();

/// The updater, created on first use with its file in `data_dir`.
fn updates(data_dir: &std::path::Path) -> Arc<Updates> {
    Arc::clone(UPDATES.get_or_init(|| Arc::new(Updates::new(data_dir.join("updates.json")))))
}

/// The Tauri updater plugin for this build. Call before building the app:
/// it completes `plugins.updater` in `config` (the plugin won't start
/// without a valid section) with the public key from the build.
pub fn plugin(
    config: &mut tauri::Config,
) -> tauri::plugin::TauriPlugin<Wry, tauri_plugin_updater::Config> {
    let (section, pubkey) = setup::prepare(
        config.plugins.0.get("updater"),
        option_env!("TAURI_UPDATER_PUBKEY"),
    );
    config.plugins.0.insert("updater".into(), section);
    // Only the Windows build has TLS for the updater and installers to run.
    let _ = SET_UP.set(cfg!(windows) && pubkey.is_some());
    tauri_plugin_updater::Builder::new().build()
}

/// This feature's search results.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(provider::UpdateProvider::new(updates(
        &ctx.paths.data_dir,
    ))))
}

/// Connects the updater to the app and starts the automatic checks.
pub fn start(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let updates = updates(&state.paths.data_dir);
    let set_up = SET_UP.get().copied().unwrap_or(false);
    updates.attach(app, set_up);
    if !set_up {
        tracing::info!("updates aren't set up for this build (no public key)");
        return;
    }
    // A dev build would offer the latest release in place of itself.
    if cfg!(debug_assertions) {
        tracing::debug!("dev build: no automatic update checks");
        return;
    }
    let shared = Arc::clone(&state.settings);
    tauri::async_runtime::spawn(async move {
        let started = schedule::unix_now();
        loop {
            let wait = schedule::wait(schedule::unix_now(), started, updates.last_attempt());
            if !wait.is_zero() {
                tokio::time::sleep(wait.min(schedule::TICK)).await;
                continue;
            }
            let enabled = settings::current(&shared).updates.check_automatically;
            let ran = enabled && matches!(updates.check().await, tracker::CheckResult::Done(_));
            if !ran {
                // Off, or a check or download already under way.
                tokio::time::sleep(schedule::TICK).await;
            }
        }
    });
}

/// Before the launcher restarts: installs a downloaded update instead (its
/// installer opens the new version), so "restart to install" holds for any
/// restart. Returns at once when nothing is downloaded, or if it fails.
pub async fn install_if_ready(app: &AppHandle) {
    let Some(updates) = UPDATES.get() else { return };
    if updates.ready().is_none() {
        return;
    }
    if let Err(e) = updates.install().await {
        toast::error(app, format!("Couldn't install the update: {e}"));
    }
}
