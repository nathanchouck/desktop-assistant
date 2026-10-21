//! Other platforms: there are no windows to move (the app is Windows-only;
//! this keeps it building and testable elsewhere).

use super::geometry::Rect;
use super::{Show, WindowState};

pub const SUPPORTED: bool = false;

const ONLY_WINDOWS: &str = "Window layouts work only on Windows.";

pub fn read(_handle: isize) -> Result<WindowState, String> {
    Err(ONLY_WINDOWS.into())
}

pub fn show(_handle: isize, _show: Show) -> Result<(), String> {
    Err(ONLY_WINDOWS.into())
}

pub fn place(_handle: isize, _target: Rect) -> Result<Rect, String> {
    Err(ONLY_WINDOWS.into())
}
