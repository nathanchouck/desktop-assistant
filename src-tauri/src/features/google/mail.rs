//! `mail`: unread count and the latest threads in your Gmail inbox, from
//! their headers only (the `gmail.metadata` scope can't read bodies, and
//! can't search either: Gmail refuses `q` with it). Parsing and rows are
//! pure and tested with recorded answers.

use serde::Deserialize;

use crate::features::clipboard::content::age;
use crate::net;
use crate::search::{glyph, Action, ItemKind, ResultItem};

pub const HOST: &str = "gmail.googleapis.com";
pub const ID_PREFIX: &str = "gmail:";
/// The "N unread in Inbox" row.
pub const INBOX_ID: &str = "google:inbox";
/// The "Search Gmail for …" row; the text follows.
pub const SEARCH_ID: &str = "google:mail-search:";
const WEB: &str = "https://mail.google.com/mail/u/0/";
/// Unread threads listed under the count.
pub const UNREAD_SHOWN: u32 = 10;
/// Recent inbox threads kept for `mail <text>` (read or not).
pub const RECENT_KEPT: u32 = 20;

pub fn inbox_url() -> String {
    format!("{WEB}#inbox")
}

pub fn thread_url(id: &str) -> String {
    format!("{WEB}#inbox/{}", net::encode(id))
}

pub fn search_url(text: &str) -> String {
    format!("{WEB}#search/{}", net::encode(text))
}

pub fn inbox_label_path() -> String {
    "/gmail/v1/users/me/labels/INBOX?fields=threadsUnread".into()
}

/// Inbox threads, newest first; only unread ones when `unread`.
pub fn threads_path(unread: bool, max: u32) -> String {
    let max = max.to_string();
    let mut pairs = vec![("labelIds", "INBOX")];
    if unread {
        pairs.push(("labelIds", "UNREAD"));
    }
    pairs.extend([("maxResults", max.as_str()), ("fields", "threads(id,historyId)")]);
    format!("/gmail/v1/users/me/threads?{}", net::form(&pairs))
}

/// One thread's subject, senders and dates (no bodies, no snippets).
pub fn thread_path(id: &str) -> String {
    let query = net::form(&[
        ("format", "metadata"),
        ("metadataHeaders", "Subject"),
        ("metadataHeaders", "From"),
        ("fields", "id,historyId,messages(labelIds,internalDate,payload/headers)"),
    ]);
    format!("/gmail/v1/users/me/threads/{}?{query}", net::encode(id))
}

/// A thread as the rows show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    /// Changes whenever anything in the thread does, so an unchanged
    /// thread isn't fetched again.
    pub history_id: String,
    pub subject: String,
    /// Who wrote the latest unread message (else the latest one).
    pub from: String,
    /// When the latest message came, Unix milliseconds.
    pub date_ms: i64,
    pub unread: bool,
}

/// What `mail` shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inbox {
    pub unread_count: u32,
    /// Newest first.
    pub unread: Vec<Thread>,
    /// The newest inbox threads, read or not, for `mail <text>`.
    pub recent: Vec<Thread>,
}

