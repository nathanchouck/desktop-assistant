//! Windows only: the thread behind `start_guard.rs`, and the hook's link to
//! the rest of the app.
//!
//! The keyboard hook posts each handled tap here (one `PostThreadMessageW`,
//! no allocation, no Tauri code inside the hook callback, which Windows is
//! timing). This thread then toggles the launcher and, for about a second,
//! watches for Start or Windows Search coming to the front:
//! - `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`, delivered to this thread
//!   while it waits for messages (no DLL injection);
//! - `IAppVisibility`'s "launcher visibility" event, documented and used by
//!   other shell tools for "Start opened" (Start only, not Search);
//! - a 25 ms poll while watching, in case an event is missed or arrives
//!   before Start is actually in front.
//!
//! Each one just re-reads the foreground window and asks `StartGuard`. To
//! close Start it sends one Escape (tagged so our hook ignores it), checked
//! again right before sending that Start/Search is still in front and no
//! modifier is held. Every Windows key event's answer time is also posted
//! here and logged, so slow answers can be seen in the log.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Instant;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PeekMessageW, SetTimer, TranslateMessage,
    EVENT_SYSTEM_FOREGROUND, MSG, PM_NOREMOVE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_APP, WM_QUIT, WM_TIMER,
};
// The raw form for the hook thread: returns a plain BOOL, nothing to build.
use windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use super::flyout_win::{describe_front, front_window, modifiers_down, press_escape, windows_dir};
use super::hook_timeout::LATE_MS;
use super::start_guard::{Ending, Report, StartGuard, Step, Tap};
use super::start_visibility as visibility;

/// Hook → guard: a tap was handled. wParam: ms the hook took to answer the
/// release; lParam: 1 if Windows had seen the press.
const WM_TAP: u32 = WM_APP + 0x40;
/// Hook → guard: Escape went down; stop watching.
const WM_ESCAPE: u32 = WM_APP + 0x41;
/// Hook → guard: a Windows key event was answered (a press also ends any
/// watch). wParam: ms after the key; lParam: bit 0 release, bit 1
/// right-hand key.
const WM_ANSWERED: u32 = WM_APP + 0x42;
/// IAppVisibility → guard. wParam: 1 if Start is now visible.
const WM_START_VISIBLE: u32 = WM_APP + 0x43;

const POLL_MS: u32 = 25;

/// The guard thread's id, for the hook's notify functions (0: not running).
static GUARD_THREAD: AtomicU32 = AtomicU32::new(0);
/// For the settings menu, since the app started.
static START_CLOSED: AtomicU32 = AtomicU32::new(0);
static LATE_ANSWERS: AtomicU32 = AtomicU32::new(0);

/// How often Start was closed after a tap, and how many Windows key
/// presses and releases the hook answered late, since the app started.
pub fn counts() -> (u32, u32) {
    (
        START_CLOSED.load(Ordering::Relaxed),
        LATE_ANSWERS.load(Ordering::Relaxed),
    )
}

/// From the hook: a lone tap was handled. Returns false if the guard isn't
/// running, so the caller opens the launcher itself.
pub fn notify_tap(answered_ms: u32, windows_saw_press: bool) -> bool {
    post(WM_TAP, answered_ms as usize, isize::from(windows_saw_press))
}

/// From the hook: Escape went down (Ctrl+Esc opens Start on purpose).
pub fn notify_escape() {
    post(WM_ESCAPE, 0, 0);
}

/// From the hook: a Windows key event, and how long after the key it was
/// answered. A press means whatever opens next is the user's doing (Win+S,
/// a second tap).
pub fn notify_answered(release: bool, right: bool, answered_ms: u32) {
    let flags = isize::from(release) | (isize::from(right) << 1);
    post(WM_ANSWERED, answered_ms as usize, flags);
}

fn post(message: u32, wparam: usize, lparam: isize) -> bool {
    let thread = GUARD_THREAD.load(Ordering::Relaxed);
    // SAFETY: posting to a thread id has no preconditions; it fails
    // harmlessly if the thread is gone.
    thread != 0 && unsafe { PostThreadMessageW(thread, message, wparam, lparam) } != 0
}

/// What the guard does in the rest of the app.
pub struct Callbacks {
    /// Toggles the launcher for a tap (hand off; don't block). Returns
    /// whether Start should be closed if it opens (the setting).
    pub on_tap: Box<dyn Fn() -> bool + Send>,
    /// Start was closed after a tap: put the launcher back in front.
    pub on_closed: Box<dyn Fn() + Send>,
}

