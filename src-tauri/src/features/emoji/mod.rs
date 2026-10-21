//! Emoji and symbol picker: `:fire` or `emoji fire` lists matches, recently
//! used first; Enter pastes into the window you were in, Ctrl+Shift+C
//! copies. Without a prefix, a query that is exactly an emoji's name
//! ("thumbs up", "degree") gets one low-ranked suggestion.

mod index;
mod symbols;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::ProviderContext;
use crate::search::{
    text_icon, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider,
};
use crate::settings::SharedSettings;
use crate::store::Store;
use index::{Entry, Index};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct EmojiSettings {
    pub enabled: bool,
    /// For people and hands.
    pub skin_tone: SkinTone,
}

impl Default for EmojiSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            skin_tone: SkinTone::None,
        }
    }
}

/// Skin tone for emoji that have them. `None` is the default yellow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SkinTone {
    None,
    Light,
    MediumLight,
    Medium,
    MediumDark,
    Dark,
}

impl SkinTone {
    fn modifier(self) -> Option<emojis::SkinTone> {
        Some(match self {
            Self::None => return None,
            Self::Light => emojis::SkinTone::Light,
            Self::MediumLight => emojis::SkinTone::MediumLight,
            Self::Medium => emojis::SkinTone::Medium,
            Self::MediumDark => emojis::SkinTone::MediumDark,
            Self::Dark => emojis::SkinTone::Dark,
        })
    }
}

const ID_PREFIX: &str = "emoji:";
/// The suggestion without a prefix sits below anything that matched.
const SUGGESTION_SCORE: f32 = 0.05;
/// Shortest query that gets a suggestion without a prefix.
const SUGGESTION_MIN_CHARS: usize = 3;
/// Emoticons (":)") come before name matches.
const EMOTICON_SCORE: f32 = 1.1;

/// This feature's search results. Builds the emoji index (a few ms).
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(EmojiProvider::new(
        Arc::clone(&ctx.settings),
        Arc::clone(&ctx.store),
    )))
}

/// Nothing runs in the background.
pub fn start(_app: &AppHandle) {}

pub struct EmojiProvider {
    index: Index,
    settings: SharedSettings,
    /// How often each emoji was picked (recorded by `actions`).
    store: Arc<Store>,
}

/// The search term after ":" or "emoji ", if the query starts with one.
fn term(query: &str) -> Option<&str> {
    if let Some(rest) = query.strip_prefix(':') {
        return Some(rest.trim());
    }
    let (head, rest) = (query.get(..5)?, query.get(5..)?);
    let prefixed = head.eq_ignore_ascii_case("emoji") && (rest.is_empty() || rest.starts_with(char::is_whitespace));
    prefixed.then(|| rest.trim())
}

