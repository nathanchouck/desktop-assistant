//! Media controls: Play / Pause, Next and Previous Track press the media
//! keys; "Now Playing" shows the track Windows' media controls report.
//!
//! - `commands.rs`: the rows and their keys.
//! - `now_playing.rs`: the current track, shared with the watcher.
//! - `win.rs` (Windows): pressing keys; a thread that watches the media
//!   sessions, so search never waits on another app.

mod commands;
#[cfg(not(windows))]
mod fallback;
mod now_playing;
mod provider;
#[cfg(windows)]
mod win;

#[cfg(not(windows))]
use fallback as os;
#[cfg(windows)]
use win as os;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tauri::AppHandle;

use super::ProviderContext;
use crate::search::SearchProvider;
pub use commands::MediaKey;
use now_playing::NowPlaying;

/// Between hiding the launcher and pressing a key, so the key reaches
/// Windows rather than the launcher's web view (which keeps media keys
/// for itself while it has focus).
const HIDE_DELAY: Duration = Duration::from_millis(150);

/// Shared by the provider and the watcher `start` runs.
static NOW_PLAYING: OnceLock<Arc<NowPlaying>> = OnceLock::new();

fn now_playing() -> Arc<NowPlaying> {
    Arc::clone(NOW_PLAYING.get_or_init(Default::default))
}

/// Media rows in search.
pub fn provider(_ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(provider::MediaProvider::new(now_playing())))
}

/// Watches what's playing (Windows).
pub fn start(_app: &AppHandle) {
    #[cfg(windows)]
    {
        let now = now_playing();
        let wake = now.connect();
        let started = std::thread::Builder::new()
            .name("now-playing".into())
            .spawn(move || win::watch(now, wake));
        if let Err(e) = started {
            tracing::warn!("now playing is off: {e}");
        }
    }
}

/// Presses `key` once the launcher has hidden (Activation::Hide). Returns
/// at once; a key that can't be pressed is logged.
pub fn press_after_hide(key: MediaKey) -> Result<(), String> {
    if !os::SUPPORTED {
        return Err("Media keys work only on Windows.".into());
    }
    std::thread::Builder::new()
        .name("media-key".into())
        .spawn(move || {
            std::thread::sleep(HIDE_DELAY);
            if let Err(e) = os::press(key.vk()) {
                tracing::warn!("could not press {key:?}: {e}");
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}
