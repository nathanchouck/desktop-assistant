//! Windows: picking a color off the screen.
//!
//! The system cursors turn into a crosshair, and low-level mouse and
//! keyboard hooks on this thread watch the next click or key (see
//! `picking.rs`), keeping it from the app underneath. Then the pixel is
//! copied out of the screen in physical pixels, so it's exact at any
//! display scale.
//!
//! Replaced system cursors outlive a crash, so a marker file is written
//! while they're replaced; `recover` puts them back at the next start.

use std::cell::RefCell;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::Duration;

use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetPixel,
    ReleaseDC, SelectObject, CAPTUREBLT, CLR_INVALID, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CopyIcon, GetCursorPos, GetMessageW, KillTimer, LoadCursorW, PeekMessageW,
    PostQuitMessage, SetSystemCursor, SetTimer, SetWindowsHookExW, SystemParametersInfoW,
    UnhookWindowsHookEx, HC_ACTION, HHOOK, IDC_CROSS, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
    OCR_APPSTARTING, OCR_HAND, OCR_IBEAM, OCR_NO, OCR_NORMAL, OCR_SIZEALL, OCR_SIZENESW,
    OCR_SIZENS, OCR_SIZENWSE, OCR_SIZEWE, OCR_UP, OCR_WAIT, PM_NOREMOVE, SPI_SETCURSORS,
    SYSTEM_CURSOR_ID, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_TIMER,
};

use super::model::Rgb;
use super::picking::{Input, Picking, Reply};

pub const SUPPORTED: bool = true;

/// Every cursor shape an app might show while you aim.
const CURSORS: [SYSTEM_CURSOR_ID; 12] = [
    OCR_NORMAL,
    OCR_IBEAM,
    OCR_HAND,
    OCR_APPSTARTING,
    OCR_WAIT,
    OCR_SIZEALL,
    OCR_SIZENESW,
    OCR_SIZENS,
    OCR_SIZENWSE,
    OCR_SIZEWE,
    OCR_NO,
    OCR_UP,
];

thread_local! {
    /// The pick in progress on this thread (the hooks run on it).
    static PICK: RefCell<Picking> = RefCell::new(Picking::default());
}

/// Waits for a click (or Enter) and returns the color under it; `None`
/// if you cancelled or `timeout` passed. Blocks; run on its own thread.
pub fn pick(timeout: Duration, marker: &Path) -> Result<Option<Rgb>, String> {
    // SAFETY: this thread's own DPI mode, so the hook's points and the
    // screen copy below both use physical pixels.
    unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    PICK.with(|p| *p.borrow_mut() = Picking::default());
    // SAFETY: hooks and a timer on this thread with its message loop; the
    // hook procedures only run while it waits in GetMessageW. Everything
    // is removed before returning.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        // Create the message queue before anything can post to it.
        PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
        let module = GetModuleHandleW(null());
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(on_mouse), module, 0);
        let keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), module, 0);
        if mouse.is_null() || keys.is_null() {
            let err = std::io::Error::last_os_error().to_string();
            unhook(mouse);
            unhook(keys);
            return Err(err);
        }
        let crosshair = Crosshair::show(marker);
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let timer = SetTimer(null_mut(), 0, ms, None);
        // Ends with WM_QUIT (a hook finished), WM_TIMER, or an error.
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == WM_TIMER {
                break;
            }
        }
        KillTimer(null_mut(), timer);
        unhook(mouse);
        unhook(keys);
        drop(crosshair);
    }
    match PICK.with(|p| p.borrow().point()) {
        Some((x, y)) => pixel_at(x, y).map(Some),
        None => Ok(None),
    }
}

unsafe fn unhook(hook: HHOOK) {
    if !hook.is_null() {
        // SAFETY: a hook this thread installed.
        unsafe { UnhookWindowsHookEx(hook) };
    }
}