struct Ctx {
    callbacks: Callbacks,
    guard: StartGuard,
    timer: usize,
    windows_dir: String,
    tapped_at: Option<Instant>,
    /// When IAppVisibility said Start opened and closed, during this watch.
    start_shown_at: Option<Instant>,
    start_hidden_at: Option<Instant>,
    /// Which Windows keys are down, to log presses but not auto-repeats.
    keys_down: [bool; 2],
    /// How late the hook answered the last Windows key press, for the report.
    last_press_ms: Option<u32>,
    _visibility: Option<visibility::Watch>,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// Owns the guard thread. Dropping it stops the thread.
pub struct GuardThread {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Drop for GuardThread {
    fn drop(&mut self) {
        GUARD_THREAD.store(0, Ordering::Relaxed);
        // SAFETY: see `post`.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start(callbacks: Callbacks) -> std::io::Result<GuardThread> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("start-guard".into())
        .spawn(move || run(callbacks, ready_tx))?;
    match ready_rx.recv() {
        Ok(thread_id) => Ok(GuardThread {
            thread_id,
            thread: Some(thread),
        }),
        Err(_) => Err(std::io::Error::other(
            "the Start guard thread exited during startup",
        )),
    }
}

fn run(callbacks: Callbacks, ready: mpsc::Sender<u32>) {
    if let Err(e) = super::hook_boost::opt_out_of_ecoqos() {
        tracing::debug!("start guard: could not opt out of EcoQoS: {e}");
    }
    let mut msg = MSG::default();
    // SAFETY: plain Win32 thread setup; everything created here is released
    // on this thread before it ends.
    let (com, thread_id, foreground_hook) = unsafe {
        // It hands taps to the launcher: keep that quick on a busy PC.
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        // Create the message queue before anyone posts to it.
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let com = visibility::Apartment::enter();
        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        (com, GetCurrentThreadId(), hook)
    };
    if foreground_hook.is_invalid() {
        tracing::debug!("start guard: no foreground events; polling only");
    }
    let visibility =
        com.as_ref().and_then(
            |_| match visibility::Watch::start(thread_id, WM_START_VISIBLE) {
                Ok(watch) => Some(watch),
                Err(e) => {
                    tracing::debug!("start guard: no Start visibility events: {e}");
                    None
                }
            },
        );
    CTX.with(|c| {
        *c.borrow_mut() = Some(Ctx {
            callbacks,
            guard: StartGuard::default(),
            timer: 0,
            windows_dir: windows_dir(),
            tapped_at: None,
            start_shown_at: None,
            start_hidden_at: None,
            keys_down: [false; 2],
            last_press_ms: None,
            _visibility: visibility,
        })
    });
    GUARD_THREAD.store(thread_id, Ordering::Relaxed);
    let _ = ready.send(thread_id);

    // SAFETY: the standard message loop; `msg` is ours.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        let (wparam, lparam) = (msg.wParam.0, msg.lParam.0);
        match msg.message {
            WM_TAP => with_ctx(|c| c.tapped(wparam as u32, lparam != 0)),
            WM_ESCAPE => with_ctx(Ctx::escape_down),
            WM_ANSWERED => with_ctx(|c| c.answered(wparam as u32, lparam)),
            WM_START_VISIBLE => with_ctx(|c| c.start_visibility(wparam != 0)),
            WM_TIMER if msg.hwnd.is_invalid() => with_ctx(Ctx::check),
            _ => {
                // COM's hidden window (STA) needs its messages dispatched.
                // SAFETY: as above.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }

    GUARD_THREAD.store(0, Ordering::Relaxed);
    with_ctx(Ctx::stop_poll);
    if !foreground_hook.is_invalid() {
        // SAFETY: the hook came from SetWinEventHook on this thread.
        let _ = unsafe { UnhookWinEvent(foreground_hook) };
    }
    // Drops the IAppVisibility watch before COM goes away.
    CTX.with(|c| c.borrow_mut().take());
    drop(com);
    tracing::debug!("start guard stopped");
}

fn with_ctx(f: impl FnOnce(&mut Ctx)) {
    CTX.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut() {
            if let Some(c) = c.as_mut() {
                f(c);
            }
        }
    });
}

impl Ctx {
    fn tapped(&mut self, release_answered_ms: u32, windows_saw_press: bool) {
        let close = (self.callbacks.on_tap)();
        let now = Instant::now();
        let tap = Tap {
            close,
            press_answered_ms: self.last_press_ms.take(),
            release_answered_ms,
            windows_saw_press,
        };
        if !self.guard.tapped(now, tap) {
            tracing::debug!("Start or Search was open before this tap; not watching it");
            self.stop_poll();
            return;
        }
        self.tapped_at = Some(now);
        self.start_shown_at = None;
        self.start_hidden_at = None;
        self.start_poll();
        // When the hook answered late, Windows has already acted on the
        // key and Start may be in front by now.
        self.check();
    }

