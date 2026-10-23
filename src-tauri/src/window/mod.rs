//! The launcher window: created hidden at startup and never destroyed, so
//! showing it is just position + show + focus.

mod idle;
mod placement;
pub mod tray;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, LogicalSize, Manager, Monitor, PhysicalPosition, WebviewWindow, Window, WindowEvent,
};
use tauri_specta::Event;

use crate::ipc::LauncherShown;
use crate::state::AppState;

pub const LABEL: &str = "launcher";
/// Logical width; the height follows the content (see `set_height`).
pub const WIDTH: f64 = 720.0;
/// Logical width while Claude and ChatGPT answer side by side (see
/// `set_wide`): two columns about as wide as most answers' lines. Never
/// wider than the screen's work area (see `placement`).
const WIDE_WIDTH: f64 = 1040.0;
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
/// After the window's scale changes, it's fitted again this much later,
/// once Windows and tao are done moving and resizing it.
const REFIT_DELAY: Duration = Duration::from_millis(50);

/// Whether the launcher is on screen, for background work that only
/// matters while it is (see `is_shown`).
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Whether the launcher is on screen. Background work that only feeds
/// what it shows (the media watcher) rests while it's hidden.
pub fn is_shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// Window facts other modules need.
pub struct LauncherState {
    height: AtomicU32,
    /// Two answers side by side: the window is `WIDE_WIDTH` wide.
    wide: AtomicBool,
    glass: AtomicBool,
    last_hidden: Mutex<Option<Instant>>,
    /// Hidden long enough to give memory back (see `idle`).
    idle: idle::Idle,
}

impl Default for LauncherState {
    fn default() -> Self {
        Self {
            height: AtomicU32::new(DEFAULT_HEIGHT),
            wide: AtomicBool::new(false),
            glass: AtomicBool::new(false),
            last_hidden: Mutex::new(None),
            idle: idle::Idle::default(),
        }
    }
}

impl LauncherState {
    pub fn height(&self) -> u32 {
        self.height.load(Ordering::Relaxed)
    }

    /// Logical width: wider while two answers are side by side.
    pub fn width(&self) -> f64 {
        if self.wide.load(Ordering::Relaxed) {
            WIDE_WIDTH
        } else {
            WIDTH
        }
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
    let size = state
        .as_ref()
        .map_or((WIDTH, DEFAULT_HEIGHT), |s| launcher_size(s));
    if let Some(state) = &state {
        // Before the launcher takes focus, while your window is still in
        // front. Always captured (pasting and window commands need it); the
        // setting decides whether Claude is told about it.
        state.desktop.capture(&state.shell);
        state.apps.refresh_if_stale(APPS_MAX_AGE);
        crate::check_ai_status(app, false);
    }
    if let Some(state) = &state {
        idle::on_show(&window, &state.launcher.idle);
    }
    if let Err(e) = place_on_cursor_monitor(app, &window, size) {
        tracing::debug!("could not position launcher: {e}");
    }
    if let Err(e) = window.show().and_then(|()| window.set_focus()) {
        tracing::warn!("could not show launcher: {e}");
        return;
    }
    SHOWN.store(true, Ordering::Relaxed);
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
    SHOWN.store(false, Ordering::Relaxed);
    if let Some(state) = app.try_state::<AppState>() {
        state.launcher.mark_hidden();
        idle::on_hide(app, &state.launcher.idle);
    }
    if let Err(e) = LauncherHidden.emit(app) {
        tracing::debug!("could not notify UI of hide: {e}");
    }
}

/// Sent each time the launcher hides, so the UI can hold toasts for the
/// next open and stop anything that only matters while it's on screen.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct LauncherHidden;

/// Resizes to `height` logical pixels (clamped), keeping the top edge
/// fixed unless the bottom would leave the screen (see `placement`).
pub fn set_height(app: &AppHandle, height: u32) {
    tracing::debug!("launcher height requested: {height}");
    let height = height.clamp(MIN_HEIGHT, MAX_HEIGHT);
    let mut width = WIDTH;
    if let Some(state) = app.try_state::<AppState>() {
        state.launcher.height.store(height, Ordering::Relaxed);
        width = state.launcher.width();
    }
    if let Some(window) = launcher(app) {
        if let Err(e) = fit_on_current_monitor(&window, (width, height)) {
            tracing::warn!("could not resize launcher: {e}");
        }
    }
}

/// Widens the window while two answers are side by side (`wide`), or
/// brings it back to its usual width. Like the height, it changes once
/// when that view opens or closes, never while typing; it stays centered
/// on its monitor.
pub fn set_wide(app: &AppHandle, wide: bool) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if state.launcher.wide.swap(wide, Ordering::Relaxed) == wide {
        return;
    }
    let Some(window) = launcher(app) else { return };
    // A hidden window is placed at its new size when it opens.
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    if let Err(e) = fit_on_current_monitor(&window, launcher_size(&state)) {
        tracing::warn!("could not resize launcher: {e}");
    }
}

