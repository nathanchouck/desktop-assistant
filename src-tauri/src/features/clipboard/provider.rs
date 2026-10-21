//! Clipboard history in search: `clip` (or `clipboard`) lists what you
//! copied, newest first with pinned entries on top; text after it filters.
//! Enter pastes an entry into the window you were in.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use super::content::{self, ClipKind};
use super::history::{thumb_name, Entry, History, Stored};
use super::image;
use super::system::{self, Content};
use crate::fuzzy::FuzzyQuery;
use crate::search::{
    glyph, thumb_icon, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider,
};
use crate::settings::{self, SharedSettings};

const NAME: &str = "clipboard";
/// History entries: `clip:<row id>`.
const ENTRY_PREFIX: &str = "clip:";
/// The provider's commands: `clips:<name>`.
const CLEAR_ID: &str = "clips:clear";
const OPEN_ID: &str = "clips:history";
const SETTINGS_ID: &str = "clips:settings";
const EMPTY_ID: &str = "clips:empty";
/// A filtered entry must match at least this well: long texts contain
/// nearly any letters in order somewhere, so a scattered match means little.
const MIN_FILTER_SCORE: f32 = 0.5;

pub struct ClipboardProvider {
    history: Arc<History>,
    settings: SharedSettings,
}

impl ClipboardProvider {
    pub fn new(history: Arc<History>, settings: SharedSettings) -> Self {
        Self { history, settings }
    }
}

/// The filter text if `query` is meant for clipboard history: "clip" or
/// "clipboard", alone or followed by a space and text.
pub fn history_filter(query: &str) -> Option<&str> {
    ["clipboard", "clip"].iter().find_map(|prefix| {
        let head = query.get(..prefix.len())?;
        if !head.eq_ignore_ascii_case(prefix) {
            return None;
        }
        let rest = &query[prefix.len()..];
        if rest.is_empty() {
            Some("")
        } else if rest.starts_with(char::is_whitespace) {
            Some(rest.trim())
        } else {
            None
        }
    })
}

/// The row id for a history entry, and back.
pub fn entry_id(id: i64) -> String {
    format!("{ENTRY_PREFIX}{id}")
}

pub fn parse_entry_id(id: &str) -> Option<i64> {
    id.strip_prefix(ENTRY_PREFIX)?.parse().ok()
}

/// Rows for the history, in its order, filtered by `filter`. Scores fall
/// from 1 down the list: the ranker keeps clip rows in this order.
pub fn entry_rows(entries: &[Entry], filter: &str, now_ms: i64, limit: usize) -> Vec<ResultItem> {
    let mut fuzzy = (!filter.is_empty()).then(|| FuzzyQuery::new(filter));
    let matching = entries.iter().filter(|e| match fuzzy.as_mut() {
        None => true,
        Some(q) => [&e.title, &e.preview, &e.source]
            .iter()
            .filter_map(|text| q.score(text))
            .any(|score| score >= MIN_FILTER_SCORE),
    });
    matching
        .take(limit)
        .enumerate()
        .map(|(i, e)| entry_row(e, now_ms, 1.0 - i as f32 / (limit as f32 + 1.0)))
        .collect()
}

fn entry_row(entry: &Entry, now_ms: i64, score: f32) -> ResultItem {
    let age = content::age(now_ms, entry.used_ms);
    let subtitle = if entry.source.is_empty() {
        age
    } else {
        format!("{} · {age}", entry.source)
    };
    let icon = match entry.kind {
        ClipKind::Text => glyph("clipboard"),
        ClipKind::Image => thumb_icon(&thumb_name(&entry.hash)),
        ClipKind::Files => glyph("file"),
    };
    let item = ResultItem::new(entry_id(entry.id), ItemKind::Clip, &entry.title)
        .with_subtitle(subtitle)
        .with_icon(icon)
        .with_score(score)
        .with_actions([Action::Paste, Action::Copy, Action::Pin, Action::Delete]);
    if entry.pinned {
        item.with_accessory("Pinned")
    } else {
        item
    }
}

fn clear_row(score: f32) -> ResultItem {
    ResultItem::new(CLEAR_ID, ItemKind::Command, "Clear Clipboard History")
        .with_subtitle("Pinned entries stay")
        .with_icon(glyph("trash"))
        .with_score(score)
        .with_actions([Action::Run])
        .with_confirm("Clear your clipboard history?")
}

