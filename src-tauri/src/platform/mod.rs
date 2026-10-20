//! OS-specific work behind one small interface: listing installed apps,
//! getting their icons, and launching them. Windows does the real work
//! (`win/`); other platforms get stubs so the app builds and tests anywhere.

#[cfg(windows)]
mod win;

/// An app as the OS reports it, before filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawApp {
    /// Windows: the Start menu's "parsing name" (an AppUserModelID such as
    /// `Microsoft.WindowsCalculator_8wekyb3d8bbwe!App`, or a path).
    pub target: String,
    pub name: String,
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
}

/// Entry point for shell work. Cheap to share behind an `Arc`.
pub struct Shell {
    #[cfg(windows)]
    worker: win::sta::StaWorker,
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

    pub async fn app_icon(&self, target: String, size: u32) -> Result<Image, PlatformError> {
        #[cfg(windows)]
        {
            self.worker
                .run(move || win::app_icon(&target, size).map_err(os))
                .await?
        }
        #[cfg(not(windows))]
        {
            let _ = (target, size);
            Err(PlatformError::Unsupported)
        }
    }

    /// Opens an app (or runs it as administrator). Runs on its own thread so a
    /// slow launch never queues behind icon work.
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
}

#[cfg(windows)]
fn os(e: windows::core::Error) -> PlatformError {
    PlatformError::Os(e.message())
}
