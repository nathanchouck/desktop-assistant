//! Ways to open the launcher: a configurable global shortcut (Ctrl+Space by
//! default) and, on Windows, tapping the Windows key on its own.

#[cfg(windows)]
mod flyout_win;
#[cfg(windows)]
mod hook_boost;
pub mod hook_timeout;
pub mod shortcut;
pub mod start_guard;
#[cfg(windows)]
mod start_guard_win;
#[cfg(windows)]
mod start_visibility;
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
    win_key: std::sync::Mutex<Option<WinKey>>,
    /// When a Windows key tap last opened (not closed) the launcher, so it
    /// can come back if Start pushed it away.
    #[cfg(windows)]
    tap_opened_at: std::sync::Mutex<Option<Instant>>,
}

/// The Windows key override's two threads. Fields drop in order: the hook
/// first, so nothing posts to the guard once it's gone.
#[cfg(windows)]
struct WinKey {
    _hook: win_key::WinKeyHook,
    _guard: Option<start_guard_win::GuardThread>,
}

/// The Windows key override, for the settings menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct WinKeyStatus {
    pub enabled: bool,
    /// The keyboard hook is installed.
    pub running: bool,
    /// Seconds since the hook last saw a Windows key press.
    pub last_seen_secs: Option<u32>,
    /// Times Start opened anyway after a tap and was closed again, since the
    /// launcher started.
    pub start_closed: u32,
    /// Windows key presses and releases the hook answered late (Windows may
    /// have used the key anyway), since the launcher started.
    pub late_answers: u32,
    /// `LowLevelHooksTimeout` in ms, if set (otherwise Windows' default,
    /// about 300 ms).
    pub hook_timeout_ms: Option<u32>,
    /// The timeout changed since the launcher started, so it applies after
    /// signing out.
    pub hook_timeout_pending: bool,
}

/// Registers the toggle shortcut and, if enabled, the Windows key override.
/// Failures are logged rather than fatal: the tray icon still opens the launcher.
pub fn init(app: &AppHandle, settings: &HotkeySettings) {
    match shortcut::register(app, &settings.toggle) {
        Ok(()) => tracing::info!("shortcut {} registered", settings.toggle),
        Err(e) => crate::toast::report(e.to_string()),
    }
    app.manage(Hotkeys::default());
    hook_timeout::remember_startup_value();
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
    let (running, last_seen, (start_closed, late_answers)) = {
        let (running, last_seen) = win_key::status();
        (running, last_seen, start_guard_win::counts())
    };
    #[cfg(not(windows))]
    let (running, last_seen, (start_closed, late_answers)) = (false, None::<u64>, (0, 0));
    WinKeyStatus {
        enabled,
        running: enabled && running,
        last_seen_secs: last_seen.map(|s| u32::try_from(s).unwrap_or(u32::MAX)),
        start_closed,
        late_answers,
        hook_timeout_ms: hook_timeout::configured_ms(),
        hook_timeout_pending: hook_timeout::change_pending(),
    }
}

/// Gives the Windows key hook a full second to answer (or puts Windows'
/// default back). Applies after signing out.
pub fn set_longer_timeout(longer: bool) -> Result<(), HotkeyError> {
    hook_timeout::set_longer(longer)
}

/// The guard thread first: the hook posts its taps there.
#[cfg(windows)]
fn start_win_key(app: &AppHandle) -> Option<WinKey> {
    let guard = match start_guard_win::start(guard_callbacks(app)) {
        Ok(guard) => Some(guard),
        Err(e) => {
            // Taps still open the launcher, from the hook thread.
            crate::toast::report(format!(
                "Start may open with the launcher after a Windows key tap: couldn't start \
                 the Start guard: {e}"
            ));
            None
        }
    };
    let app = app.clone();
    match win_key::start(move || toggle_from_tap(&app)) {
        Ok(hook) => {
            tracing::info!("windows key override on");
            Some(WinKey {
                _hook: hook,
                _guard: guard,
            })
        }
        Err(e) => {
            crate::toast::report(e.to_string());
            None
        }
    }
}

#[cfg(windows)]
fn guard_callbacks(app: &AppHandle) -> start_guard_win::Callbacks {
    let tap_app = app.clone();
    let closed_app = app.clone();
    start_guard_win::Callbacks {
        on_tap: Box::new(move || {
            toggle_from_tap(&tap_app);
            close_start_enabled(&tap_app)
        }),
        on_closed: Box::new(move || {
            let handle = closed_app.clone();
            let _ = closed_app.run_on_main_thread(move || restore_after_start(&handle));
        }),
    }
}

/// Toggles the launcher for a Windows key tap. Called off the main thread:
/// hands off and returns at once.
#[cfg(windows)]
fn toggle_from_tap(app: &AppHandle) {
    let started = Instant::now();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let was_visible = launcher_visible(&handle);
        window::toggle(&handle, started);
        let opened = !was_visible && launcher_visible(&handle);
        if let Some(hotkeys) = handle.try_state::<Hotkeys>() {
            if let Ok(mut at) = hotkeys.tap_opened_at.lock() {
                *at = opened.then(Instant::now);
            }
        }
    });
}

#[cfg(windows)]
fn close_start_enabled(app: &AppHandle) -> bool {
    app.try_state::<crate::state::AppState>()
        .and_then(|state| state.settings.read().ok().map(|s| s.hotkeys.close_start))
        .unwrap_or(true)
}

/// Start was closed after a tap that opened the launcher: Start took the
/// focus (which hid the launcher, or kept it from getting focus), so bring
/// the launcher back. `show` rather than `toggle`, whose reopen guard would
/// keep it hidden right after that hide.
#[cfg(windows)]
fn restore_after_start(app: &AppHandle) {
    let opened_at = app
        .try_state::<Hotkeys>()
        .and_then(|h| h.tap_opened_at.lock().ok().and_then(|at| *at));
    if !start_guard::should_restore(opened_at, Instant::now()) {
        return;
    }
    let Some(launcher) = window::launcher(app) else {
        return;
    };
    if launcher.is_visible().unwrap_or(false) {
        if let Err(e) = launcher.set_focus() {
            tracing::debug!("could not focus the launcher after closing Start: {e}");
        }
    } else {
        window::show(app, Instant::now());
    }
}

#[cfg(windows)]
fn launcher_visible(app: &AppHandle) -> bool {
    window::launcher(app).is_some_and(|w| w.is_visible().unwrap_or(false))
}

/// Shared by every trigger so timing logs are comparable.
pub(crate) fn toggle_now(app: &AppHandle) {
    window::toggle(app, Instant::now());
}
