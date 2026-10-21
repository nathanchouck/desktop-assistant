//! The media rows: names, keywords, and which key each presses.

/// A key on a media keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKey {
    PlayPause,
    Next,
    Previous,
    Mute,
}

impl MediaKey {
    /// Its Windows virtual-key code.
    pub fn vk(self) -> u16 {
        match self {
            Self::PlayPause => 0xB3,
            Self::Next => 0xB0,
            Self::Previous => 0xB1,
            Self::Mute => 0xAD,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    PlayPause,
    Next,
    Previous,
    NowPlaying,
}

impl MediaCommand {
    pub const ALL: [Self; 4] = [Self::PlayPause, Self::Next, Self::Previous, Self::NowPlaying];

    pub fn id(self) -> &'static str {
        match self {
            Self::PlayPause => "media:play-pause",
            Self::Next => "media:next",
            Self::Previous => "media:previous",
            Self::NowPlaying => "media:now-playing",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    /// The name it's found by (the Now Playing row shows the track instead).
    pub fn title(self) -> &'static str {
        match self {
            Self::PlayPause => "Play / Pause",
            Self::Next => "Next Track",
            Self::Previous => "Previous Track",
            Self::NowPlaying => "Now Playing",
        }
    }

    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::PlayPause => &["play", "pause", "resume", "music", "media", "stop music"],
            Self::Next => &["skip", "next song", "skip song", "forward"],
            Self::Previous => &["previous song", "prev", "back", "last song", "rewind"],
            Self::NowPlaying => &["playing", "current song", "song", "track", "whats playing"],
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::PlayPause => "play-pause",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::NowPlaying => "music",
        }
    }

    /// The key it presses (Now Playing presses none).
    pub fn key(self) -> Option<MediaKey> {
        match self {
            Self::PlayPause => Some(MediaKey::PlayPause),
            Self::Next => Some(MediaKey::Next),
            Self::Previous => Some(MediaKey::Previous),
            Self::NowPlaying => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for cmd in MediaCommand::ALL {
            assert_eq!(MediaCommand::from_id(cmd.id()), Some(cmd));
        }
        assert_eq!(MediaCommand::from_id("media:nope"), None);
    }

    #[test]
    fn keys_are_the_media_keys() {
        assert_eq!(MediaCommand::PlayPause.key().map(MediaKey::vk), Some(0xB3));
        assert_eq!(MediaCommand::Next.key().map(MediaKey::vk), Some(0xB0));
        assert_eq!(MediaCommand::Previous.key().map(MediaKey::vk), Some(0xB1));
        assert_eq!(MediaCommand::NowPlaying.key(), None);
        assert_eq!(MediaKey::Mute.vk(), 0xAD);
    }
}
