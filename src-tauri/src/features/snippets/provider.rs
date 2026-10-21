//! Snippets in search: found by name or keyword (`snip` lists them all).
//! Enter pastes the snippet into the window you were in; Ctrl+Shift+C
//! copies it.

use async_trait::async_trait;

use super::expand;
use super::{Snippet, SnippetSettings};
use crate::features::clipboard::system;
use crate::fuzzy::FuzzyQuery;
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};
use crate::settings::{self, SharedSettings};

const NAME: &str = "snippets";
/// Snippets: `snippet:<keyword>` (or `snippet:<name>` without a keyword).
const PREFIX: &str = "snippet:";
const SETTINGS_ID: &str = "snippets:settings";
/// Characters of the text shown under the name.
const SUBTITLE_CHARS: usize = 80;

pub struct SnippetProvider {
    settings: SharedSettings,
}

impl SnippetProvider {
    pub fn new(settings: SharedSettings) -> Self {
        Self { settings }
    }
}

/// The filter text if `query` asks for the snippet list: "snip",
/// "snippet" or "snippets", alone or followed by a space and text.
pub fn list_filter(query: &str) -> Option<&str> {
    ["snippets", "snippet", "snip"].iter().find_map(|prefix| {
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

/// A snippet's stable id: by keyword, else by name.
pub fn snippet_id(snippet: &Snippet) -> String {
    let key = snippet.keyword.trim();
    let key = if key.is_empty() {
        snippet.name.trim()
    } else {
        key
    };
    format!("{PREFIX}{key}")
}

/// How well `query` finds a snippet: its keyword typed out (with or
/// without the trigger) is a sure hit; otherwise its name, fuzzily.
fn score(fuzzy: &mut FuzzyQuery, query: &str, snippet: &Snippet, trigger: &str) -> Option<f32> {
    let query = query.trim().to_lowercase();
    let keyword = snippet.keyword.trim().to_lowercase();
    let trigger = trigger.to_lowercase();
    let bare = keyword.strip_prefix(&trigger).unwrap_or(&keyword);
    if !keyword.is_empty() && (query == keyword || query == bare) {
        return Some(1.0);
    }
    // The start of a keyword, from two characters on (one letter would
    // match half the list).
    let by_keyword = (query.chars().count() >= 2
        && !keyword.is_empty()
        && (keyword.starts_with(&query) || (!bare.is_empty() && bare.starts_with(&query))))
    .then_some(0.9);
    let by_name = fuzzy.score(&snippet.name);
    by_keyword.into_iter().chain(by_name).reduce(f32::max)
}

fn row(snippet: &Snippet, score: f32) -> ResultItem {
    let first_line = snippet
        .text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut subtitle: String = first_line.chars().take(SUBTITLE_CHARS).collect();
    if first_line.chars().count() > SUBTITLE_CHARS {
        subtitle.push('…');
    }
    let title = if snippet.name.trim().is_empty() {
        snippet.keyword.trim()
    } else {
        snippet.name.trim()
    };
    let item = ResultItem::new(snippet_id(snippet), ItemKind::Snippet, title)
        .with_subtitle(subtitle)
        .with_icon(glyph("snippet"))
        .with_score(score)
        .with_actions([Action::Paste, Action::Copy]);
    let keyword = snippet.keyword.trim();
    if keyword.is_empty() {
        item
    } else {
        item.with_accessory(keyword)
    }
}

/// Rows for `query`: every snippet for the `snip` list (filtered by any
/// text after it), else the snippets the query finds.
pub fn rows(settings: &SnippetSettings, query: &str) -> Vec<ResultItem> {
    let trigger = settings.trigger.trim();
    let usable = settings
        .items
        .iter()
        .filter(|s| !(s.name.trim().is_empty() && s.keyword.trim().is_empty()));
    match list_filter(query) {
        Some("") => usable.map(|s| row(s, 0.5)).collect(),
        Some(filter) => {
            let mut fuzzy = FuzzyQuery::new(filter);
            usable
                .filter_map(|s| {
                    let found = score(&mut fuzzy, filter, s, trigger)
                        .or_else(|| fuzzy.score(&s.text).map(|t| t * 0.5))?;
                    Some(row(s, found))
                })
                .collect()
        }
        None => {
            let mut fuzzy = FuzzyQuery::new(query);
            usable
                .filter_map(|s| Some(row(s, score(&mut fuzzy, query, s, trigger)?)))
                .collect()
        }
    }
}

fn failed(message: impl ToString) -> SearchError {
    SearchError::Provider {
        provider: NAME,
        message: message.to_string(),
    }
}

/// A snippet's text with its placeholders filled in.
pub async fn filled_in(text: &str) -> String {
    let clipboard = if expand::needs_clipboard(text) {
        tokio::task::spawn_blocking(system::read_text)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    } else {
        String::new()
    };
    expand::expand(text, &clipboard, chrono::Local::now().naive_local())
}

#[async_trait]
impl SearchProvider for SnippetProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let settings = settings::current(&self.settings).snippets;
        if !settings.enabled {
            return Ok(Vec::new());
        }
        let mut found = rows(&settings, query);
        found.truncate(limit);
        if found.is_empty() && list_filter(query) == Some("") {
            found.push(
                ResultItem::new(SETTINGS_ID, ItemKind::Command, "No snippets yet")
                    .with_subtitle("Add some in Settings")
                    .with_icon(glyph("settings"))
                    .with_actions([Action::Run]),
            );
        }
        Ok(found)
    }

    fn claims(&self, query: &str) -> bool {
        list_filter(query).is_some() && settings::current(&self.settings).snippets.enabled
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(PREFIX) || id.starts_with("snippets:")
    }

    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError> {
        if id == SETTINGS_ID {
            return Ok(Activation::OpenSettings);
        }
        let settings = settings::current(&self.settings).snippets;
        let snippet = settings
            .items
            .iter()
            .find(|s| snippet_id(s) == id)
            .ok_or_else(|| failed("that snippet is gone"))?;
        let text = filled_in(&snippet.text).await;
        match action {
            Action::Copy => Ok(Activation::Copy { text }),
            _ => Ok(Activation::Paste { text: Some(text) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn snippet(name: &str, keyword: &str, text: &str) -> Snippet {
        Snippet {
            name: name.into(),
            keyword: keyword.into(),
            text: text.into(),
        }
    }

    fn settings() -> SnippetSettings {
        SnippetSettings {
            items: vec![
                snippet("Email signature", ";sig", "Best,\nNathan"),
                snippet("Home address", ";addr", "1 Main St"),
                snippet("Shrug", "", "¯\\_(ツ)_/¯"),
            ],
            ..SnippetSettings::default()
        }
    }

    fn ids(rows: &[ResultItem]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn list_prefixes() {
        assert_eq!(list_filter("snip"), Some(""));
        assert_eq!(list_filter("Snippets"), Some(""));
        assert_eq!(list_filter("snippet sig"), Some("sig"));
        assert_eq!(list_filter("snipping tool"), None);
        assert_eq!(list_filter("sni"), None);
    }

    #[test]
    fn ids_are_stable_and_readable() {
        assert_eq!(snippet_id(&snippet("Sig", ";sig", "")), "snippet:;sig");
        assert_eq!(snippet_id(&snippet("Shrug", " ", "")), "snippet:Shrug");
    }

    #[test]
    fn finds_by_keyword_or_name() {
        let s = settings();
        assert_eq!(ids(&rows(&s, ";sig")), ["snippet:;sig"]);
        let by_bare = rows(&s, "sig");
        assert_eq!(by_bare[0].id, "snippet:;sig");
        assert_eq!(by_bare[0].score, 1.0);
        assert!(ids(&rows(&s, "address")).contains(&"snippet:;addr"));
        assert!(ids(&rows(&s, "shrug")).contains(&"snippet:Shrug"));
        assert!(rows(&s, "zzz").is_empty());
        let sig = &rows(&s, ";sig")[0];
        assert_eq!(sig.title, "Email signature");
        assert_eq!(sig.subtitle, "Best,");
        assert_eq!(sig.accessory.as_deref(), Some(";sig"));
        assert_eq!(sig.actions, [Action::Paste, Action::Copy]);
    }

    #[test]
    fn snip_lists_all_and_filters_by_text_too() {
        let s = settings();
        assert_eq!(rows(&s, "snip").len(), 3);
        // The text counts too, below a name match.
        assert_eq!(ids(&rows(&s, "snip nathan")), ["snippet:;sig"]);
        assert!(rows(&s, "snip nathan")[0].score <= 0.5);
    }

    #[tokio::test]
    async fn pastes_or_copies_the_filled_in_text() {
        let mut s = settings();
        s.items.push(snippet("Today", ";d", "on {date}"));
        let all = crate::settings::Settings {
            snippets: s,
            ..crate::settings::Settings::default()
        };
        let p = SnippetProvider::new(Arc::new(RwLock::new(all)));
        assert!(p.claims("snip"));
        assert!(p.owns("snippet:;sig"));
        assert_eq!(
            p.activate("snippet:;sig", Action::Paste).await.unwrap(),
            Activation::Paste {
                text: Some("Best,\nNathan".into())
            }
        );
        let Activation::Copy { text } = p.activate("snippet:;d", Action::Copy).await.unwrap()
        else {
            panic!("expected a copy");
        };
        assert!(text.starts_with("on 20") && !text.contains('{'), "{text}");
        assert!(p.activate("snippet:gone", Action::Paste).await.is_err());
    }

    #[tokio::test]
    async fn empty_or_off() {
        let p = SnippetProvider::new(Arc::new(RwLock::new(crate::settings::Settings {
            snippets: SnippetSettings {
                items: Vec::new(),
                ..SnippetSettings::default()
            },
            ..crate::settings::Settings::default()
        })));
        let found = p.query("snip", 32).await.unwrap();
        assert_eq!(ids(&found), [SETTINGS_ID]);
        p.settings.write().unwrap().snippets.enabled = false;
        assert!(!p.claims("snip"));
        assert!(p.query("snip", 32).await.unwrap().is_empty());
    }
}
