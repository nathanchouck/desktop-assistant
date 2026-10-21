//! Screenshots of one window, by handle: `PrintWindow` draws that window
//! alone, even where others (the launcher included) cover it (D-019).

use windows::core::{Error, Result};
use windows::Win32::Foundation::{E_FAIL, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindow};

use super::icon::read_bitmap;
use crate::platform::Image;

/// Asks the window to draw everything, including GPU-rendered content.
const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);

pub fn capture_window(handle: isize) -> Result<Image> {
    let hwnd = HWND(handle as *mut _);
    // SAFETY: GDI objects are created and released here in reverse order;
    // the window is only drawn into our own bitmap.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err(Error::new(E_FAIL, "that window has closed"));
        }
        if IsIconic(hwnd).as_bool() {
            return Err(Error::new(E_FAIL, "that window is minimized"));
        }
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect)?;
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        if width <= 0 || height <= 0 {
            return Err(Error::new(E_FAIL, "that window has no size"));
        }

        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(memory, bitmap.into());
        let drawn = PrintWindow(hwnd, memory, PW_RENDERFULLCONTENT).as_bool();
        SelectObject(memory, previous);
        let image = if drawn {
            read_bitmap(bitmap, true)
        } else {
            Err(Error::from_thread())
        };
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        image
    }
}
