//! Other platforms: system commands do nothing (the app is Windows-only;
//! this keeps it building and testable elsewhere).

use super::commands::{BinContents, Exit};

pub const SUPPORTED: bool = false;

const ONLY_WINDOWS: &str = "only on Windows";

pub fn lock() -> Result<(), String> {
    Err(ONLY_WINDOWS.into())
}

pub fn hibernate_available() -> bool {
    false
}

pub fn suspend(_hibernate: bool) -> Result<(), String> {
    Err(ONLY_WINDOWS.into())
}

pub fn exit_windows(_exit: Exit) -> Result<(), String> {
    Err(ONLY_WINDOWS.into())
}

pub fn recycle_bin() -> Result<BinContents, String> {
    Err(ONLY_WINDOWS.into())
}

pub fn empty_recycle_bin() -> Result<(), String> {
    Err(ONLY_WINDOWS.into())
}

pub fn apps_use_light_theme() -> Option<bool> {
    None
}

pub fn toggle_theme() -> Result<bool, String> {
    Err(ONLY_WINDOWS.into())
}
