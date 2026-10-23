//! Windows: pressing media keys, and watching what's playing through the
//! system media controls (`GlobalSystemMediaTransportControlsSessionManager`,
//! the same source as the volume flyout's player).

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    VIRTUAL_KEY,
};

use super::now_playing::{NowPlaying, Track};
use crate::platform::INJECTED_TAG;

pub const SUPPORTED: bool = true;

/// Between looks when nobody asks sooner, while the launcher is showing.
/// While it's hidden there's no periodic look (a cross-process call): a
/// search asks for a fresh one when what it has is a few seconds old.
const POLL: Duration = Duration::from_secs(5);
/// Before trying again when Windows' media controls aren't available.
const RETRY: Duration = Duration::from_secs(60);

/// Presses and releases a media key, as a media keyboard sends it.
pub fn press(vk: u16) -> Result<(), String> {
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                // Media keys are extended keys on real keyboards.
                dwFlags: if up {
                    KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
                } else {
                    KEYEVENTF_EXTENDEDKEY
                },
                time: 0,
                dwExtraInfo: INJECTED_TAG,
            },
        },
    };
    let inputs = [key(false), key(true)];
    // SAFETY: `inputs` is a valid slice for the duration of the call.
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(windows::core::Error::from_thread().message())
    }
}

/// Keeps `now` up to date until the app exits: looks every few seconds,
/// or sooner when search asks (`wake`).
pub fn watch(now: Arc<NowPlaying>, wake: Receiver<()>) {
    // SAFETY: this thread's own apartment; the media controls are agile.
    let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if joined.is_err() {
        tracing::warn!("now playing is off: {}", joined.message());
        return;
    }
    let mut manager: Option<SessionManager> = None;
    loop {
        if manager.is_none() {
            match SessionManager::RequestAsync().and_then(|op| op.join()) {
                Ok(m) => manager = Some(m),
                Err(e) => {
                    tracing::debug!("media controls unavailable: {}", e.message());
                    if wait(&wake, RETRY).is_err() {
                        return;
                    }
                    continue;
                }
            }
        }
        let track = manager.as_ref().and_then(read);
        now.set(track);
        let next = if crate::window::is_shown() {
            wait(&wake, POLL)
        } else {
            wait_for_search(&wake)
        };
        if next.is_err() {
            return;
        }
    }
}

/// Waits for a wake-up or the timeout; `Err` once nobody can wake us.
fn wait(wake: &Receiver<()>, timeout: Duration) -> Result<(), ()> {
    match wake.recv_timeout(timeout) {
        Ok(()) | Err(RecvTimeoutError::Timeout) => {
            drain(wake);
            Ok(())
        }
        Err(RecvTimeoutError::Disconnected) => Err(()),
    }
}

/// Waits until a search asks for a look.
fn wait_for_search(wake: &Receiver<()>) -> Result<(), ()> {
    wake.recv().map_err(|_| ())?;
    drain(wake);
    Ok(())
}

/// Several searches may have asked at once: one look is enough.
fn drain(wake: &Receiver<()>) {
    while wake.try_recv().is_ok() {}
}

/// The current session's track, if one has a title.
fn read(manager: &SessionManager) -> Option<Track> {
    // No session (nothing playing) comes back as an error.
    let session = manager.GetCurrentSession().ok()?;
    let props = session
        .TryGetMediaPropertiesAsync()
        .and_then(|op| op.join())
        .map_err(|e| tracing::debug!("could not read the track: {}", e.message()))
        .ok()?;
    let title = props.Title().ok()?.to_string_lossy();
    if title.trim().is_empty() {
        return None;
    }
    let artist = props
        .Artist()
        .map(|a| a.to_string_lossy())
        .unwrap_or_default();
    let app = session
        .SourceAppUserModelId()
        .map(|a| a.to_string_lossy())
        .unwrap_or_default();
    let playing = session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .is_ok_and(|status| status == PlaybackStatus::Playing);
    Some(Track {
        title,
        artist,
        app,
        playing,
    })
}
