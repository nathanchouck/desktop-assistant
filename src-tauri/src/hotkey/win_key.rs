//! Windows only: a low-level keyboard hook that turns a lone tap of the
//! Windows key into "toggle the launcher", while Win+<key> shortcuts still
//! work. `tap.rs` decides; this file carries it out (D-022):
//!
//! - A Windows key press is hidden until another key shows it's a shortcut;
//!   then Win is injected just before that key. Keys injected from inside a
//!   hook are handled *before* the key being hooked (D-015), so the order is
//!   right. A lone tap is hidden entirely, so Start never sees it.
//! - A release Windows did see is replayed as mask-down, mask-up, Win-up in
//!   one `SendInput`; the mask is an unassigned key, so Windows sees
//!   "Win+mask", a shortcut, and doesn't open Start.
//!
//! The hook lives on its own high-priority thread with a message loop,
//! because Windows calls a low-level hook on the thread that installed it,
//! and skips it if it's slow to answer (`LowLevelHooksTimeout`). So the
//! callback only updates the detector, injects keys, and hands work off.

use std::cell::RefCell;
use std::ptr::null_mut;
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Shutdown::LockWorkStation;
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, MSG,
    PM_NOREMOVE, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::tap::{KeyEvent, Side, TapDetector, Verdict};

/// Scan codes for the Windows keys (both "extended").
const SCAN_LWIN: u16 = 0x5B;
const SCAN_RWIN: u16 = 0x5C;
use super::HotkeyError;

/// An unassigned virtual-key code. Pressing it while Win is held makes
/// Windows treat the press as a combo, so releasing Win doesn't open Start.
/// AutoHotkey uses the same key for the same purpose.
const MASK_VK: u16 = 0xE8;

/// Marks events we inject so the hook can recognize and skip them.
const INJECTED_TAG: usize = 0x4441_5354;

struct HookState {
    detector: TapDetector,
    on_tap: Box<dyn Fn()>,
}

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

/// Owns the hook thread. Dropping it removes the hook.
pub struct WinKeyHook {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Drop for WinKeyHook {
    fn drop(&mut self) {
        // SAFETY: posting to a thread id is always safe; it fails harmlessly
        // if the thread has already exited.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Installs the hook. `on_tap` runs on the hook thread, so it must be quick:
/// post work elsewhere instead of doing it inline.
pub fn start(on_tap: impl Fn() + Send + 'static) -> Result<WinKeyHook, HotkeyError> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let on_tap: Box<dyn Fn() + Send> = Box::new(on_tap);
    let thread = std::thread::Builder::new()
        .name("win-key-hook".into())
        .spawn(move || run_hook_thread(on_tap, ready_tx))
        .map_err(|e| HotkeyError::Hook(e.to_string()))?;

    match ready_rx.recv() {
        Ok(Ok(thread_id)) => Ok(WinKeyHook {
            thread_id,
            thread: Some(thread),
        }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => Err(HotkeyError::Hook(
            "hook thread exited during startup".into(),
        )),
    }
}

fn run_hook_thread(on_tap: Box<dyn Fn() + Send>, ready: mpsc::Sender<Result<u32, HotkeyError>>) {
    STATE.with(|s| {
        *s.borrow_mut() = Some(HookState {
            detector: TapDetector::default(),
            on_tap,
        })
    });

    // SAFETY: standard Win32 hook + message loop, all on this thread. The hook
    // procedure only runs while this thread is inside GetMessageW.
    unsafe {
        // Answer key events promptly even when the app is busy (indexing,
        // searching): a late answer makes Windows skip the hook.
        if SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) == 0 {
            tracing::debug!("could not raise the keyboard hook's priority");
        }
        let mut msg: MSG = std::mem::zeroed();
        // Create this thread's message queue now, so a WM_QUIT posted right
        // after startup isn't lost.
        PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);

        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(hook_proc),
            GetModuleHandleW(std::ptr::null()),
            0,
        );
        if hook.is_null() {
            let err = std::io::Error::last_os_error();
            let _ = ready.send(Err(HotkeyError::Hook(err.to_string())));
            return;
        }
        let _ = ready.send(Ok(GetCurrentThreadId()));

        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(hook);
    }
    tracing::debug!("windows key hook stopped");
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
        let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if kb.dwExtraInfo != INJECTED_TAG {
            if let Some(event) = to_event(kb.vkCode, wparam as u32, modifiers_held) {
                match detect(event) {
                    Verdict::Pass => {}
                    Verdict::Swallow => return 1,
                    Verdict::Tap => {
                        notify_tap();
                        return 1;
                    }
                    Verdict::PressWinFirst { left, right } => {
                        if !press_win(left, right) {
                            tracing::debug!("could not press the Windows key for a shortcut");
                        }
                        // The key itself goes through after the injected Win.
                    }
                    Verdict::Lock => {
                        // SAFETY: no preconditions; it fails harmlessly if
                        // locking is disabled by policy.
                        unsafe { LockWorkStation() };
                        return 1;
                    }
                    Verdict::MaskedRelease => {
                        if replay_masked_release(kb.vkCode, kb.scanCode) {
                            // The replay above stands in for this key-up.
                            return 1;
                        }
                        tracing::debug!("could not mask the Windows key; Start may open");
                    }
                }
            }
        }
    }
    // SAFETY: passing the event on unchanged, as Windows requires.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

/// Maps a raw keyboard message to a detector event. Key-ups of other keys
/// don't matter to the detector, so they map to `None`.
fn to_event(vk: u32, message: u32, modifiers_held: fn() -> bool) -> Option<KeyEvent> {
    let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
    let up = matches!(message, WM_KEYUP | WM_SYSKEYUP);
    let side = match u16::try_from(vk).ok()? {
        VK_LWIN => Some(Side::Left),
        VK_RWIN => Some(Side::Right),
        _ => None,
    };
    match (side, down, up) {
        (Some(side), true, _) => Some(KeyEvent::WinDown {
            side,
            modifiers_held: modifiers_held(),
        }),
        (Some(side), _, true) => Some(KeyEvent::WinUp { side }),
        (None, true, _) => Some(KeyEvent::OtherDown { vk }),
        _ => None,
    }
}

fn modifiers_held() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU]
        .into_iter()
        // SAFETY: GetAsyncKeyState has no preconditions. High bit set = down.
        .any(|vk| unsafe { GetAsyncKeyState(i32::from(vk)) } < 0)
}

