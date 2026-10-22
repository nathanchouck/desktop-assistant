//! Media rows: Play / Pause (with what's playing), Next and Previous
//! Track, and Now Playing (Enter copies "Title — Artist").

use std::sync::Arc;

use async_trait::async_trait;

use super::commands::MediaCommand;
use super::now_playing::{NowPlaying, Track};
use crate::features::system::matching;
use crate::fuzzy::FuzzyQuery;
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};

const NAME: &str = "media";

pub struct MediaProvider {
    now: Arc<NowPlaying>,
}

impl MediaProvider {
    pub fn new(now: Arc<NowPlaying>) -> Self {
        Self { now }
    }
}

/// Rows for `query`, given what's playing.
pub fn rows(query: &str, track: Option<&Track>) -> Vec<ResultItem> {
    if !matching::long_enough(query) {
        return Vec::new();
    }
    let mut fuzzy = FuzzyQuery::new(query);
    MediaCommand::ALL
        .into_iter()
        .filter_map(|cmd| {
            let score = matching::score(&mut fuzzy, cmd.title(), cmd.keywords())?;
            row(cmd, track).map(|r| r.with_score(score))
        })
        .collect()
}

fn row(cmd: MediaCommand, track: Option<&Track>) -> Option<ResultItem> {
    let item = match (cmd, track) {
        (MediaCommand::NowPlaying, None) => return None,
        (MediaCommand::NowPlaying, Some(track)) => {
            ResultItem::new(cmd.id(), ItemKind::Command, track.text())
                .with_subtitle(track.status())
                .with_actions([Action::Copy])
        }
        (MediaCommand::PlayPause, Some(track)) => {
            ResultItem::new(cmd.id(), ItemKind::Command, cmd.title())
                .with_subtitle(format!("{} · {}", track.text(), track.status()))
                .with_actions([Action::Run])
        }
        _ => ResultItem::new(cmd.id(), ItemKind::Command, cmd.title())
            .with_subtitle("Media key")
            .with_actions([Action::Run]),
    };
    Some(item.with_icon(glyph(cmd.glyph())))
}

#[async_trait]
impl SearchProvider for MediaProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn query(&self, query: &str, _limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        if !matching::long_enough(query) {
            return Ok(Vec::new());
        }
        Ok(rows(query, self.now.current().as_ref()))
    }

    fn owns(&self, id: &str) -> bool {
        MediaCommand::from_id(id).is_some()
    }

    async fn activate(&self, id: &str, _action: Action) -> Result<Activation, SearchError> {
        let cmd = MediaCommand::from_id(id).ok_or_else(|| SearchError::Provider {
            provider: NAME,
            message: format!("{id} can't be opened"),
        })?;
        let Some(key) = cmd.key() else {
            return Ok(match self.now.current() {
                Some(track) => Activation::Copy { text: track.text() },
                None => Activation::Notice {
                    message: "Nothing is playing.".into(),
                },
            });
        };
        Ok(match super::press_after_hide(key) {
            Ok(()) => Activation::Hide,
            Err(message) => Activation::Notice { message },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Track {
        Track {
            title: "Bohemian Rhapsody".into(),
            artist: "Queen".into(),
            app: "Spotify.exe".into(),
            playing: true,
        }
    }

    fn ids(rows: &[ResultItem]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn finds_media_commands() {
        assert_eq!(ids(&rows("pause", None)), ["media:play-pause"]);
        assert_eq!(ids(&rows("next track", None)), ["media:next"]);
        assert_eq!(ids(&rows("skip", None)), ["media:next"]);
        assert_eq!(ids(&rows("previous", None)), ["media:previous"]);
        assert!(rows("p", None).is_empty());
        assert!(rows("excel", None).is_empty());
    }

    #[test]
    fn shows_whats_playing() {
        let t = track();
        let found = rows("now playing", Some(&t));
        let now = found.iter().find(|r| r.id == "media:now-playing").unwrap();
        assert_eq!(now.title, "Bohemian Rhapsody — Queen");
        assert_eq!(now.subtitle, "Playing · Spotify");
        assert_eq!(now.actions, [Action::Copy]);
        // Nothing playing: no Now Playing row.
        assert!(!ids(&rows("now playing", None)).contains(&"media:now-playing"));
        let play = &rows("play pause", Some(&t))[0];
        assert_eq!(
            play.subtitle,
            "Bohemian Rhapsody — Queen · Playing · Spotify"
        );
    }

    #[tokio::test]
    async fn now_playing_copies_the_track() {
        let now = Arc::new(NowPlaying::default());
        let provider = MediaProvider::new(Arc::clone(&now));
        assert!(provider.owns("media:now-playing"));
        assert!(!provider.owns("sys:mute"));
        assert_eq!(
            provider
                .activate("media:now-playing", Action::Copy)
                .await
                .unwrap(),
            Activation::Notice {
                message: "Nothing is playing.".into()
            }
        );
        now.set(Some(track()));
        assert_eq!(
            provider
                .activate("media:now-playing", Action::Copy)
                .await
                .unwrap(),
            Activation::Copy {
                text: "Bohemian Rhapsody — Queen".into()
            }
        );
    }
}