impl Inbox {
    pub fn thread(&self, id: &str) -> Option<&Thread> {
        self.unread.iter().chain(&self.recent).find(|t| t.id == id)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LabelJson {
    threads_unread: Option<u32>,
}

/// `labels.get` for the inbox → its unread thread count.
pub fn parse_unread_count(body: &[u8]) -> Result<u32, String> {
    let label: LabelJson = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    Ok(label.threads_unread.unwrap_or(0))
}

#[derive(Deserialize)]
struct ThreadListJson {
    #[serde(default)]
    threads: Vec<ThreadRefJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadRefJson {
    id: String,
    #[serde(default)]
    history_id: String,
}

/// `threads.list` → (id, history id), newest first.
pub fn parse_thread_list(body: &[u8]) -> Result<Vec<(String, String)>, String> {
    let list: ThreadListJson = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    Ok(list
        .threads
        .into_iter()
        .map(|t| (t.id, t.history_id))
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadJson {
    id: String,
    #[serde(default)]
    history_id: String,
    #[serde(default)]
    messages: Vec<MessageJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageJson {
    #[serde(default)]
    label_ids: Vec<String>,
    internal_date: Option<String>,
    payload: Option<PayloadJson>,
}

#[derive(Deserialize)]
struct PayloadJson {
    #[serde(default)]
    headers: Vec<HeaderJson>,
}

#[derive(Deserialize)]
struct HeaderJson {
    name: String,
    value: String,
}

impl MessageJson {
    fn header(&self, name: &str) -> Option<&str> {
        self.payload
            .as_ref()?
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.trim())
            .filter(|v| !v.is_empty())
    }

    fn unread(&self) -> bool {
        self.label_ids.iter().any(|l| l == "UNREAD")
    }
}

/// `threads.get` (metadata) → the thread's row data.
pub fn parse_thread(body: &[u8]) -> Result<Thread, String> {
    let json: ThreadJson = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let messages = &json.messages;
    let first = messages.first().ok_or("the thread has no messages")?;
    let latest = messages.last().unwrap_or(first);
    let latest_unread = messages.iter().rev().find(|m| m.unread());
    let subject = messages
        .iter()
        .find_map(|m| m.header("Subject"))
        .unwrap_or("(no subject)")
        .to_owned();
    let from = latest_unread
        .unwrap_or(latest)
        .header("From")
        .map(sender_name)
        .unwrap_or_default();
    Ok(Thread {
        id: json.id,
        history_id: json.history_id,
        subject,
        from,
        date_ms: latest
            .internal_date
            .as_deref()
            .and_then(|d| d.parse().ok())
            .unwrap_or(0),
        unread: latest_unread.is_some(),
    })
}

/// `"Ana Silva" <ana@example.com>` → `Ana Silva`; a bare address stays.
pub fn sender_name(from: &str) -> String {
    let from = from.trim();
    let name = match from.rfind('<') {
        Some(at) if from.ends_with('>') => from[..at].trim(),
        _ => from,
    };
    let name = name.trim_matches('"').trim();
    if name.is_empty() {
        from.trim_start_matches('<').trim_end_matches('>').to_owned()
    } else {
        name.to_owned()
    }
}

/// "3 unread in Inbox"; Enter opens Gmail.
pub fn inbox_row(unread: u32, account: Option<&str>) -> ResultItem {
    let title = match unread {
        0 => "No unread mail in Inbox".to_owned(),
        n => format!("{n} unread in Inbox"),
    };
    let subtitle = match account {
        Some(email) => format!("Gmail · {email}"),
        None => "Gmail".to_owned(),
    };
    ResultItem::new(INBOX_ID, ItemKind::Link, title)
        .with_subtitle(subtitle)
        .with_icon(glyph("mail"))
        .with_actions([Action::Open])
        .with_score(1.0)
}

fn thread_row(thread: &Thread, now_ms: i64, mark_unread: bool) -> ResultItem {
    let mut subtitle = thread.from.clone();
    if thread.date_ms > 0 {
        if !subtitle.is_empty() {
            subtitle.push_str(" · ");
        }
        subtitle.push_str(&age(now_ms, thread.date_ms));
    }
    let row = ResultItem::new(
        format!("{ID_PREFIX}{}", thread.id),
        ItemKind::Link,
        &thread.subject,
    )
    .with_subtitle(subtitle)
    .with_icon(glyph("mail"))
    .with_actions([Action::Open]);
    if mark_unread && thread.unread {
        row.with_accessory("Unread")
    } else {
        row
    }
}

/// Whether every word of `filter` is in the subject or the sender.
fn matches(thread: &Thread, filter: &str) -> bool {
    let text = format!("{} {}", thread.subject, thread.from).to_lowercase();
    filter
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

/// `mail`: the count, then the unread threads. `mail <text>`: threads
/// whose subject or sender has every word (unread or recent), then a row
/// that searches all of Gmail in the browser.
pub fn rows(inbox: &Inbox, filter: &str, account: Option<&str>, now_ms: i64) -> Vec<ResultItem> {
    let filter = filter.trim();
    let mut rows = Vec::new();
    if filter.is_empty() {
        rows.push(inbox_row(inbox.unread_count, account));
        rows.extend(
            inbox
                .unread
                .iter()
                .take(UNREAD_SHOWN as usize)
                .map(|t| thread_row(t, now_ms, false)),
        );
    } else {
        let mut seen = std::collections::HashSet::new();
        let mut found: Vec<&Thread> = inbox
            .unread
            .iter()
            .chain(&inbox.recent)
            .filter(|t| seen.insert(t.id.as_str()) && matches(t, filter))
            .collect();
        found.sort_by_key(|t| std::cmp::Reverse(t.date_ms));
        rows.extend(found.into_iter().map(|t| thread_row(t, now_ms, true)));
        rows.push(
            ResultItem::new(
                format!("{SEARCH_ID}{filter}"),
                ItemKind::Link,
                format!("Search Gmail for “{filter}”"),
            )
            .with_subtitle("Opens Gmail in the browser")
            .with_icon(glyph("search"))
            .with_actions([Action::Open]),
        );
    }
    let count = rows.len();
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| row.with_score(1.0 - i as f32 / (count as f32 + 1.0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = include_str!("../../../tests/fixtures/google/threads.json");
    const THREAD: &str = include_str!("../../../tests/fixtures/google/thread.json");
    const LABEL: &str = include_str!("../../../tests/fixtures/google/inbox-label.json");
    /// 23 Oct 2026, 08:00 UTC.
    const NOW_MS: i64 = 1_792_742_400_000;

    fn thread(id: &str, subject: &str, from: &str, hours_ago: i64, unread: bool) -> Thread {
        Thread {
            id: id.into(),
            history_id: "1".into(),
            subject: subject.into(),
            from: from.into(),
            date_ms: NOW_MS - hours_ago * 3_600_000,
            unread,
        }
    }

    #[test]
    fn parses_the_recorded_answers() {
        assert_eq!(parse_unread_count(LABEL.as_bytes()), Ok(4));
        assert_eq!(parse_unread_count(b"{}"), Ok(0));
        assert_eq!(
            parse_thread_list(LIST.as_bytes()).unwrap(),
            [
                ("19a1f2e3d4c5b6a7".to_owned(), "998877".to_owned()),
                ("19a1f2e3d4c5b6a0".to_owned(), "998801".to_owned()),
            ]
        );
        assert!(parse_thread_list(b"{}").unwrap().is_empty());
        let thread = parse_thread(THREAD.as_bytes()).unwrap();
        assert_eq!(
            thread,
            Thread {
                id: "19a1f2e3d4c5b6a7".into(),
                history_id: "998877".into(),
                // The first message's subject; the latest unread sender.
                subject: "Lunch on Friday?".into(),
                from: "Ana Silva".into(),
                date_ms: 1_792_738_800_000,
                unread: true,
            }
        );
        assert!(parse_thread(br#"{"id":"x","messages":[]}"#).is_err());
        assert!(parse_thread(b"[]").is_err());
    }

    #[test]
    fn sender_names() {
        assert_eq!(sender_name("\"Ana Silva\" <ana@example.com>"), "Ana Silva");
        assert_eq!(sender_name("Bob <bob@example.com>"), "Bob");
        assert_eq!(sender_name("\"Silva, Ana\" <ana@example.com>"), "Silva, Ana");
        assert_eq!(sender_name("carol@example.com"), "carol@example.com");
        assert_eq!(sender_name("<dave@example.com>"), "dave@example.com");
        assert_eq!(sender_name("  "), "");
    }

    #[test]
    fn requests() {
        assert_eq!(
            threads_path(true, 10),
            "/gmail/v1/users/me/threads?labelIds=INBOX&labelIds=UNREAD&maxResults=10&fields=threads%28id%2ChistoryId%29"
        );
        assert!(!threads_path(false, 20).contains("UNREAD"));
        assert!(thread_path("19a/../x").starts_with("/gmail/v1/users/me/threads/19a%2F..%2Fx?format=metadata&metadataHeaders=Subject&metadataHeaders=From&"));
        assert_eq!(thread_url("19a1"), "https://mail.google.com/mail/u/0/#inbox/19a1");
        assert_eq!(search_url("from:ana lunch"), "https://mail.google.com/mail/u/0/#search/from%3Aana%20lunch");
        assert_eq!(inbox_url(), "https://mail.google.com/mail/u/0/#inbox");
    }

    #[test]
    fn mail_lists_the_count_then_unread_threads() {
        let inbox = Inbox {
            unread_count: 2,
            unread: vec![
                thread("t2", "Invoice", "Acme Billing", 2, true),
                thread("t1", "Lunch on Friday?", "Ana Silva", 30, true),
            ],
            recent: vec![thread("t3", "Old news", "Bob", 50, false)],
        };
        let rows = rows(&inbox, "", Some("me@example.com"), NOW_MS);
        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["2 unread in Inbox", "Invoice", "Lunch on Friday?"]);
        assert_eq!(rows[0].id, INBOX_ID);
        assert_eq!(rows[0].subtitle, "Gmail · me@example.com");
        assert_eq!(rows[1].id, "gmail:t2");
        assert_eq!(rows[1].subtitle, "Acme Billing · 2 h ago");
        assert_eq!(rows[2].subtitle, "Ana Silva · yesterday");
        assert_eq!(rows[1].icon.as_deref(), Some("glyph:mail"));
        assert!(rows.windows(2).all(|w| w[0].score > w[1].score));
        assert_eq!(super::rows(&Inbox::default(), "", None, NOW_MS)[0].title, "No unread mail in Inbox");
        assert_eq!(inbox.thread("t3").map(|t| t.subject.as_str()), Some("Old news"));
    }

    #[test]
    fn mail_text_filters_and_offers_gmail_search() {
        let inbox = Inbox {
            unread_count: 1,
            unread: vec![thread("t1", "Lunch on Friday?", "Ana Silva", 3, true)],
            recent: vec![
                thread("t1", "Lunch on Friday?", "Ana Silva", 3, true),
                thread("t3", "Friday lunch photos", "Bob", 1, false),
                thread("t4", "Invoice", "Acme", 0, false),
            ],
        };
        let rows = rows(&inbox, "LUNCH fri", None, NOW_MS);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["gmail:t3", "gmail:t1", "google:mail-search:LUNCH fri"]);
        assert_eq!(rows[1].accessory.as_deref(), Some("Unread"));
        assert_eq!(rows[0].accessory, None);
        assert_eq!(rows[2].title, "Search Gmail for “LUNCH fri”");
        let by_sender = super::rows(&inbox, "acme", None, NOW_MS);
        assert_eq!(by_sender[0].id, "gmail:t4");
        let nothing = super::rows(&inbox, "zebra", None, NOW_MS);
        assert_eq!(nothing.len(), 1, "just the search row");
    }
}