fn title(entry: &Entry) -> String {
    let mut chars = entry.name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl EmojiProvider {
    fn new(settings: SharedSettings, store: Arc<Store>) -> Self {
        Self {
            index: Index::build(),
            settings,
            store,
        }
    }

    fn current(&self) -> EmojiSettings {
        match self.settings.read() {
            Ok(s) => s.emoji.clone(),
            Err(poisoned) => poisoned.into_inner().emoji.clone(),
        }
    }

    /// The text for `glyph` in the chosen skin tone.
    fn toned(&self, glyph: &str, tone: SkinTone) -> String {
        let toned = self
            .index
            .by_glyph(glyph)
            .and_then(|i| self.index.get(i).emoji)
            .zip(tone.modifier())
            .and_then(|(emoji, modifier)| emoji.with_skin_tone(modifier));
        toned.map_or_else(|| glyph.to_owned(), |e| e.as_str().to_owned())
    }

    fn item(&self, i: usize, score: f32, tone: SkinTone) -> ResultItem {
        let entry = self.index.get(i);
        let mut item = ResultItem::new(format!("{ID_PREFIX}{}", entry.glyph), ItemKind::Emoji, title(entry))
            .with_icon(text_icon(&self.toned(entry.glyph, tone)))
            .with_score(score)
            .with_actions([Action::Paste, Action::Copy]);
        if let Some(accessory) = entry.accessory() {
            item = item.with_accessory(accessory);
        }
        item
    }

    /// ":" alone: what you picked recently, then popular ones.
    fn recent_and_popular(&self, limit: usize) -> Vec<(usize, f32)> {
        let mut recent: Vec<(usize, f64)> = (0..self.index.len())
            .filter_map(|i| {
                let used = self.store.frecency(&format!("{ID_PREFIX}{}", self.index.get(i).glyph));
                (used > 0.0).then_some((i, used))
            })
            .collect();
        recent.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut picked: Vec<usize> = recent.into_iter().map(|(i, _)| i).collect();
        for glyph in symbols::POPULAR {
            let i = emojis::get(glyph)
                .and_then(|e| self.index.by_glyph(e.as_str()))
                .or_else(|| self.index.by_glyph(glyph));
            if let Some(i) = i.filter(|i| !picked.contains(i)) {
                picked.push(i);
            }
        }
        picked.truncate(limit);
        // Scores that keep this order through ranking.
        picked
            .into_iter()
            .enumerate()
            .map(|(n, i)| (i, 1.0 - n as f32 * 0.01))
            .collect()
    }

    fn emoticon(&self, term: &str) -> Option<usize> {
        let (_, glyph) = symbols::EMOTICONS.iter().find(|(text, _)| *text == term)?;
        self.index.by_glyph(emojis::get(glyph)?.as_str())
    }
}

#[async_trait]
impl SearchProvider for EmojiProvider {
    fn name(&self) -> &'static str {
        "emoji"
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let settings = self.current();
        if !settings.enabled {
            return Ok(Vec::new());
        }
        let tone = settings.skin_tone;
        let Some(term) = term(query) else {
            // No prefix: one suggestion, only for an emoji's exact name.
            if query.chars().count() < SUGGESTION_MIN_CHARS {
                return Ok(Vec::new());
            }
            let found = self.index.by_name(&index::normalize(query));
            return Ok(found
                .map(|i| self.item(i, SUGGESTION_SCORE, tone))
                .into_iter()
                .collect());
        };
        let mut found = Vec::new();
        if let Some(i) = self.emoticon(term) {
            found.push((i, EMOTICON_SCORE));
        }
        if index::normalize(term).is_empty() {
            found.extend(self.recent_and_popular(limit));
        } else {
            found.extend(self.index.search(term, limit));
        }
        let mut seen = Vec::with_capacity(found.len());
        Ok(found
            .into_iter()
            .filter(|(i, _)| {
                let new = !seen.contains(i);
                seen.push(*i);
                new
            })
            .take(limit)
            .map(|(i, score)| self.item(i, score, tone))
            .collect())
    }

    fn claims(&self, query: &str) -> bool {
        let enabled = match self.settings.read() {
            Ok(s) => s.emoji.enabled,
            Err(poisoned) => poisoned.into_inner().emoji.enabled,
        };
        enabled && term(query).is_some()
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(ID_PREFIX)
    }

    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError> {
        let glyph = id.strip_prefix(ID_PREFIX).unwrap_or_default();
        if glyph.is_empty() {
            return Err(SearchError::Provider {
                provider: self.name(),
                message: format!("{id} isn't an emoji"),
            });
        }
        let text = self.toned(glyph, self.current().skin_tone);
        Ok(match action {
            Action::Copy => Activation::Copy { text },
            _ => Activation::Paste { text: Some(text) },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::RwLock;

    fn provider(settings: Settings) -> EmojiProvider {
        EmojiProvider::new(Arc::new(RwLock::new(settings)), Arc::new(Store::in_memory()))
    }

    async fn icons(p: &EmojiProvider, query: &str, n: usize) -> Vec<String> {
        p.query(query, 32)
            .await
            .unwrap()
            .into_iter()
            .take(n)
            .filter_map(|i| i.icon)
            .collect()
    }

    #[test]
    fn prefixes() {
        assert_eq!(term(":fire"), Some("fire"));
        assert_eq!(term(":"), Some(""));
        assert_eq!(term("emoji thumbs up"), Some("thumbs up"));
        assert_eq!(term("Emoji"), Some(""));
        assert_eq!(term("emojis"), None);
        assert_eq!(term("fire"), None);
        assert_eq!(term("é"), None);
    }

    #[tokio::test]
    async fn prefixed_search_lists_matches() {
        let p = provider(Settings::default());
        assert!(p.claims(":fire"));
        assert!(p.claims("emoji fire"));
        assert!(!p.claims("fire"));
        let items = p.query(":fire", 32).await.unwrap();
        let first = &items[0];
        assert_eq!(first.title, "Fire");
        assert_eq!(first.icon.as_deref(), Some("text:🔥"));
        assert_eq!(first.accessory.as_deref(), Some(":fire:"));
        assert_eq!(first.kind, ItemKind::Emoji);
        assert_eq!(first.actions, [Action::Paste, Action::Copy]);
        assert!(items.len() > 1);
        assert_eq!(icons(&p, ":)", 1).await, ["text:🙂"]);
        assert_eq!(icons(&p, ":arrow", 32).await.contains(&"text:→".to_owned()), true);
    }

    #[tokio::test]
    async fn recently_used_come_first() {
        let p = provider(Settings::default());
        assert_eq!(icons(&p, ":", 1).await, ["text:😂"]);
        p.store.record_open("emoji:🦀").unwrap();
        assert_eq!(icons(&p, ":", 2).await, ["text:🦀", "text:😂"]);
    }

    #[tokio::test]
    async fn one_quiet_suggestion_without_a_prefix() {
        let p = provider(Settings::default());
        let items = p.query("thumbs up", 32).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].score, SUGGESTION_SCORE);
        assert!(p.query("chrome", 32).await.unwrap().is_empty());
        assert!(p.query("thumbs", 32).await.unwrap().is_empty());
        assert!(p.query("ok", 32).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn skin_tone_applies_to_icons_and_paste() {
        let mut settings = Settings::default();
        settings.emoji.skin_tone = SkinTone::MediumDark;
        let p = provider(settings);
        assert_eq!(icons(&p, ":thumbs up", 1).await, ["text:👍🏾"]);
        assert_eq!(
            p.activate("emoji:👍", Action::Paste).await.unwrap(),
            Activation::Paste {
                text: Some("👍🏾".into())
            }
        );
        // No tone for emoji without one, or symbols.
        assert_eq!(
            p.activate("emoji:→", Action::Copy).await.unwrap(),
            Activation::Copy { text: "→".into() }
        );
        assert!(p.activate("emoji:", Action::Copy).await.is_err());
    }

    #[tokio::test]
    async fn off_means_nothing() {
        let mut settings = Settings::default();
        settings.emoji.enabled = false;
        let p = provider(settings);
        assert!(!p.claims(":fire"));
        assert!(p.query(":fire", 32).await.unwrap().is_empty());
    }
}
