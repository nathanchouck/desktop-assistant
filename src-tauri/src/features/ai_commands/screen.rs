//! "Ask About Screen Region": the launcher hides, Windows' snipping
//! overlay opens (`ms-screenclip:`, as Win+Shift+S does), and the picture
//! you snip arrives on the clipboard. The launcher then comes back with it
//! attached. The launcher is hidden before the overlay opens, so it's
//! never in the shot.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use super::images;
use crate::features::clipboard::system;
use crate::state::AppState;
use crate::{toast, window};

/// The snipping overlay, through its protocol.
const SNIP_URI: &str = "ms-screenclip:";
/// The launcher's hide has to reach the screen before the overlay freezes it.
const HIDE_DELAY: Duration = Duration::from_millis(250);
/// How long a snip may take before we stop waiting for it.
const SNIP_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(150);

/// One snip at a time.
static SNIPPING: AtomicBool = AtomicBool::new(false);

/// Starts a snip in the background; the launcher hides meanwhile
/// (`Activation::Hide`).
pub fn start(app: &AppHandle) {
    if SNIPPING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        snip(&app).await;
        SNIPPING.store(false, Ordering::SeqCst);
    });
}

async fn snip(app: &AppHandle) {
    tokio::time::sleep(HIDE_DELAY).await;
    let before = blocking(system::sequence).await.unwrap_or(0);
    if let Err(e) = app.opener().open_url(SNIP_URI, None::<&str>) {
        toast::error(app, format!("Couldn't open the snipping overlay: {e}"));
        return;
    }
    let picture = blocking(move || wait_for_picture(before)).await.flatten();
    let Some(image) = picture else {
        tracing::info!("no screen region was snipped within {SNIP_TIMEOUT:?}");
        return;
    };
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let dir = state.paths.data_dir.join(images::DIR_NAME);
    let thumbs = state.paths.thumbnails_dir();
    let saved = blocking(move || {
        images::sweep(&dir, &thumbs, images::KEEP_FOR);
        images::save(image, &dir, &thumbs)
    })
    .await;
    window::show(app, Instant::now());
    match saved {
        Some(Ok(attachment)) => super::compose(app, vec![attachment]),
        Some(Err(e)) => toast::error(app, format!("Couldn't keep the snip: {e}")),
        None => {}
    }
}

/// Waits for a picture to be copied (the snip). Other copies made
/// meanwhile are skipped.
fn wait_for_picture(mut before: u32) -> Option<crate::platform::Image> {
    let started = Instant::now();
    while started.elapsed() < SNIP_TIMEOUT {
        std::thread::sleep(POLL);
        let now = system::sequence();
        if now == before {
            continue;
        }
        before = now;
        if let Some(image) = system::read().as_ref().and_then(images::from_clipboard) {
            return Some(image);
        }
    }
    None
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}
