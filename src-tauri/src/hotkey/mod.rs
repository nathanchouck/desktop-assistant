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

/// Keeps the Windows key hook alive while it's turned on.
#[derive(Default)]
pub struct Hotkeys {
    #[cfg(windows)]
    win_key: std::sync::Mutex<Option<win_key::WinKeyHook>>,
}

/// The Windows key override, for the settings menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct WinKeyStatus {
    pub enabled: bool,
    /// The keyboard hook is installed.
    pub running: bool,
    /// Seconds since the hook last saw a Windows key press.
    pub last_seen_secs: Option<u32>,
}

/// Registers the toggle shortcut and, if enabled, the Windows key override.
/// Failures are logged rather than fatal: the tray icon still opens the launcher.
pub fn init(app: &AppHandle, settings: &HotkeySettings) {
    match shortcut::register(app, &settings.toggle) {
        Ok(()) => tracing::info!("shortcut {} registered", settings.toggle),
        Err(e) => tracing::warn!("{e}"),
    }
    app.manage(Hotkeys::default());
    set_win_key(app, settings.win_key_override);
}

/// Swaps the global shortcut. If the new one can't be registered, the old
/// one stays and the error says why.
pub fn change_toggle(app: &AppHandle, old: &str, new: &str) -> Result<(), HotkeyError> {
    shortcut::parse(new)?;
    let _ = shortcut::unregister(app, old);
    if let Err(e) = shortcut::register(app, new) {
        let _ = shortcut::register(app, old);
        return Err(e);
    }
    tracing::info!("shortcut changed to {new}");
    Ok(())
}

/// Turns the Windows key override on or off, now.
pub fn set_win_key(app: &AppHandle, enabled: bool) {
    #[cfg(windows)]
    {
        let Some(hotkeys) = app.try_state::<Hotkeys>() else {
            return;
        };
        let Ok(mut slot) = hotkeys.win_key.lock() else {
            return;
        };
        if !enabled {
            if slot.take().is_some() {
                tracing::info!("windows key override off");
            }
        } else if slot.is_none() {
            *slot = start_win_key(app);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        if enabled {
            tracing::debug!("windows key override is only available on Windows");
        }
    }
}

pub fn win_key_status(enabled: bool) -> WinKeyStatus {
    #[cfg(windows)]
    {
        let (running, last_seen) = win_key::status();
        WinKeyStatus {
            enabled,
            running: enabled && running,
            last_seen_secs: last_seen.map(|s| u32::try_from(s).unwrap_or(u32::MAX)),
        }
    }
    #[cfg(not(windows))]
    {
        WinKeyStatus {
            enabled,
            running: false,
            last_seen_secs: None,
        }
    }
}

#[cfg(windows)]
fn start_win_key(app: &AppHandle) -> Option<win_key::WinKeyHook> {
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
            Some(hook)
        }
        Err(e) => {
            tracing::warn!("{e}");
            None
        }
    }
}

/// Shared by every trigger so timing logs are comparable.
pub(crate) fn toggle_now(app: &AppHandle) {
    window::toggle(app, Instant::now());
}
