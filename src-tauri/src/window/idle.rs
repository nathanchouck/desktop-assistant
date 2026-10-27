//! Giving memory back while the launcher sits hidden (Windows).
//!
//! The window is hidden, never closed, so its WebView2 keeps everything it
//! loaded. After `IDLE_AFTER` hidden:
//! - WebView2 is asked to use less (`MemoryUsageTargetLevel` Low): its
//!   processes drop caches and let Windows page out what they can.
//! - This process's working set is trimmed, so idle pages leave RAM, but
//!   only while no low-level keyboard hook runs (the Windows key override,
//!   snippet expansion): a hook whose data was paged out answers late, and
//!   Windows then uses the key itself (D-031).
//!
//! Opening the launcher sets WebView2 back to Normal before the window
//! shows. Trimmed pages come back on first touch, mostly from memory
//! Windows still holds (a soft fault), so opening stays quick.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::state::AppState;

/// Hidden this long counts as idle.
pub const IDLE_AFTER: Duration = Duration::from_secs(2 * 60);

/// Whether the launcher has gone idle since it was last shown.
#[derive(Default)]
pub struct Idle {
    /// Bumped on every hide and show, so a timer from an older hide knows
    /// the launcher opened in between.
    generation: AtomicU64,
    low: AtomicBool,
}

impl Idle {
    /// The launcher hid: returns what the idle timer must still see.
    fn hidden(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// The timer ran out: whether the launcher stayed hidden all along
    /// (then it's idle now).
    fn timer_ended(&self, generation: u64) -> bool {
        let still_hidden = self.generation.load(Ordering::Acquire) == generation;
        if still_hidden {
            self.low.store(true, Ordering::Release);
        }
        still_hidden
    }

    /// The launcher is about to show: whether it was idle (and memory
    /// needs to go back to normal).
    fn shown(&self) -> bool {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.low.swap(false, Ordering::AcqRel)
    }
}

/// Starts the idle timer for this hide.
pub fn on_hide(app: &AppHandle, idle: &Idle) {
    let generation = idle.hidden();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(IDLE_AFTER).await;
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        if !state.launcher.idle.timer_ended(generation) {
            return;
        }
        if let Some(window) = super::launcher(&app) {
            let settings = state.settings();
            // A keyboard hook must answer within ~300 ms or Windows uses the
            // key itself (D-031). Trimming would send the hook's data to the
            // page file, so keep this process's pages while one is on.
            let hooks_on = settings.hotkeys.win_key_override
                || (settings.snippets.enabled && settings.snippets.expand_as_you_type);
            os::use_less_memory(&window, !hooks_on);
        }
    });
}

/// Before the window shows: back to normal if it went idle.
pub fn on_show(window: &WebviewWindow, idle: &Idle) {
    if idle.shown() {
        os::use_normal_memory(window);
    }
}

#[cfg(windows)]
mod os {
    use tauri::WebviewWindow;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL,
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use windows::core::Interface;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::ProcessStatus::{
        K32EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    const MB: usize = 1024 * 1024;

    /// WebView2 to Low; this process's own pages are trimmed only when
    /// `trim` (no keyboard hook running).
    pub fn use_less_memory(window: &WebviewWindow, trim: bool) {
        set_target(window, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
        if trim {
            trim_working_set();
        } else {
            // SAFETY: the pseudo-handle for this process needs no closing.
            if let Some((set, _)) = working_set(unsafe { GetCurrentProcess() }) {
                tracing::info!(
                    "hidden for a while: WebView2 set to low memory; working set {} MB kept (keyboard hooks are on)",
                    set / MB
                );
            }
        }
    }

    pub fn use_normal_memory(window: &WebviewWindow) {
        set_target(window, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL);
        tracing::debug!("launcher memory back to normal");
    }

    /// Runs on the main thread, where the WebView2 objects live. Needs
    /// WebView2 Runtime 114+ (`ICoreWebView2_19`); older ones skip it.
    fn set_target(window: &WebviewWindow, level: COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL) {
        let queued = window.with_webview(move |webview| {
            // SAFETY: COM calls on the WebView2's own (main) thread, on
            // interfaces that live for the call.
            let set = unsafe {
                webview
                    .controller()
                    .CoreWebView2()
                    .and_then(|core| core.cast::<ICoreWebView2_19>())
                    .and_then(|core| core.SetMemoryUsageTargetLevel(level))
            };
            if let Err(e) = set {
                tracing::debug!("could not set WebView2's memory target: {e}");
            }
        });
        if let Err(e) = queued {
            tracing::debug!("could not reach the launcher's WebView2: {e}");
        }
    }

    /// Takes this process's idle pages out of RAM and logs the working set
    /// before and after (what Task Manager's "Memory" column follows).
    fn trim_working_set() {
        // SAFETY: the pseudo-handle for this process needs no closing.
        let process = unsafe { GetCurrentProcess() };
        let before = working_set(process);
        // SAFETY: as above.
        if !unsafe { K32EmptyWorkingSet(process) }.as_bool() {
            tracing::debug!("could not trim the working set");
            return;
        }
        let after = working_set(process);
        if let (Some(before), Some(after)) = (before, after) {
            tracing::info!(
                "hidden for a while: working set {} MB -> {} MB (committed {} MB)",
                before.0 / MB,
                after.0 / MB,
                after.1 / MB
            );
        }
    }

    /// (working set, committed private bytes) of `process`.
    fn working_set(process: HANDLE) -> Option<(usize, usize)> {
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        // SAFETY: `counters` is writable and its size is given.
        let ok = unsafe { K32GetProcessMemoryInfo(process, &mut counters, counters.cb) };
        ok.as_bool()
            .then_some((counters.WorkingSetSize, counters.PagefileUsage))
    }
}

/// Elsewhere (development on Linux) there's nothing to give back.
#[cfg(not(windows))]
mod os {
    use tauri::WebviewWindow;

    pub fn use_less_memory(_window: &WebviewWindow, _trim: bool) {
        tracing::debug!("launcher idle");
    }

    pub fn use_normal_memory(_window: &WebviewWindow) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_only_after_a_hide_that_lasted() {
        let idle = Idle::default();
        let first = idle.hidden();
        // Opened again before the timer ran out: that timer does nothing.
        assert!(!idle.shown());
        assert!(!idle.timer_ended(first));
        // Hidden and left alone.
        let second = idle.hidden();
        assert!(idle.timer_ended(second));
        // The next open restores memory, once.
        assert!(idle.shown());
        assert!(!idle.shown());
    }

    #[test]
    fn only_the_latest_hide_counts() {
        let idle = Idle::default();
        let old = idle.hidden();
        idle.shown();
        let new = idle.hidden();
        assert!(!idle.timer_ended(old));
        assert!(idle.timer_ended(new));
    }
}
