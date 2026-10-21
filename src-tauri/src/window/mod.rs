//! The launcher window: created hidden at startup and never destroyed, so
//! showing it is just position + show + focus.

pub mod tray;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewWindow, Window, WindowEvent,
};
use tauri_specta::Event;

use crate::ipc::LauncherShown;
use crate::state::AppState;

pub const LABEL: &str = "launcher";
/// Logical width; the height follows the content (see `set_height`).
pub const WIDTH: f64 = 720.0;
const MIN_HEIGHT: u32 = 48;
const MAX_HEIGHT: u32 = 720;
const DEFAULT_HEIGHT: u32 = 64;
/// The window's top edge sits this far down the screen, so it grows downward.
const TOP_FRACTION: f64 = 0.22;
/// Pressing the hotkey can itself take focus away (X11 keyboard grabs do), so
/// hide-on-blur fires just before the toggle. A toggle this soon after a hide
/// is treated as "close", not "reopen".
const REOPEN_GUARD: Duration = Duration::from_millis(250);
/// Opening the launcher re-reads the app list if it's older than this, to
/// catch Store installs (which don't touch the watched Start Menu folders).
const APPS_MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// Window facts other modules need.
pub struct LauncherState {
    height: AtomicU32,
    glass: AtomicBool,
    last_hidden: Mutex<Option<Instant>>,
}

impl Default for LauncherState {
    fn default() -> Self {
        Self {
            height: AtomicU32::new(DEFAULT_HEIGHT),
            glass: AtomicBool::new(false),
            last_hidden: Mutex::new(None),
        }
    }
}

impl LauncherState {
    pub fn height(&self) -> u32 {
        self.height.load(Ordering::Relaxed)
    }

    /// Whether the OS blur (Acrylic) is active. The UI uses an opaque
    /// background when it isn't.
    pub fn glass(&self) -> bool {
        self.glass.load(Ordering::Relaxed)
    }

    fn mark_hidden(&self) {
        if let Ok(mut t) = self.last_hidden.lock() {
            *t = Some(Instant::now());
        }
    }

    fn hidden_within(&self, window: Duration) -> bool {
        self.last_hidden
            .lock()
            .ok()
            .and_then(|t| *t)
            .is_some_and(|t| t.elapsed() < window)
    }
}

pub fn launcher(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// `started` is when the hotkey fired, for the hotkey-to-visible timing log.
pub fn toggle(app: &AppHandle, started: Instant) {
    let Some(window) = launcher(app) else { return };
    if window.is_visible().unwrap_or(false) {
        hide(app);
        return;
    }
    let just_hidden = app
        .try_state::<AppState>()
        .is_some_and(|s| s.launcher.hidden_within(REOPEN_GUARD));
    if just_hidden {
        tracing::debug!("toggle right after hide: staying hidden");
        return;
    }
    show(app, started);
}

pub fn show(app: &AppHandle, started: Instant) {
    let Some(window) = launcher(app) else {
        tracing::warn!("launcher window missing");
        return;
    };
    let state = app.try_state::<AppState>();
    let height = state
        .as_ref()
        .map_or(DEFAULT_HEIGHT, |s| s.launcher.height());
    if let Some(state) = &state {
        // Before the launcher takes focus, while your window is still in
        // front. Always captured (pasting and window commands need it); the
        // setting decides whether Claude is told about it.
        state.desktop.capture(&state.shell);
        state.apps.refresh_if_stale(APPS_MAX_AGE);
        crate::check_claude_status(app, false);
    }
    if let Err(e) = place_on_cursor_monitor(app, &window, height) {
        tracing::debug!("could not position launcher: {e}");
    }
    if let Err(e) = window.show().and_then(|()| window.set_focus()) {
        tracing::warn!("could not show launcher: {e}");
        return;
    }
    if let Err(e) = LauncherShown.emit(app) {
        tracing::debug!("could not notify UI of show: {e}");
    }
    tracing::debug!(
        "launcher shown in {:.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

pub fn hide(app: &AppHandle) {
    let Some(window) = launcher(app) else { return };
    if !window.is_visible().unwrap_or(true) {
        return;
    }
    if let Err(e) = window.hide() {
        tracing::warn!("could not hide launcher: {e}");
        return;
    }
    if let Some(state) = app.try_state::<AppState>() {
        state.launcher.mark_hidden();
    }
}

/// Resizes to `height` logical pixels (clamped), keeping the top edge fixed.
pub fn set_height(app: &AppHandle, height: u32) {
    tracing::debug!("launcher height requested: {height}");
    let height = height.clamp(MIN_HEIGHT, MAX_HEIGHT);
    if let Some(state) = app.try_state::<AppState>() {
        state.launcher.height.store(height, Ordering::Relaxed);
    }
    if let Some(window) = launcher(app) {
        if let Err(e) = window.set_size(LogicalSize::new(WIDTH, f64::from(height))) {
            tracing::warn!("could not resize launcher: {e}");
        }
    }
}

/// Turns on the Acrylic blur behind the window (Windows 10 1809+ / 11).
/// Returns whether it worked and records it for the UI.
pub fn apply_glass(app: &AppHandle) -> bool {
    let glass = launcher(app).is_some_and(|w| enable_acrylic(&w));
    if let Some(state) = app.try_state::<AppState>() {
        state.launcher.glass.store(glass, Ordering::Relaxed);
    }
    glass
}

#[cfg(windows)]
fn enable_acrylic(window: &WebviewWindow) -> bool {
    match window_vibrancy::apply_acrylic(window, None) {
        Ok(()) => true,
        Err(e) => {
            tracing::info!("glass effect unavailable: {e}");
            false
        }
    }
}

#[cfg(not(windows))]
fn enable_acrylic(_window: &WebviewWindow) -> bool {
    false
}

/// Hides on focus loss and turns close (Alt+F4) into hide, so the window is
/// never destroyed. Set `DA_KEEP_OPEN=1` to stop hide-on-blur while debugging.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != LABEL {
        return;
    }
    match event {
        WindowEvent::Focused(false) if !keep_open() => hide(window.app_handle()),
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide(window.app_handle());
        }
        _ => {}
    }
}

