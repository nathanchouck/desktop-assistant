//! Windows only: is Start (or Windows Search) the window in front, and
//! closing it with one Escape. Used by the Start guard thread.

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
};

use super::start_guard::{flyout_for_process, Front};
use crate::platform::INJECTED_TAG;

/// `%SystemRoot%`, where the shell's flyout hosts live.
pub fn windows_dir() -> String {
    std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())
}

/// What's in front: Start/Search (by program), another window, or none.
pub fn front_window(windows_dir: &str) -> Front {
    // SAFETY: no preconditions.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return Front::Unknown;
    }
    let Some(pid) = window_process(hwnd) else {
        return Front::Unknown;
    };
    if pid == std::process::id() {
        return Front::Other;
    }
    process_image(pid)
        .and_then(|image| flyout_for_process(&image, windows_dir))
        .map_or(Front::Other, Front::Flyout)
}

fn window_process(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: valid out-pointer; a window that just closed yields 0.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

fn process_image(pid: u32) -> Option<String> {
    // SAFETY: the handle is checked and always closed; the buffer and its
    // length go together.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        ok.ok()?;
        buf.get(..len as usize).map(String::from_utf16_lossy)
    }
}

/// The window in front as "<class> from <program path>", for the log.
pub fn describe_front() -> String {
    // SAFETY: no preconditions.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return "nothing".into();
    }
    let mut class = [0u16; 256];
    // SAFETY: a valid buffer; a closed window just yields 0.
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    let class = String::from_utf16_lossy(class.get(..len.max(0) as usize).unwrap_or(&[]));
    let program = window_process(hwnd)
        .and_then(process_image)
        .unwrap_or_else(|| "an unknown program".into());
    format!("{class} from {program}")
}

/// Ctrl, Alt, Shift or a Windows key is held, so an Escape now would mean
/// something else (Ctrl+Esc opens Start, Alt+Esc switches windows).
pub fn modifiers_down() -> bool {
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .into_iter()
        // SAFETY: no preconditions. High bit set = down.
        .any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0)
}

/// One Escape for Start/Search, if it's still in front with no modifier
/// held. Checking again right before sending shrinks (it can't remove) the
/// moment in which focus could move elsewhere before the key lands. Tagged
/// so our own keyboard hook lets it through untouched.
pub fn press_escape(windows_dir: &str) -> bool {
    if !matches!(front_window(windows_dir), Front::Flyout(_)) || modifiers_down() {
        return false;
    }
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_ESCAPE,
                wScan: 0x01,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTED_TAG,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    // SAFETY: a valid slice of INPUT for the duration of the call.
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}
