//! What's playing: the track Windows' media controls show (the volume
//! flyout's player), kept fresh by a background thread so search only
//! reads memory.

use std::sync::mpsc;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

/// Older than this, a search asks for a fresh look (the answer is there by
/// the next keystroke).
const STALE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    /// The player's AppUserModelID ("Spotify.exe", "MSEdge", ...).
    pub app: String,
    pub playing: bool,
}

impl Track {
    /// "Title — Artist", or just the title.
    pub fn text(&self) -> String {
        if self.artist.is_empty() {
            self.title.clone()
        } else {
            format!("{} — {}", self.title, self.artist)
        }
    }

    /// "Playing · Spotify" / "Paused · Spotify".
    pub fn status(&self) -> String {
        let state = if self.playing { "Playing" } else { "Paused" };
        let app = app_name(&self.app);
        if app.is_empty() {
            state.to_owned()
        } else {
            format!("{state} · {app}")
        }
    }
}

/// A readable player name from its AppUserModelID: a Store app's
/// (`SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`), an exe name, a path,
/// or a browser's own id.
pub fn app_name(aumid: &str) -> String {
    let last = aumid.rsplit('!').next().unwrap_or(aumid);
    let last = last.rsplit(['\\', '/']).next().unwrap_or(last);
    let stem = match last.len().checked_sub(4) {
        Some(cut) if last.is_char_boundary(cut) && last[cut..].eq_ignore_ascii_case(".exe") => {
            &last[..cut]
        }
        _ => last,
    };
    // Package-style names: the last part ("Microsoft.ZuneMusic").
    let short = stem.rsplit('.').next().unwrap_or(stem);
    let known = match short.to_ascii_lowercase().as_str() {
        "chrome" => Some("Chrome"),
        "msedge" => Some("Edge"),
        // Firefox registers under a hash of its install folder.
        "308046b0af4a39cb" | "firefox" => Some("Firefox"),
        "zunemusic" => Some("Media Player"),
        "zunevideo" => Some("Movies & TV"),
        "spotify" | "spotifymusic" => Some("Spotify"),
        "vlc" => Some("VLC"),
        "applemusic" | "applemusicwin11" => Some("Apple Music"),
        _ => None,
    };
    if let Some(name) = known {
        return name.to_owned();
    }
    let mut chars = short.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The latest track, shared by the watcher and search.
#[derive(Default)]
pub struct NowPlaying {
    track: RwLock<Option<Track>>,
    updated: Mutex<Option<Instant>>,
    /// Wakes the watcher early.
    wake: Mutex<Option<mpsc::Sender<()>>>,
}

impl NowPlaying {
    /// What's playing, as last seen. Asks the watcher to look again when
    /// that was a while ago.
    pub fn current(&self) -> Option<Track> {
        let stale = self
            .updated
            .lock()
            .ok()
            .and_then(|t| *t)
            .is_none_or(|t| t.elapsed() > STALE);
        if stale {
            if let Ok(wake) = self.wake.lock() {
                if let Some(wake) = wake.as_ref() {
                    let _ = wake.send(());
                }
            }
        }
        match self.track.read() {
            Ok(track) => track.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn set(&self, track: Option<Track>) {
        if let Ok(mut current) = self.track.write() {
            *current = track;
        }
        if let Ok(mut updated) = self.updated.lock() {
            *updated = Some(Instant::now());
        }
    }

    /// Connects the watcher; returns what it waits on between looks.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn connect(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut wake) = self.wake.lock() {
            *wake = Some(tx);
        }
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(artist: &str, playing: bool) -> Track {
        Track {
            title: "Bohemian Rhapsody".into(),
            artist: artist.into(),
            app: "Spotify.exe".into(),
            playing,
        }
    }

    #[test]
    fn describes_the_track() {
        assert_eq!(track("Queen", true).text(), "Bohemian Rhapsody — Queen");
        assert_eq!(track("", true).text(), "Bohemian Rhapsody");
        assert_eq!(track("Queen", true).status(), "Playing · Spotify");
        assert_eq!(track("Queen", false).status(), "Paused · Spotify");
    }

    #[test]
    fn player_names() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(
            app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "Media Player"
        );
        assert_eq!(app_name("MSEdge"), "Edge");
        assert_eq!(app_name("Chrome"), "Chrome");
        assert_eq!(app_name("308046B0AF4A39CB"), "Firefox");
        assert_eq!(app_name(r"C:\Program Files\VideoLAN\VLC\vlc.exe"), "VLC");
        assert_eq!(app_name("foobar2000.exe"), "Foobar2000");
        assert_eq!(app_name(""), "");
        assert_eq!(app_name("é.exe"), "É");
    }

    #[test]
    fn a_stale_track_asks_for_a_look() {
        let now = NowPlaying::default();
        let wake = now.connect();
        assert_eq!(now.current(), None);
        assert!(wake.try_recv().is_ok(), "never looked: asks");
        now.set(Some(track("Queen", true)));
        assert_eq!(now.current().unwrap().artist, "Queen");
        assert!(wake.try_recv().is_err(), "fresh: doesn't ask");
    }
}
