//! Short messages in the launcher's footer, for things that happen away
//! from the result you picked (a background task failed, an update is
//! ready). Errors offer to open the log.
//!
//! - The same error shows at most once per `REPEAT_WINDOW` (it's still
//!   logged each time), so something failing every few seconds can't
//!   flood the footer.
//! - The last few toasts are kept for `KEEP_FOR`, so the ones sent before
//!   the UI was listening (startup problems) still reach it (`recent`).
//! - Background code without an `AppHandle` (indexes, hooks, settings)
//!   uses `report`. Before `init` runs, `recent` is the only way out.
//!
//! A toast that arrives while the launcher is hidden waits for it to open
//! (the UI's `ToastStore`).

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_specta::Event;

/// An error with the same text isn't shown again for this long.
pub const REPEAT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// How long a toast is kept for a UI that wasn't listening yet. Matches
/// how long a toast waits for the launcher to open (`PENDING_MS` in the UI).
pub const KEEP_FOR: Duration = Duration::from_secs(2 * 60);
/// At most this many are kept; the UI shows only the newest anyway.
const KEEP_AT_MOST: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ToastLevel {
    Info,
    Error,
}

/// Shown in the footer for a few seconds.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, tauri_specta::Event,
)]
pub struct Toast {
    pub level: ToastLevel,
    pub message: String,
}

/// A toast sent a moment ago, for a UI that just started listening.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct RecentToast {
    pub toast: Toast,
    /// How long ago it was sent.
    pub age_ms: u32,
}

/// Where `report` sends toasts; set once the app is built.
static APP: OnceLock<AppHandle> = OnceLock::new();
static BOARD: LazyLock<Mutex<Board>> = LazyLock::new(|| Mutex::new(Board::new()));

/// Lets `report` reach the UI. Call once in setup.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

pub fn info(app: &AppHandle, message: impl Into<String>) {
    send(
        Some(app),
        Toast {
            level: ToastLevel::Info,
            message: message.into(),
        },
    );
}

/// Also logged, so "Open log" has the details.
pub fn error(app: &AppHandle, message: impl Into<String>) {
    send_error(Some(app), message.into());
}

/// An error from background work that you should hear about (an index
/// that can't be built, a hook that won't start), from code that has no
/// `AppHandle`. Logged; shown unless the same text was shown recently.
pub fn report(message: impl Into<String>) {
    send_error(None, message.into());
}

/// Toasts from the last `KEEP_FOR`, oldest first.
pub fn recent() -> Vec<RecentToast> {
    match BOARD.lock() {
        Ok(mut board) => board.recent(Instant::now()),
        Err(_) => Vec::new(),
    }
}

fn send_error(app: Option<&AppHandle>, message: String) {
    tracing::warn!("{message}");
    let allowed = BOARD
        .lock()
        .map(|mut board| board.limiter.allow(&message, Instant::now()))
        .unwrap_or(true);
    if !allowed {
        tracing::debug!("same error shown recently; not showing it again yet");
        return;
    }
    send(
        app,
        Toast {
            level: ToastLevel::Error,
            message,
        },
    );
}

fn send(app: Option<&AppHandle>, mut toast: Toast) {
    toast.message = sentence(&toast.message);
    if let Ok(mut board) = BOARD.lock() {
        board.keep(toast.clone(), Instant::now());
    }
    let Some(app) = app.or_else(|| APP.get()) else {
        return;
    };
    if let Err(e) = toast.emit(app) {
        tracing::debug!("could not show a toast: {e}");
    }
}

/// `text` with a capital first letter: error texts often start lower case
/// ("could not register Ctrl+Space ...") and read as a footer sentence.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Lets each message through once per window.
pub struct RepeatLimiter {
    window: Duration,
    shown: HashMap<String, Instant>,
}

impl RepeatLimiter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            shown: HashMap::new(),
        }
    }

    /// Whether `message` may show at `now`; if so, it's remembered.
    pub fn allow(&mut self, message: &str, now: Instant) -> bool {
        let window = self.window;
        self.shown
            .retain(|_, at| now.saturating_duration_since(*at) < window);
        if self.shown.contains_key(message) {
            return false;
        }
        self.shown.insert(message.to_owned(), now);
        true
    }
}

