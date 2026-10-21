//! Window layouts for the window you were in when the launcher opened:
//! halves, thirds, two thirds, maximize, almost maximize, center, restore,
//! and move to the next monitor.
//!
//! - `layouts.rs`: the commands and where each puts a window (pure).
//! - `geometry.rs`: the rectangle math, incl. DWM's invisible borders (pure).
//! - `restore.rs`: where windows were before the launcher moved them (pure).
//! - `provider.rs`: search rows, and the steps of a layout.
//! - `win.rs` (Windows): reading and moving windows; `fallback.rs` elsewhere.

#[cfg(not(windows))]
mod fallback;
mod geometry;
mod layouts;
mod provider;
mod restore;
#[cfg(windows)]
mod win;

use std::sync::Arc;

use tauri::AppHandle;

use super::ProviderContext;
use crate::search::SearchProvider;
use layouts::Placement;

/// Where a window is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowState {
    pub placement: Placement,
    pub maximized: bool,
    pub minimized: bool,
}

/// Window states Windows itself switches between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    Restore,
    Maximize,
}

/// Window classes that aren't app windows: the desktop, the taskbars, and
/// Start, Search and the other shell flyouts.
pub fn can_move(class: &str) -> bool {
    !matches!(
        class,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "Windows.UI.Core.CoreWindow"
            | "XamlExplorerHostIslandWindow"
            | "TopLevelWindowForOverflowXamlIsland"
            | "NotifyIconOverflowWindow"
    )
}

/// Window commands in search.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(provider::LayoutProvider::new(
        Arc::clone(&ctx.desktop),
        Arc::clone(&ctx.shell),
    )))
}

/// Nothing runs in the background: windows are read when a command runs.
pub fn start(_app: &AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_and_taskbar_dont_move() {
        assert!(!can_move("Progman"));
        assert!(!can_move("WorkerW"));
        assert!(!can_move("Shell_TrayWnd"));
        assert!(!can_move("Windows.UI.Core.CoreWindow"));
        assert!(can_move("Notepad"));
        assert!(can_move("CabinetWClass"));
        assert!(can_move("Chrome_WidgetWin_1"));
    }
}
