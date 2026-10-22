//! Search rows for conversation history (pure): which queries are meant
//! for it, which conversations match, and how each row reads.

use super::store::Summary;
use crate::features::clipboard::content::age;
use crate::fuzzy::FuzzyQuery;
use crate::search::{glyph, Action, ItemKind, ResultItem};

/// Every id this feature makes starts with this.
pub const PREFIX: &str = "history:";
/// The commands: `history:<name>`. Conversations are `history:<number>`.
pub const OPEN_ID: &str = "history:open";
pub const RESUME_ID: &str = "history:resume";
pub const CLEAR_ID: &str = "history:clear";
pub const OFF_ID: &str = "history:off";
pub const EMPTY_ID: &str = "history:empty";

/// What `Chat History` and a cleared list go back to.
pub const LIST_QUERY: &str = "history ";

/// A title must match at least this well: scattered letters across a long
/// question match nearly anything.
const MIN_TITLE_SCORE: f32 = 0.5;

/// The filter text if `query` is meant for chat history: `>history`,
/// `history` or `chats` (with or without the `>`), alone or followed by a
/// space and text.
pub fn history_filter(query: &str) -> Option<&str> {
    let query = query.strip_prefix('>').unwrap_or(query);
    ["history", "chats"].iter().find_map(|word| {
        let head = query.get(..word.len())?;
        if !head.eq_ignore_ascii_case(word) {
            return None;
        }
        let rest = &query[word.len()..];
        if rest.is_empty() {
            Some("")
        } else if rest.starts_with(char::is_whitespace) {
            Some(rest.trim())
        } else {
            None
        }
    })
}

/// The row id for a saved conversation, and back.
pub fn entry_id(id: i64) -> String {
    format!("{PREFIX}{id}")
}

pub fn parse_entry_id(id: &str) -> Option<i64> {
    id.strip_prefix(PREFIX)?.parse().ok()
}

/// Matches a conversation by its title (fuzzy) or by words in what was
/// said (each word anywhere, as typed).
pub struct Filter {
    title: FuzzyQuery,
    words: Vec<String>,
}

impl Filter {
    pub fn new(text: &str) -> Self {
        Self {
            title: FuzzyQuery::new(text),
            words: text.split_whitespace().map(str::to_lowercase).collect(),
        }
    }

    pub fn matches(&mut self, summary: &Summary) -> bool {
        let title = self
            .title
            .score(&summary.title)
            .is_some_and(|score| score >= MIN_TITLE_SCORE);
        title || (!self.words.is_empty() && self.words.iter().all(|w| summary.search.contains(w)))
    }
}

/// Rows for the list (pinned first, then most recently used), filtered by
/// `filter`. Scores fall from 1 down the list: the ranker keeps
/// conversation rows in this order.
pub fn conversation_rows(
    summaries: &[Summary],
    filter: &str,
    now_ms: i64,
    limit: usize,
) -> Vec<ResultItem> {
    let mut filter = (!filter.is_empty()).then(|| Filter::new(filter));
    summaries
        .iter()
        .filter(|s| filter.as_mut().is_none_or(|f| f.matches(s)))
        .take(limit)
        .enumerate()
        .map(|(i, s)| conversation_row(s, now_ms, 1.0 - i as f32 / (limit as f32 + 1.0)))
        .collect()
}

fn conversation_row(summary: &Summary, now_ms: i64, score: f32) -> ResultItem {
    let mut subtitle = format!(
        "{} · {}",
        summary.provider.name(),
        age(now_ms, summary.updated_ms)
    );
    if summary.pinned {
        subtitle.push_str(" · Pinned");
    }
    ResultItem::new(entry_id(summary.id), ItemKind::Conversation, &summary.title)
        .with_subtitle(subtitle)
        .with_icon(glyph("chat"))
        .with_score(score)
        .with_actions([Action::Open, Action::Pin, Action::Delete])
        .with_accessory(messages(summary.messages))
}

/// "1 message", "12 messages".
pub fn messages(count: u32) -> String {
    if count == 1 {
        "1 message".into()
    } else {
        format!("{count} messages")
    }
}

/// "Chat History": opens the list.
pub fn open_row() -> ResultItem {
    ResultItem::new(OPEN_ID, ItemKind::Command, "Chat History")
        .with_subtitle("Past chats with Claude and ChatGPT. Type history to search them")
        .with_icon(glyph("history"))
        .with_actions([Action::Run])
}

/// "Resume Last Chat", naming the chat it opens (`latest`).
pub fn resume_row(latest: Option<&Summary>) -> ResultItem {
    let subtitle = match latest {
        Some(s) => format!("{} · {}", s.title, s.provider.name()),
        None => "No saved chats yet".into(),
    };
    ResultItem::new(RESUME_ID, ItemKind::Command, "Resume Last Chat")
        .with_subtitle(subtitle)
        .with_icon(glyph("chat"))
        .with_actions([Action::Run])
}

pub fn clear_row() -> ResultItem {
    ResultItem::new(CLEAR_ID, ItemKind::Command, "Clear Chat History")
        .with_subtitle("Pinned chats stay")
        .with_icon(glyph("trash"))
        .with_actions([Action::Run])
        .with_confirm("Delete your saved chats?")
}

/// Shown instead of the list while nothing is saved.
pub fn empty_row(enabled: bool) -> ResultItem {
    if enabled {
        ResultItem::new(EMPTY_ID, ItemKind::Command, "No saved chats yet")
            .with_subtitle("Chats with Claude and ChatGPT show up here")
            .with_icon(glyph("chat"))
            .with_actions([Action::Run])
    } else {
        off_row()
    }
}