fn detect(event: KeyEvent) -> Verdict {
    STATE.with(|s| match s.try_borrow_mut() {
        Ok(mut state) => state
            .as_mut()
            .map_or(Verdict::Pass, |st| st.detector.on_event(event)),
        Err(_) => Verdict::Pass,
    })
}

fn notify_tap() {
    STATE.with(|s| {
        if let Ok(state) = s.try_borrow() {
            if let Some(st) = state.as_ref() {
                (st.on_tap)();
            }
        }
    });
}

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTED_TAG,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> bool {
    // SAFETY: `inputs` is a valid slice of INPUT for the duration of the call.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    sent == inputs.len() as u32
}

/// Presses the hidden Windows key(s) for Windows, ahead of the key being
/// hooked, so the shortcut works.
fn press_win(left: bool, right: bool) -> bool {
    let mut inputs = Vec::with_capacity(2);
    if left {
        inputs.push(key_input(VK_LWIN, SCAN_LWIN, KEYEVENTF_EXTENDEDKEY));
    }
    if right {
        inputs.push(key_input(VK_RWIN, SCAN_RWIN, KEYEVENTF_EXTENDEDKEY));
    }
    inputs.is_empty() || send(&inputs)
}

/// Injects mask-down, mask-up, Win-up as one batch. Returns true only if all
/// three went in; otherwise the caller lets the real key-up through, so
/// Windows can never be left thinking Win is still held.
fn replay_masked_release(win_vk: u32, scan: u32) -> bool {
    let (Ok(win_vk), Ok(scan)) = (u16::try_from(win_vk), u16::try_from(scan)) else {
        return false;
    };
    let key = key_input;
    let inputs = [
        key(MASK_VK, 0, 0),
        key(MASK_VK, 0, KEYEVENTF_KEYUP),
        key(win_vk, scan, KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY),
    ];
    send(&inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> bool {
        false
    }

    fn held() -> bool {
        true
    }

    #[test]
    fn maps_windows_keys() {
        assert_eq!(
            to_event(u32::from(VK_LWIN), WM_KEYDOWN, none),
            Some(KeyEvent::WinDown {
                side: Side::Left,
                modifiers_held: false
            })
        );
        assert_eq!(
            to_event(u32::from(VK_RWIN), WM_SYSKEYUP, none),
            Some(KeyEvent::WinUp { side: Side::Right })
        );
        assert_eq!(
            to_event(u32::from(VK_LWIN), WM_KEYDOWN, held),
            Some(KeyEvent::WinDown {
                side: Side::Left,
                modifiers_held: true
            })
        );
    }

    #[test]
    fn maps_other_keys() {
        assert_eq!(
            to_event(0x45, WM_KEYDOWN, none),
            Some(KeyEvent::OtherDown { vk: 0x45 })
        );
        assert_eq!(to_event(0x45, WM_KEYUP, none), None);
    }
}