/// The window's logical (width, height) as the UI last asked for it.
fn launcher_size(state: &AppState) -> (f64, u32) {
    (state.launcher.width(), state.launcher.height())
}

/// Fits the window to its own monitor again at logical `(width, height)`:
/// centered, at its usual top edge or higher if needed. Moves only when
/// the top edge has to change, and in the order that never pokes past the
/// work area.
fn fit_on_current_monitor(window: &WebviewWindow, (width, height): (f64, u32)) -> tauri::Result<()> {
    let size = LogicalSize::new(width, f64::from(height));
    let Some(monitor) = window.current_monitor()? else {
        return window.set_size(size);
    };
    let frame = frame_on(&monitor, (width, height));
    let target = PhysicalPosition::new(frame.x, frame.y);
    let now = window.outer_position()?;
    let size = logical_size(&frame, monitor.scale_factor());
    if now == target {
        window.set_size(size)
    } else if target.y < now.y {
        // Growing near the bottom: up first, then taller.
        window.set_position(target)?;
        window.set_size(size)
    } else {
        window.set_size(size)?;
        window.set_position(target)
    }
}

/// The window's frame on `monitor` at logical `(width, height)`.
fn frame_on(monitor: &Monitor, (width, height): (f64, u32)) -> placement::Rect {
    let area = monitor.work_area();
    placement::frame(
        placement::Rect {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        monitor.scale_factor(),
        (width, f64::from(height)),
        TOP_FRACTION,
    )
}

/// `frame`'s size in logical pixels. Sizes are set logically on purpose:
/// tao converts with the scale it knows for the window, which is the new
/// one if Windows already said the scale changed, and otherwise gets
/// rescaled when it does. A physical size would be scaled twice then.
fn logical_size(frame: &placement::Rect, scale: f64) -> LogicalSize<f64> {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    LogicalSize::new(
        f64::from(frame.width) / scale,
        f64::from(frame.height) / scale,
    )
}

/// After a scale change (the window moved to a monitor with another
/// scale, or the display settings changed): fit it again once things
/// settle, if it's showing. A hidden one is placed afresh when it opens.
fn refit_soon(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(REFIT_DELAY).await;
        let Some(window) = launcher(&app) else { return };
        if !window.is_visible().unwrap_or(false) {
            return;
        }
        let size = app
            .try_state::<AppState>()
            .map_or((WIDTH, DEFAULT_HEIGHT), |s| launcher_size(&s));
        if let Err(e) = fit_on_current_monitor(&window, size) {
            tracing::debug!("could not fit the launcher after a scale change: {e}");
        }
    });
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
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            tracing::info!("launcher scale is now {:.0}%", scale_factor * 100.0);
            refit_soon(window.app_handle());
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
    size: (f64, u32),
) -> tauri::Result<()> {
    let cursor = app.cursor_position()?;
    let monitor = match app.monitor_from_point(cursor.x, cursor.y)? {
        Some(m) => m,
        None => match window.primary_monitor()? {
            Some(m) => m,
            None => return Ok(()),
        },
    };
    let frame = frame_on(&monitor, size);
    let target = PhysicalPosition::new(frame.x, frame.y);
    // The window is hidden here, so the extra moves don't show. Moving to a
    // monitor with another scale makes Windows rescale (and may shift) the
    // window during the first move; the size is then set for the new
    // monitor and the position set again. If Windows only tells the window
    // later (when it shows), `refit_soon` fixes it up.
    window.set_position(target)?;
    window.set_size(logical_size(&frame, monitor.scale_factor()))?;
    window.set_position(target)?;
    Ok(())
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
    fn comparing_widens_the_window() {
        let state = LauncherState::default();
        assert_eq!(state.width(), WIDTH);
        state.wide.store(true, Ordering::Relaxed);
        assert_eq!(state.width(), WIDE_WIDTH);
        assert!(WIDE_WIDTH > WIDTH);
    }

    #[test]
    fn logical_size_undoes_the_scale() {
        let frame = placement::Rect {
            x: 0,
            y: 0,
            width: 1080,
            height: 96,
        };
        assert_eq!(logical_size(&frame, 1.5), LogicalSize::new(720.0, 64.0));
        assert_eq!(logical_size(&frame, 0.0), LogicalSize::new(1080.0, 96.0));
    }
}