/// History is off: new chats aren't saved.
pub fn off_row() -> ResultItem {
    ResultItem::new(OFF_ID, ItemKind::Command, "Chat history is off")
        .with_subtitle("New chats aren't saved. Turn it on in Settings")
        .with_icon(glyph("settings"))
        .with_actions([Action::Run])
}

/// The commands that show in normal search, scored by how well their
/// title matches.
pub fn command_rows(query: &str, latest: Option<&Summary>) -> Vec<ResultItem> {
    let mut fuzzy = FuzzyQuery::new(query);
    [open_row(), resume_row(latest), clear_row()]
        .into_iter()
        .filter_map(|row| {
            let score = fuzzy.score(&row.title)?;
            Some(row.with_score(score))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ProviderId;

    #[test]
    fn claims_history_and_chats() {
        assert_eq!(history_filter("history"), Some(""));
        assert_eq!(history_filter(">history"), Some(""));
        assert_eq!(
            history_filter("History  sort my files "),
            Some("sort my files")
        );
        assert_eq!(history_filter(">History budget"), Some("budget"));
        assert_eq!(history_filter("chats"), Some(""));
        assert_eq!(history_filter(">chats taxes"), Some("taxes"));
        assert_eq!(history_filter("historyx"), None);
        assert_eq!(history_filter("chat"), None, "ChatGPT stays findable");
        assert_eq!(history_filter("chatgpt"), None);
        assert_eq!(history_filter("hist"), None);
        assert_eq!(history_filter("my history"), None);
        assert_eq!(history_filter(">"), None);
        assert_eq!(history_filter("historé"), None);
    }

    #[test]
    fn entry_ids_round_trip() {
        assert_eq!(parse_entry_id(&entry_id(42)), Some(42));
        assert_eq!(parse_entry_id(CLEAR_ID), None);
        assert_eq!(parse_entry_id(RESUME_ID), None);
        assert_eq!(parse_entry_id("clip:42"), None);
        for id in [OPEN_ID, RESUME_ID, CLEAR_ID, OFF_ID, EMPTY_ID] {
            assert!(id.starts_with(PREFIX));
        }
    }

    fn summary(id: i64, title: &str, search: &str, updated_ms: i64) -> Summary {
        Summary {
            id,
            provider: ProviderId::Claude,
            title: title.into(),
            updated_ms,
            messages: 2,
            pinned: false,
            search: search.into(),
        }
    }

    fn ids(rows: &[ResultItem]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn rows_keep_the_list_order() {
        let mut pinned = summary(3, "Budget plan", "budget plan\n", 0);
        pinned.pinned = true;
        pinned.provider = ProviderId::Chatgpt;
        let list = [
            pinned,
            summary(
                2,
                "Sort my downloads",
                "sort my downloads\nmoved 3 files\n",
                0,
            ),
            summary(1, "What's on my calendar", "what's on my calendar\n", 0),
        ];
        let rows = conversation_rows(&list, "", 3 * 3_600_000, 10);
        assert_eq!(ids(&rows), ["history:3", "history:2", "history:1"]);
        assert!(rows.windows(2).all(|w| w[0].score > w[1].score));
        assert_eq!(rows[0].subtitle, "ChatGPT · 3 h ago · Pinned");
        assert_eq!(rows[1].subtitle, "Claude · 3 h ago");
        assert_eq!(rows[1].kind, ItemKind::Conversation);
        assert_eq!(rows[1].icon.as_deref(), Some("glyph:chat"));
        assert_eq!(rows[1].accessory.as_deref(), Some("2 messages"));
        assert_eq!(rows[1].actions[0], Action::Open);
        assert_eq!(conversation_rows(&list, "", 0, 2).len(), 2);
    }

    #[test]
    fn filters_by_title_and_by_what_was_said() {
        let list = [
            summary(
                2,
                "Sort my downloads",
                "sort my downloads\nmoved 3 files into pdfs\n",
                0,
            ),
            summary(1, "Plan a trip", "plan a trip\ntry kyoto in april\n", 0),
        ];
        let found = |filter| -> Vec<String> {
            conversation_rows(&list, filter, 0, 10)
                .into_iter()
                .map(|r| r.id)
                .collect()
        };
        assert_eq!(found("sort"), ["history:2"]);
        assert_eq!(found("dwnlds"), ["history:2"], "fuzzy on the title");
        assert_eq!(found("Kyoto"), ["history:1"], "words in the answer");
        assert_eq!(found("april kyoto"), ["history:1"], "every word, any order");
        assert!(found("kyoto pdfs").is_empty());
        assert!(found("zzz").is_empty());
    }

    #[test]
    fn message_counts_read_naturally() {
        assert_eq!(messages(1), "1 message");
        assert_eq!(messages(0), "0 messages");
        assert_eq!(messages(14), "14 messages");
    }

    #[test]
    fn commands_match_by_title() {
        let latest = summary(5, "Sort my downloads", "", 0);
        let rows = command_rows("resume", Some(&latest));
        assert_eq!(ids(&rows), [RESUME_ID]);
        assert_eq!(rows[0].subtitle, "Sort my downloads · Claude");
        let rows = command_rows("chat hist", None);
        assert!(ids(&rows).contains(&OPEN_ID));
        let clear = command_rows("clear chat", None);
        assert!(clear
            .iter()
            .any(|r| r.id == CLEAR_ID && r.confirm.is_some()));
        assert!(command_rows("firefox", None).is_empty());
        assert_eq!(resume_row(None).subtitle, "No saved chats yet");
    }

    #[test]
    fn empty_and_off_rows() {
        assert_eq!(empty_row(true).id, EMPTY_ID);
        assert_eq!(empty_row(false).id, OFF_ID);
    }
}