/// Hands an input to the pick in progress and answers the hook.
fn reply(input: Input) -> Reply {
    let reply = PICK.with(|p| p.borrow_mut().on(input));
    if reply == Reply::Finish {
        // SAFETY: ends this thread's message loop (the hooks run on it).
        unsafe { PostQuitMessage(0) };
    }
    reply
}

unsafe extern "system" fn on_mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, `lparam` points at an MSLLHOOKSTRUCT.
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        let input = match wparam as u32 {
            WM_LBUTTONDOWN => Some(Input::LeftDown {
                x: info.pt.x,
                y: info.pt.y,
            }),
            WM_LBUTTONUP => Some(Input::LeftUp),
            WM_RBUTTONDOWN => Some(Input::RightDown),
            WM_RBUTTONUP => Some(Input::RightUp),
            _ => None,
        };
        if input.is_some_and(|i| reply(i) != Reply::Pass) {
            return 1;
        }
    }
    // SAFETY: passing the event on, as every hook must.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    if code == HC_ACTION as i32 && down {
        // SAFETY: for HC_ACTION, `lparam` points at a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let input = match info.vkCode {
            vk if vk == u32::from(VK_ESCAPE) => Some(Input::Escape),
            vk if vk == u32::from(VK_RETURN) => {
                let mut at = POINT { x: 0, y: 0 };
                // SAFETY: `at` is a valid out parameter.
                unsafe { GetCursorPos(&mut at) };
                Some(Input::Enter { x: at.x, y: at.y })
            }
            _ => None,
        };
        if input.is_some_and(|i| reply(i) != Reply::Pass) {
            return 1;
        }
    }
    // SAFETY: passing the event on, as every hook must.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

/// The crosshair in place of the usual cursors, until dropped.
struct Crosshair<'a> {
    marker: &'a Path,
}

impl<'a> Crosshair<'a> {
    fn show(marker: &'a Path) -> Self {
        if let Err(e) = std::fs::write(marker, b"") {
            tracing::debug!("could not note the cursor change: {e}");
        }
        for id in CURSORS {
            // SAFETY: SetSystemCursor takes ownership of (and destroys) the
            // copy; the shared cross cursor itself is never handed over.
            unsafe {
                let cross = LoadCursorW(null_mut(), IDC_CROSS);
                let copy = CopyIcon(cross);
                if !copy.is_null() {
                    SetSystemCursor(copy, id);
                }
            }
        }
        Self { marker }
    }
}

impl Drop for Crosshair<'_> {
    fn drop(&mut self) {
        restore_cursors();
        let _ = std::fs::remove_file(self.marker);
    }
}

/// Reloads the cursors you chose in Windows' settings.
fn restore_cursors() {
    // SAFETY: no pointers involved for this action.
    unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, null_mut(), 0) };
}

/// Puts the cursors back if the launcher stopped while picking.
pub fn recover(marker: &Path) {
    if marker.exists() {
        tracing::info!("restoring the cursors after an interrupted color pick");
        restore_cursors();
        let _ = std::fs::remove_file(marker);
    }
}

/// The color of one screen pixel.
fn pixel_at(x: i32, y: i32) -> Result<Rgb, String> {
    // SAFETY: GDI objects created here are selected out and deleted before
    // returning; the screen DC is released.
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("couldn't read the screen".into());
        }
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, 1, 1);
        let old = SelectObject(memory, bitmap.into());
        // CAPTUREBLT includes layered (see-through) windows.
        let copied = BitBlt(memory, 0, 0, 1, 1, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
        let color = GetPixel(memory, 0, 0);
        SelectObject(memory, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        copied.map_err(|e| e.message())?;
        if color.0 == CLR_INVALID {
            return Err("couldn't read that pixel".into());
        }
        // COLORREF is 0x00BBGGRR.
        let [r, g, b, _] = color.0.to_le_bytes();
        Ok(Rgb::new(r, g, b))
    }
}
