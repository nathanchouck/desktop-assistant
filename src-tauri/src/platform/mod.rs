//! OS-specific work behind one small interface: listing installed apps,
//! searching files (Everything), getting icons, and opening things.
//! Windows does the real work (`win/`); other platforms get stubs so the app
//! builds and tests anywhere.

#[cfg(windows)]
mod win;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Marks key presses the launcher injects, so its own keyboard hook (the
/// Windows key override) lets them through untouched.
pub const INJECTED_TAG: usize = 0x4441_5354;

/// Windows virtual-key codes, for `Shell::press_keys`.
pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const ESCAPE: u16 = 0x1B;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const C: u16 = 0x43;
    pub const H: u16 = 0x48;
    pub const V: u16 = 0x56;
}

/// An app as the OS reports it, before filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawApp {
    /// Windows: the Start menu's "parsing name" (an AppUserModelID such as
    /// `Microsoft.WindowsCalculator_8wekyb3d8bbwe!App`, or a path).
    pub target: String,
    pub name: String,
}

/// A file search for Everything: its query syntax, and how many results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSearch {
    pub search: String,
    pub max: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    pub path: String,
    pub name: String,
    /// The containing folder.
    pub dir: String,
    pub is_folder: bool,
    /// Last modified, Unix seconds.
    pub modified: Option<i64>,
}

/// The window that was in front when the launcher opened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForegroundWindow {
    /// The window handle, as a number.
    pub handle: isize,
    pub title: String,
    /// Window class, e.g. `CabinetWClass` for File Explorer.
    pub class: String,
    /// The program's file name, e.g. `explorer.exe`.
    pub app: String,
}

/// Whether a window class is File Explorer or the desktop, the windows
/// that have a folder and selection to read.
pub fn is_shell_view(class: &str) -> bool {
    matches!(
        class,
        "CabinetWClass" | "ExploreWClass" | "Progman" | "WorkerW"
    )
}

/// What a File Explorer window (or the desktop) shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExplorerView {
    /// `None` for places that aren't folders on disk (This PC, Home).
    pub folder: Option<String>,
    /// Paths of selected items (at most 50).
    pub selection: Vec<String>,
    /// How many items are selected, including any past the first 50.
    pub selected: usize,
}

/// Where an icon comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource {
    /// A Start menu app, by target.
    App(String),
    /// A file or folder on disk.
    Path(String),
}

/// A decoded image, straight (not premultiplied) RGBA, 8 bits per channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("not supported on this platform")]
    Unsupported,
    #[error("{0}")]
    Os(String),
    #[error("the background shell thread stopped")]
    WorkerGone,
    #[error("Everything isn't running")]
    EverythingNotRunning,
    /// A newer file search came in before this one started.
    #[error("superseded by a newer search")]
    Superseded,
}