    fn escape_down(&mut self) {
        self.guard.escape_down();
        self.stop_poll();
    }

    fn start_visibility(&mut self, visible: bool) {
        if !self.guard.is_watching() {
            if visible {
                // Clicked, Ctrl+Esc, or a tap the hook never saw.
                tracing::debug!("Start opened (not right after a handled tap)");
            }
            return;
        }
        let now = Some(Instant::now());
        if visible {
            self.start_shown_at = self.start_shown_at.or(now);
        } else {
            self.start_hidden_at = now;
        }
        self.check();
    }

    fn answered(&mut self, answered_ms: u32, flags: isize) {
        let release = flags & 1 != 0;
        let side = usize::from(flags & 2 != 0);
        if !release {
            // Every press, auto-repeats too: a new press is never part of
            // the last tap.
            self.guard.win_key_down(front_window(&self.windows_dir));
            self.stop_poll();
        }
        let was_down = std::mem::replace(&mut self.keys_down[side], !release);
        if !release && was_down {
            return; // auto-repeat
        }
        if !release {
            self.last_press_ms = Some(answered_ms);
        }
        let what = if release { "release" } else { "press" };
        tracing::debug!("windows key {what} answered {answered_ms} ms after the key");
        if answered_ms >= LATE_MS {
            LATE_ANSWERS.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                "the Windows key hook answered a {what} {answered_ms} ms after the key; \
                 past about 300 ms Windows uses the key anyway"
            );
        }
    }

    fn check(&mut self) {
        let now = Instant::now();
        let front = front_window(&self.windows_dir);
        match self.guard.check(now, front, modifiers_down()) {
            Step::Idle => self.stop_poll(),
            Step::Wait => {}
            Step::PressEscape => {
                if press_escape(&self.windows_dir) {
                    self.guard.escape_pressed(now);
                }
            }
            Step::Done(report) => {
                self.stop_poll();
                self.finished(report);
            }
        }
    }

    fn finished(&mut self, report: Option<Report>) {
        let since_tap = |t: Option<Instant>| {
            t.zip(self.tapped_at)
                .map(|(t, tap)| t.saturating_duration_since(tap).as_millis())
        };
        let shown = since_tap(self.start_shown_at);
        let Some(report) = report else {
            // Start opened but never came to the front as a window we know:
            // say what was there, so the matching can be fixed.
            if let Some(shown) = shown {
                let front = describe_front();
                match since_tap(self.start_hidden_at) {
                    Some(hidden) => tracing::info!(
                        "Start opened {shown} ms after a Windows key tap and closed again at \
                         {hidden} ms without coming to the front (now in front: {front})"
                    ),
                    None => tracing::info!(
                        "Start opened {shown} ms after a Windows key tap, but the window in \
                         front was {front}; left it alone"
                    ),
                }
            }
            return;
        };
        let event = shown
            .map(|ms| format!(" (Start's visibility event came {ms} ms after the tap)"))
            .unwrap_or_default();
        match report.ending {
            Ending::StayedOpen => {
                tracing::warn!("{report}{event}");
                crate::toast::report(
                    "Start opened after a Windows key tap and wouldn't close; the log has \
                     the timings",
                );
            }
            _ => tracing::info!("{report}{event}"),
        }
        if report.ending == Ending::Closed {
            START_CLOSED.fetch_add(1, Ordering::Relaxed);
            (self.callbacks.on_closed)();
        }
    }

    fn start_poll(&mut self) {
        if self.timer == 0 {
            // SAFETY: a thread timer (no window); WM_TIMER comes with no hwnd.
            self.timer = unsafe { SetTimer(None, 0, POLL_MS, None) };
        }
    }

    fn stop_poll(&mut self) {
        if self.timer != 0 {
            // SAFETY: the id came from SetTimer on this thread.
            let _ = unsafe { KillTimer(None, self.timer) };
            self.timer = 0;
        }
    }
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    // Re-read the foreground rather than trusting `hwnd`, so every source
    // goes through the same decision.
    with_ctx(|c| {
        if c.guard.is_watching() {
            c.check();
        }
    });
}