/// What's been said lately: the repeat limiter and the kept toasts.
struct Board {
    limiter: RepeatLimiter,
    kept: VecDeque<(Instant, Toast)>,
}

impl Board {
    fn new() -> Self {
        Self {
            limiter: RepeatLimiter::new(REPEAT_WINDOW),
            kept: VecDeque::new(),
        }
    }

    fn keep(&mut self, toast: Toast, now: Instant) {
        self.kept.push_back((now, toast));
        while self.kept.len() > KEEP_AT_MOST {
            self.kept.pop_front();
        }
    }

    fn recent(&mut self, now: Instant) -> Vec<RecentToast> {
        self.kept
            .retain(|(at, _)| now.saturating_duration_since(*at) < KEEP_FOR);
        self.kept
            .iter()
            .map(|(at, toast)| RecentToast {
                toast: toast.clone(),
                age_ms: u32::try_from(now.saturating_duration_since(*at).as_millis())
                    .unwrap_or(u32::MAX),
            })
            .collect()
    }
}

/// Toasts sent before the UI listened (see `recent`).
#[tauri::command]
#[specta::specta]
pub fn toast_recent() -> Vec<RecentToast> {
    recent()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast(message: &str) -> Toast {
        Toast {
            level: ToastLevel::Error,
            message: message.into(),
        }
    }

    #[test]
    fn the_same_error_shows_once_per_window() {
        let start = Instant::now();
        let mut limiter = RepeatLimiter::new(Duration::from_secs(60));
        assert!(limiter.allow("index failed", start));
        assert!(!limiter.allow("index failed", start + Duration::from_secs(1)));
        assert!(!limiter.allow("index failed", start + Duration::from_secs(59)));
        // A different message isn't held back.
        assert!(limiter.allow("hook failed", start + Duration::from_secs(2)));
        // Once the window has passed, it may show again.
        assert!(limiter.allow("index failed", start + Duration::from_secs(60)));
        assert!(!limiter.allow("index failed", start + Duration::from_secs(61)));
    }

    #[test]
    fn the_limiter_forgets_old_messages() {
        let start = Instant::now();
        let mut limiter = RepeatLimiter::new(Duration::from_secs(10));
        for i in 0..100 {
            assert!(limiter.allow(&format!("error {i}"), start));
        }
        assert!(limiter.allow("later", start + Duration::from_secs(11)));
        assert_eq!(limiter.shown.len(), 1);
    }

    #[test]
    fn keeps_recent_toasts_for_a_late_listener() {
        let start = Instant::now();
        let mut board = Board::new();
        board.keep(toast("settings broken"), start);
        board.keep(toast("hook failed"), start + Duration::from_secs(30));
        let recent = board.recent(start + Duration::from_secs(60));
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].toast.message, "settings broken");
        assert_eq!(recent[0].age_ms, 60_000);
        assert_eq!(recent[1].age_ms, 30_000);
        // Past KEEP_FOR they're dropped.
        let later = board.recent(start + KEEP_FOR + Duration::from_secs(1));
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].toast.message, "hook failed");
    }

    #[test]
    fn messages_read_as_sentences() {
        assert_eq!(
            sentence("could not register Ctrl+Space"),
            "Could not register Ctrl+Space"
        );
        assert_eq!(sentence("Already fine"), "Already fine");
        assert_eq!(sentence("éclair"), "Éclair");
        assert_eq!(sentence(""), "");
    }

    #[test]
    fn keeps_only_the_last_few() {
        let now = Instant::now();
        let mut board = Board::new();
        for i in 0..(KEEP_AT_MOST + 3) {
            board.keep(toast(&format!("t{i}")), now);
        }
        let recent = board.recent(now);
        assert_eq!(recent.len(), KEEP_AT_MOST);
        assert_eq!(recent[0].toast.message, "t3");
    }
}