fn keep_open() -> bool {
    static KEEP_OPEN: OnceLock<bool> = OnceLock::new();
    *KEEP_OPEN.get_or_init(|| std::env::var_os("DA_KEEP_OPEN").is_some())
}

fn place_on_cursor_monitor(
    app: &AppHandle,
    window: &WebviewWindow,
    height: u32,
) -> tauri::Result<()> {
    let cursor = app.cursor_position()?;
    let monitor = match app.monitor_from_point(cursor.x, cursor.y)? {
        Some(m) => m,
        None => match window.primary_monitor()? {
            Some(m) => m,
            None => return Ok(()),
        },
    };
    let area = monitor.work_area();
    let (x, y) = placement(
        (area.position.x, area.position.y),
        (area.size.width, area.size.height),
        monitor.scale_factor(),
    );
    window.set_position(PhysicalPosition::new(x, y))?;
    // Re-apply the logical size: moving to a monitor with a different scale
    // factor would otherwise leave the physical size from the old one.
    window.set_size(LogicalSize::new(WIDTH, f64::from(height)))?;
    Ok(())
}

/// Top-left corner in physical pixels: centered horizontally in the work
/// area, `TOP_FRACTION` of the way down.
fn placement(area_pos: (i32, i32), area_size: (u32, u32), scale: f64) -> (i32, i32) {
    let width = (WIDTH * scale).round() as i64;
    let x = i64::from(area_pos.0) + (i64::from(area_size.0) - width) / 2;
    let y = i64::from(area_pos.1) + (f64::from(area_size.1) * TOP_FRACTION).round() as i64;
    (x as i32, y as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopen_guard_tracks_recent_hides() {
        let state = LauncherState::default();
        assert!(!state.hidden_within(REOPEN_GUARD));
        state.mark_hidden();
        assert!(state.hidden_within(REOPEN_GUARD));
        assert!(!state.hidden_within(Duration::ZERO));
    }

    #[test]
    fn centers_on_primary_1080p() {
        assert_eq!(placement((0, 0), (1920, 1040), 1.0), (600, 229));
    }

    #[test]
    fn accounts_for_scale() {
        // 4K at 150%: the window is 1080 physical pixels wide.
        assert_eq!(placement((0, 0), (3840, 2100), 1.5), (1380, 462));
    }

    #[test]
    fn respects_monitor_offset() {
        // A second monitor to the left of the primary.
        assert_eq!(placement((-1920, 0), (1920, 1080), 1.0), (-1320, 238));
    }
}