/// Entry point for shell work. Cheap to share behind an `Arc`.
pub struct Shell {
    /// App listing and icons.
    #[cfg(windows)]
    worker: win::sta::StaWorker,
    /// Everything queries (the SDK needs a single thread to itself).
    #[cfg(windows)]
    everything: win::sta::StaWorker,
    /// Bumped per file search, so queued searches that are already out of
    /// date skip straight to `Superseded` instead of piling up while typing.
    file_search_generation: Arc<AtomicU64>,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            worker: win::sta::StaWorker::spawn("shell-worker"),
            #[cfg(windows)]
            everything: win::sta::StaWorker::spawn("everything"),
            file_search_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Everything in the Start menu's "All apps" list.
    pub async fn list_apps(&self) -> Result<Vec<RawApp>, PlatformError> {
        #[cfg(windows)]
        {
            self.worker.run(|| win::list_apps().map_err(os)).await?
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
    }

    pub async fn search_files(&self, search: FileSearch) -> Result<Vec<FileHit>, PlatformError> {
        let generation = self.file_search_generation.fetch_add(1, Ordering::SeqCst) + 1;
        #[cfg(windows)]
        {
            let latest = Arc::clone(&self.file_search_generation);
            self.everything
                .run(move || {
                    if latest.load(Ordering::SeqCst) != generation {
                        return Err(PlatformError::Superseded);
                    }
                    win::everything::search(&search)
                })
                .await?
        }
        #[cfg(not(windows))]
        {
            let _ = (generation, search);
            Err(PlatformError::Unsupported)
        }
    }

    /// Whether Everything is running (always false off Windows).
    pub async fn everything_running(&self) -> bool {
        #[cfg(windows)]
        {
            self.everything
                .run(win::everything::is_running)
                .await
                .unwrap_or(false)
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub async fn icon(&self, source: IconSource, size: u32) -> Result<Image, PlatformError> {
        #[cfg(windows)]
        {
            self.worker
                .run(move || win::icon(&source, size).map_err(os))
                .await?
        }
        #[cfg(not(windows))]
        {
            let _ = (source, size);
            Err(PlatformError::Unsupported)
        }
    }

    /// Opens an app (or runs it as administrator). Each open runs on its own
    /// thread so a slow launch never queues behind icon work.
    pub async fn launch_app(&self, target: String, as_admin: bool) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            win::sta::run_once(move || win::launch_app(&target, as_admin).map_err(os)).await?
        }
        #[cfg(not(windows))]
        {
            let _ = (target, as_admin);
            Err(PlatformError::Unsupported)
        }
    }

    /// Opens a file or folder with its default program.
    pub async fn open_path(&self, path: String) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            win::sta::run_once(move || win::open_path(&path).map_err(os)).await?
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(PlatformError::Unsupported)
        }
    }

    /// The window in front, unless it's the launcher. Instant; call it before
    /// the launcher takes focus.
    pub fn foreground_window(&self) -> Option<ForegroundWindow> {
        #[cfg(windows)]
        {
            win::desktop::foreground_window()
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// The folder and selection of `window` when it's File Explorer or the
    /// desktop (`None` for other windows). Asks Explorer over COM, so it
    /// can take a moment.
    pub async fn explorer_view(
        &self,
        window: ForegroundWindow,
    ) -> Result<Option<ExplorerView>, PlatformError> {
        #[cfg(windows)]
        {
            win::sta::run_once(move || win::desktop::explorer_view(&window).map_err(os)).await?
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            Ok(None)
        }
    }

    /// A screenshot of one window, by handle. It can't include the launcher
    /// even where it overlaps (D-019).
    pub async fn capture_window(&self, handle: isize) -> Result<Image, PlatformError> {
        #[cfg(windows)]
        {
            tokio::task::spawn_blocking(move || win::capture_window(handle).map_err(os))
                .await
                .map_err(|_| PlatformError::WorkerGone)?
        }
        #[cfg(not(windows))]
        {
            let _ = handle;
            Err(PlatformError::Unsupported)
        }
    }

    /// Brings a window to the front (restoring it if minimized).
    pub async fn focus_window(&self, handle: isize) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            tokio::task::spawn_blocking(move || win::input::focus_window(handle).map_err(os))
                .await
                .map_err(|_| PlatformError::WorkerGone)?
        }
        #[cfg(not(windows))]
        {
            let _ = handle;
            Err(PlatformError::Unsupported)
        }
    }

    /// Presses `keys` (see `vk`) as one shortcut, into whatever window is in
    /// front: down in order, up in reverse. Waits briefly for you to let go
    /// of Shift, Ctrl, Alt and Win first, so they don't mix in.
    pub async fn press_keys(&self, keys: Vec<u16>) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            tokio::task::spawn_blocking(move || win::input::press_keys(&keys).map_err(os))
                .await
                .map_err(|_| PlatformError::WorkerGone)?
        }
        #[cfg(not(windows))]
        {
            let _ = keys;
            Err(PlatformError::Unsupported)
        }
    }

    /// Opens the containing folder in Explorer with the item selected.
    pub async fn reveal_path(&self, path: String) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            win::sta::run_once(move || win::reveal(&path).map_err(os)).await?
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(PlatformError::Unsupported)
        }
    }
}

#[cfg(windows)]
fn os(e: windows::core::Error) -> PlatformError {
    PlatformError::Os(e.message())
}