fn open_row(score: f32) -> ResultItem {
    ResultItem::new(OPEN_ID, ItemKind::Command, "Clipboard History")
        .with_subtitle("What you copied. Type clip to search it")
        .with_icon(glyph("clipboard"))
        .with_score(score)
        .with_actions([Action::Run])
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn failed(message: impl ToString) -> SearchError {
    SearchError::Provider {
        provider: NAME,
        message: message.to_string(),
    }
}

#[async_trait]
impl SearchProvider for ClipboardProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let Some(filter) = history_filter(query) else {
            // Elsewhere, only the commands show up.
            let mut fuzzy = FuzzyQuery::new(query);
            let rows = [open_row(0.0), clear_row(0.0)]
                .into_iter()
                .filter_map(|row| {
                    let score = fuzzy.score(&row.title)?;
                    Some(row.with_score(score))
                })
                .collect();
            return Ok(rows);
        };
        let settings = settings::current(&self.settings).clipboard;
        if !settings.enabled {
            return Ok(vec![ResultItem::new(
                SETTINGS_ID,
                ItemKind::Command,
                "Clipboard history is off",
            )
            .with_subtitle("Turn it on in Settings")
            .with_icon(glyph("settings"))
            .with_actions([Action::Run])]);
        }
        let now = now_ms();
        let mut rows = self
            .history
            .with_entries(|entries| entry_rows(entries, filter, now, limit));
        if rows.is_empty() && filter.is_empty() {
            rows.push(
                ResultItem::new(EMPTY_ID, ItemKind::Command, "Nothing copied yet")
                    .with_subtitle("What you copy shows up here")
                    .with_icon(glyph("clipboard"))
                    .with_actions([Action::Run]),
            );
        }
        if !filter.is_empty() {
            if let Some(score) = FuzzyQuery::new(filter).score("Clear Clipboard History") {
                rows.push(clear_row(score));
            }
        }
        Ok(rows)
    }

    fn claims(&self, query: &str) -> bool {
        history_filter(query).is_some()
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(ENTRY_PREFIX) || id.starts_with("clips:")
    }

    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError> {
        match id {
            CLEAR_ID => {
                let history = Arc::clone(&self.history);
                let removed = tokio::task::spawn_blocking(move || history.clear())
                    .await
                    .map_err(failed)?
                    .map_err(failed)?;
                tracing::info!("cleared {removed} clipboard history entries");
                return Ok(Activation::SetQuery {
                    query: "clip ".into(),
                });
            }
            OPEN_ID => {
                return Ok(Activation::SetQuery {
                    query: "clip ".into(),
                })
            }
            SETTINGS_ID => return Ok(Activation::OpenSettings),
            EMPTY_ID => {
                return Ok(Activation::Notice {
                    message: "Copy something and it shows up here.".into(),
                })
            }
            _ => {}
        }
        let entry = parse_entry_id(id).ok_or_else(|| failed(format!("{id} can't be opened")))?;
        let history = Arc::clone(&self.history);
        match action {
            Action::Pin => {
                tokio::task::spawn_blocking(move || history.toggle_pin(entry))
                    .await
                    .map_err(failed)?
                    .map_err(failed)?;
                Ok(Activation::Refresh)
            }
            Action::Delete => {
                tokio::task::spawn_blocking(move || history.delete(entry))
                    .await
                    .map_err(failed)?
                    .map_err(failed)?;
                Ok(Activation::Refresh)
            }
            Action::Copy | Action::Paste => {
                // Reading an image file and putting it on the clipboard can
                // take a moment.
                tokio::task::spawn_blocking(move || put_back(&history, entry))
                    .await
                    .map_err(failed)??;
                Ok(if action == Action::Paste {
                    Activation::Paste { text: None }
                } else {
                    Activation::Hide
                })
            }
            _ => Err(failed(format!("{id} can't do that"))),
        }
    }
}

