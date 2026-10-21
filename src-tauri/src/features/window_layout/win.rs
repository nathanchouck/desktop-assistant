//! Windows: where a window is (its visible frame, its monitor's work area,
//! every monitor), and moving it there, all in physical pixels.

use std::ffi::c_void;

use windows::Win32::Foundation::{E_ACCESSDENIED, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongW, GetWindowRect, IsHungAppWindow, IsIconic, IsWindow, IsZoomed, SetWindowPos,
    ShowWindow, GWL_STYLE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SW_MAXIMIZE,
    SW_RESTORE, WS_THICKFRAME,
};

use super::geometry::{Insets, Rect};
use super::layouts::Placement;
use super::{Show, WindowState};

pub const SUPPORTED: bool = true;

/// Moving a window onto a monitor with another scale makes it resize
/// itself for the new DPI, so the size is set again (twice is usually
/// enough; three covers apps that round to their own steps).
const ATTEMPTS: usize = 3;
/// Close enough: apps such as terminals round to whole character cells.
const SLACK: i32 = 2;

/// Switches the calling thread to physical pixels until dropped, so every
/// coordinate here means the same thing whatever the monitor's scale.
struct PhysicalPixels(DPI_AWARENESS_CONTEXT);

impl PhysicalPixels {
    fn new() -> Self {
        // SAFETY: changes only this thread's DPI mode; undone on drop.
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}

impl Drop for PhysicalPixels {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: puts back the mode the thread had.
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

fn hwnd(handle: isize) -> HWND {
    HWND(handle as *mut c_void)
}

fn rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}

/// Where the window is, or why it can't be moved.
pub fn read(handle: isize) -> Result<WindowState, String> {
    let _pixels = PhysicalPixels::new();
    let hwnd = hwnd(handle);
    // SAFETY: read-only queries on a window handle; they fail harmlessly
    // if the window has gone.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err("That window has closed.".into());
        }
        if IsHungAppWindow(hwnd).as_bool() {
            return Err("That window isn't responding.".into());
        }
        let (window, frame) = bounds(hwnd)?;
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let work = work_area(monitor).ok_or("Couldn't find the window's monitor.")?;
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        Ok(WindowState {
            placement: Placement {
                frame,
                work,
                works: work_areas(),
                resizable: style & WS_THICKFRAME.0 != 0,
            },
            insets: Insets::between(window, frame),
            maximized: IsZoomed(hwnd).as_bool(),
            minimized: IsIconic(hwnd).as_bool(),
        })
    }
}

/// The window rectangle and its visible frame (without DWM's invisible
/// borders; the same rectangle when DWM doesn't say).
fn bounds(hwnd: HWND) -> Result<(Rect, Rect), String> {
    let mut window = RECT::default();
    // SAFETY: `window` is a valid out parameter.
    unsafe { GetWindowRect(hwnd, &mut window) }.map_err(|e| e.message())?;
    let mut frame = RECT::default();
    // SAFETY: `frame` is a RECT, the size DWM writes for this attribute.
    let framed = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            std::ptr::from_mut(&mut frame).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    };
    let window = rect(window);
    Ok(match framed {
        Ok(()) => (window, rect(frame)),
        Err(_) => (window, window),
    })
}

fn work_area(monitor: HMONITOR) -> Option<Rect> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is sized and valid for the call.
    unsafe { GetMonitorInfoW(monitor, &mut info) }
        .as_bool()
        .then(|| rect(info.rcWork))
}

/// Every monitor's work area.
fn work_areas() -> Vec<Rect> {
    unsafe extern "system" fn each(
        monitor: HMONITOR,
        _dc: HDC,
        _bounds: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        // SAFETY: `data` is the `Vec` below, alive for the whole enumeration.
        let works = unsafe { &mut *(data.0 as *mut Vec<Rect>) };
        if let Some(work) = work_area(monitor) {
            works.push(work);
        }
        true.into()
    }
    let mut works: Vec<Rect> = Vec::new();
    // SAFETY: the callback only runs during this call, while `works` lives.
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(each),
            LPARAM(std::ptr::from_mut(&mut works) as isize),
        )
    };
    works
}

/// Restores or maximizes the window (it animates and keeps Windows'
/// own memory of its normal size).
pub fn show(handle: isize, show: Show) -> Result<(), String> {
    let command = match show {
        Show::Restore => SW_RESTORE,
        Show::Maximize => SW_MAXIMIZE,
    };
    // SAFETY: a window handle checked in `read`; ShowWindow returns the
    // previous visibility, not an error.
    let _ = unsafe { ShowWindow(hwnd(handle), command) };
    Ok(())
}

/// Moves and sizes the window so its visible frame is `target`. Returns
/// the frame it ended up with.
pub fn place(handle: isize, target: Rect) -> Result<Rect, String> {
    let _pixels = PhysicalPixels::new();
    let hwnd = hwnd(handle);
    let mut landed = target;
    for _ in 0..ATTEMPTS {
        let (window, frame) = bounds(hwnd)?;
        // Borders differ per scale, so they're measured again each time.
        let want = Insets::between(window, frame).grow(target);
        // SAFETY: a window handle; fails harmlessly if it has gone. No
        // z-order or focus change here: the window is brought to the
        // front after the launcher hides.
        unsafe {
            SetWindowPos(
                hwnd,
                None,
                want.left,
                want.top,
                want.width(),
                want.height(),
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        }
        .map_err(|e| {
            if e.code() == E_ACCESSDENIED {
                "Windows doesn't let the launcher move that window (it runs as administrator)."
                    .to_owned()
            } else {
                e.message()
            }
        })?;
        landed = bounds(hwnd)?.1;
        if landed.near(&target, SLACK) {
            break;
        }
    }
    Ok(landed)
}
