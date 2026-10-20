//! Ways to open the launcher: a configurable global shortcut (Ctrl+Space by
//! default) and, on Windows, tapping the Windows key on its own.

pub mod shortcut;
pub mod tap;
#[cfg(windows)]
mod win_key;

use std::time::Instant;

use tauri::{AppHandle, Manager};

use crate::settings::HotkeySettings;
use crate::window;

#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    #[error("\"{shortcut}\" is not a valid shortcut: {reason}")]
    InvalidShortcut { shortcut: String, reason: String },
    #[error("could not register {shortcut} (another app may be using it): {reason}")]
    Register { shortcut: String, reason: String },
    #[error("could not install the Windows key hook: {0}")]
    Hook(String),
}

/// Keeps the Windows key hook alive for the life of the app.
#[derive(Default)]
pub struct Hotkeys {
    #[cfg(windows)]
    win_key: std::sync::Mutex<Option<win_key::WinKeyHook>>,
}

/// Registers the toggle shortcut and, if enabled, the Windows key override.
/// Failures are logged rather than fatal: the tray icon still opens the launcher.
pub fn init(app: &AppHandle, settings: &HotkeySettings) {
    match shortcut::register(app, &settings.toggle) {
        Ok(()) => tracing::info!("shortcut {} registered", settings.toggle),
        Err(e) => tracing::warn!("{e}"),
    }

    let hotkeys = Hotkeys::default();
    if settings.win_key_override {
        start_win_key(app, &hotkeys);
    }
    app.manage(hotkeys);
}

#[cfg(windows)]
fn start_win_key(app: &AppHandle, hotkeys: &Hotkeys) {
    let app = app.clone();
    let on_tap = move || {
        let started = Instant::now();
        let handle = app.clone();
        // Runs on the hook thread: hand off and return immediately.
        let _ = app.run_on_main_thread(move || window::toggle(&handle, started));
    };
    match win_key::start(on_tap) {
        Ok(hook) => {
            tracing::info!("windows key override on");
            if let Ok(mut slot) = hotkeys.win_key.lock() {
                *slot = Some(hook);
            }
        }
        Err(e) => tracing::warn!("{e}"),
    }
}

#[cfg(not(windows))]
fn start_win_key(_app: &AppHandle, _hotkeys: &Hotkeys) {
    tracing::debug!("windows key override is only available on Windows");
}

/// Shared by every trigger so timing logs are comparable.
pub(crate) fn toggle_now(app: &AppHandle) {
    window::toggle(app, Instant::now());
}