/// Puts an entry back on the clipboard and moves it to the top.
fn put_back(history: &History, id: i64) -> Result<(), SearchError> {
    let stored = history
        .stored(id)
        .map_err(failed)?
        .ok_or_else(|| failed("that entry is gone"))?;
    let content = match stored {
        Stored::Text(text) => Content::Text(text),
        Stored::Files(paths) => Content::Files(paths),
        Stored::Image(path) => {
            let bytes = std::fs::read(&path).map_err(|e| failed(format!("image missing: {e}")))?;
            Content::Image(image::decode_png(&bytes).map_err(failed)?)
        }
    };
    system::write(&content, false).map_err(|e| failed(format!("couldn't copy: {e}")))?;
    history.bump(id, now_ms()).map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::clipboard::history::NewClip;
    use crate::settings::Settings;
    use std::sync::RwLock;

    #[test]
    fn claims_clip_and_clipboard_prefixes() {
        assert_eq!(history_filter("clip"), Some(""));
        assert_eq!(history_filter("Clipboard"), Some(""));
        assert_eq!(history_filter("clip  hello world "), Some("hello world"));
        assert_eq!(history_filter("clipboard foo"), Some("foo"));
        assert_eq!(history_filter("clipchamp"), None);
        assert_eq!(history_filter("clipb"), None);
        assert_eq!(history_filter("cli"), None);
        assert_eq!(history_filter("my clip"), None);
        assert_eq!(history_filter("clipé"), None);
    }

    #[test]
    fn entry_ids_round_trip() {
        assert_eq!(parse_entry_id(&entry_id(42)), Some(42));
        assert_eq!(parse_entry_id("clip:x"), None);
        assert_eq!(parse_entry_id(CLEAR_ID), None);
    }

    fn entry(id: i64, title: &str, preview: &str, pinned: bool) -> Entry {
        Entry {
            id,
            kind: ClipKind::Text,
            title: title.into(),
            preview: preview.into(),
            hash: format!("{id:016x}"),
            source: "Code".into(),
            used_ms: 0,
            pinned,
        }
    }

    #[test]
    fn rows_keep_history_order_and_filter() {
        let entries = [
            entry(3, "pinned thing", "pinned thing", true),
            entry(
                2,
                "fn main() {",
                "fn main() {\n  println!(\"hello\");\n}",
                false,
            ),
            entry(1, "grocery list", "grocery list\nmilk\neggs", false),
        ];
        let rows = entry_rows(&entries, "", 120_000, 10);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["clip:3", "clip:2", "clip:1"]);
        assert!(rows.windows(2).all(|w| w[0].score > w[1].score));
        assert_eq!(rows[0].accessory.as_deref(), Some("Pinned"));
        assert_eq!(rows[1].accessory, None);
        assert_eq!(rows[1].subtitle, "Code · 2 min ago");
        assert_eq!(rows[1].actions[0], Action::Paste);

        let ids = |filter| -> Vec<String> {
            entry_rows(&entries, filter, 0, 10)
                .into_iter()
                .map(|r| r.id)
                .collect()
        };
        // Matches inside the text, not just the title.
        assert_eq!(ids("println"), ["clip:2"]);
        assert_eq!(ids("milk"), ["clip:1"]);
        assert!(ids("zzz").is_empty());
        assert_eq!(entry_rows(&entries, "", 0, 2).len(), 2);
    }

    #[test]
    fn scattered_letters_in_long_text_dont_match() {
        let text = "The quick brown fox jumps over the lazy dog while the band plays \
                    a merry tune about kettles, ropes and seven silver spoons.";
        let entries = [entry(1, "The quick brown fox", text, false)];
        assert!(entry_rows(&entries, "kettle", 0, 10).len() == 1);
        assert!(entry_rows(&entries, "tqbf", 0, 10).len() == 1);
        assert!(entry_rows(&entries, "password", 0, 10).is_empty());
    }

    fn provider() -> (tempfile::TempDir, ClipboardProvider) {
        let dir = tempfile::tempdir().unwrap();
        let history = History::in_memory(dir.path().join("i"), dir.path().join("t")).unwrap();
        let settings = Arc::new(RwLock::new(Settings::default()));
        (dir, ClipboardProvider::new(Arc::new(history), settings))
    }

    #[tokio::test]
    async fn lists_history_and_offers_commands() {
        let (_dir, p) = provider();
        assert!(p.claims("clip"));
        let rows = p.query("clip", 32).await.unwrap();
        assert_eq!(rows[0].id, EMPTY_ID);
        p.history
            .add(
                &NewClip {
                    kind: ClipKind::Text,
                    content: "hello".into(),
                    hash: content::text_hash("hello"),
                    width: 0,
                    height: 0,
                    source: String::new(),
                },
                now_ms(),
                10,
            )
            .unwrap();
        let rows = p.query("clip", 32).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "hello");
        assert_eq!(rows[0].subtitle, "just now");
        let id = rows[0].id.clone();
        assert!(p.owns(&id));

        // "clip clear" offers the clear command too; general search finds both commands.
        let rows = p.query("clip clear", 32).await.unwrap();
        assert!(rows.iter().any(|r| r.id == CLEAR_ID && r.confirm.is_some()));
        let rows = p.query("clipboard zzz", 32).await.unwrap();
        assert!(rows.is_empty());
        let rows = p.query("clipb", 32).await.unwrap();
        assert!(rows.iter().any(|r| r.id == OPEN_ID));

        // Pin and delete refresh the list.
        assert_eq!(
            p.activate(&id, Action::Pin).await.unwrap(),
            Activation::Refresh
        );
        assert!(p.history.with_entries(|e| e[0].pinned));
        assert_eq!(
            p.activate(&id, Action::Delete).await.unwrap(),
            Activation::Refresh
        );
        assert!(p.history.is_empty());
        assert!(p.activate("clip:999", Action::Paste).await.is_err());
        assert_eq!(
            p.activate(OPEN_ID, Action::Run).await.unwrap(),
            Activation::SetQuery {
                query: "clip ".into()
            }
        );
    }

    #[tokio::test]
    async fn says_when_history_is_off() {
        let (_dir, p) = provider();
        p.settings.write().unwrap().clipboard.enabled = false;
        let rows = p.query("clip", 32).await.unwrap();
        assert_eq!(rows[0].id, SETTINGS_ID);
        assert_eq!(
            p.activate(SETTINGS_ID, Action::Run).await.unwrap(),
            Activation::OpenSettings
        );
    }
}
