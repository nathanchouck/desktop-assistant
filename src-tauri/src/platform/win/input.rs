//! Pressing keys and bringing windows to the front, for pasting into the
//! window you were in and the like.

use std::time::{Duration, Instant};

use windows::core::{Error, Result};
use windows::Win32::Foundation::{E_FAIL, HWND};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow, SetForegroundWindow,
    ShowWindow, SW_RESTORE,
};

use crate::platform::{vk, INJECTED_TAG};

/// How long to wait for you to let go of Shift, Ctrl, Alt or Win.
const MODIFIER_WAIT: Duration = Duration::from_millis(600);

pub fn focus_window(handle: isize) -> Result<()> {
    let hwnd = HWND(handle as *mut _);
    // SAFETY: plain Win32 calls on a window handle; they fail harmlessly if
    // the window is gone.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err(Error::new(E_FAIL, "that window has closed"));
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            return Ok(());
        }
        // Windows only lets the app in front hand over the foreground.
        // Borrow its input queue for a moment, then try again.
        let front = GetForegroundWindow();
        let front_thread = GetWindowThreadProcessId(front, None);
        let ours = GetCurrentThreadId();
        let attached = front_thread != 0
            && front_thread != ours
            && AttachThreadInput(ours, front_thread, true).as_bool();
        let ok = SetForegroundWindow(hwnd).as_bool();
        if attached {
            let _ = AttachThreadInput(ours, front_thread, false);
        }
        if ok {
            Ok(())
        } else {
            Err(Error::new(
                E_FAIL,
                "Windows didn't let the window come to the front",
            ))
        }
    }
}

/// Presses `keys` as one shortcut: down in order, up in reverse.
pub fn press_keys(keys: &[u16]) -> Result<()> {
    wait_for_modifiers_up();
    let mut inputs: Vec<INPUT> = keys.iter().map(|&k| key(k, false)).collect();
    inputs.extend(keys.iter().rev().map(|&k| key(k, true)));
    // SAFETY: `inputs` is a valid slice for the duration of the call.
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(Error::from_thread())
    }
}

/// So a shortcut you're still holding (Ctrl+Enter) doesn't mix into the
/// keys we press.
fn wait_for_modifiers_up() {
    let started = Instant::now();
    let held = || {
        [vk::SHIFT, vk::CONTROL, vk::MENU, vk::LWIN, vk::RWIN]
            .into_iter()
            // SAFETY: no preconditions. High bit set = down.
            .any(|k| unsafe { GetAsyncKeyState(i32::from(k)) } < 0)
    };
    while held() && started.elapsed() < MODIFIER_WAIT {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: INJECTED_TAG,
            },
        },
    }
}
