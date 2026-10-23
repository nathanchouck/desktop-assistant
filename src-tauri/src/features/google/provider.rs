//! What Google adds to search: `cal` and `mail` (claimed while connected),
//! Drive files merged into normal results, and what picking one does.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone};

use super::api::{ApiError, Service};
use super::calendar::{self, Event};
use super::drive;
use super::hub::Google;
use super::mail::{self, Inbox};
use super::sentence;
use super::sync::Fetched;
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};

/// Opens Settings → Google.
pub const CONNECT_ID: &str = "google:connect";
/// Reads Google again and refreshes the rows.
pub const REFRESH_ID: &str = "google:refresh";
/// Opens Google Calendar in the browser.
pub const CALENDAR_WEB_ID: &str = "google:calendar";
/// Opens the Cloud Console page that turns an API on; the address follows.
pub const ENABLE_PREFIX: &str = "google:enable:";
/// Where to turn APIs on when Google didn't say.
const API_LIBRARY: &str = "https://console.cloud.google.com/apis/library";
/// "Connect Google" rows sit under apps named like the prefix (Calculator).
const HINT_SCORE: f32 = 0.05;
/// How long Enter on "check again" waits for Google.
const REFRESH_WAIT: Duration = Duration::from_secs(8);

/// Ids whose rows keep this provider's order (time order) in the ranker.
pub fn keeps_order(id: &str) -> bool {
    [calendar::ID_PREFIX, mail::ID_PREFIX, "google:"]
        .iter()
        .any(|p| id.starts_with(p))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode<'a> {
    /// `cal` or `cal <title words>`.
    Calendar(&'a str),
    /// `mail` or `mail <words>`.
    Mail(&'a str),
    /// Anything else: Drive files, maybe.
    Search,
}

/// What a query asks for.
pub fn mode(query: &str) -> Mode<'_> {
    if let Some(text) = after_word(query, "cal") {
        Mode::Calendar(text)
    } else if let Some(text) = after_word(query, "mail") {
        Mode::Mail(text)
    } else {
        Mode::Search
    }
}

/// The text after `word` (any case) when the query is that word alone or
/// followed by a space: `cal` → "", `cal standup` → "standup", `calc` → no.
fn after_word<'a>(query: &'a str, word: &str) -> Option<&'a str> {
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
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// "Connect Google to see your calendar": opens the settings.
fn connect_row(service: Service) -> ResultItem {
    let (what, icon) = match service {
        Service::Calendar => ("calendar", "calendar"),
        Service::Gmail => ("mail", "mail"),
        Service::Drive => ("Drive", "drive"),
    };
    ResultItem::new(
        CONNECT_ID,
        ItemKind::Command,
        format!("Connect Google to see your {what}"),
    )
    .with_subtitle("Settings → Google")
    .with_icon(glyph(icon))
    .with_actions([Action::Open])
    .with_score(HINT_SCORE)
}

fn loading_row(service: Service) -> ResultItem {
    let what = match service {
        Service::Calendar => "calendar",
        _ => "inbox",
    };
    ResultItem::new(
        REFRESH_ID,
        ItemKind::Command,
        format!("Loading your {what}…"),
    )
    .with_subtitle("Press Enter to check again")
    .with_icon(glyph(if service == Service::Calendar {
        "calendar"
    } else {
        "mail"
    }))
    .with_actions([Action::Run])
    .with_score(1.0)
}

/// A row that says what went wrong and what Enter does about it.
fn error_row(service: Service, error: &ApiError) -> ResultItem {
    let row = match error {
        ApiError::Disabled { link, .. } => ResultItem::new(
            format!("{ENABLE_PREFIX}{}", link.as_deref().unwrap_or(API_LIBRARY)),
            ItemKind::Command,
            format!("Turn on the {}", service.api_name()),
        )
        .with_subtitle("It's off in your Google Cloud project · Enter opens the page"),
        ApiError::NotConnected | ApiError::Reconnect(_) | ApiError::NotAllowed(_) => {
            ResultItem::new(CONNECT_ID, ItemKind::Command, "Connect Google again")
                .with_subtitle(sentence(&error.to_string()))
        }
        _ => ResultItem::new(
            REFRESH_ID,
            ItemKind::Command,
            format!("Couldn't read your {}", service.short_name()),
        )
        .with_subtitle(sentence(&error.to_string())),
    };
    let action = if row.id == REFRESH_ID {
        Action::Run
    } else {
        Action::Open
    };
    row.with_icon(glyph("warning"))
        .with_actions([action])
        .with_score(1.0)
}

/// "today or tomorrow", for the empty calendar row.
fn period(days: u32) -> String {
    match days {
        0 | 1 => "today".into(),
        2 => "today or tomorrow".into(),
        n => format!("in the next {n} days"),
    }
}

fn calendar_web_row(title: String) -> ResultItem {
    ResultItem::new(CALENDAR_WEB_ID, ItemKind::Link, title)
        .with_subtitle("Google Calendar")
        .with_icon(glyph("calendar"))
        .with_actions([Action::Open])
}

/// `cal …` rows from what's cached.
pub fn calendar_rows<Tz: TimeZone>(
    fetched: &Fetched<Vec<Event>>,
    text: &str,
    days: u32,
    now: &DateTime<Tz>,
) -> Vec<ResultItem> {
    let Some(events) = &fetched.data else {
        return vec![match &fetched.error {
            Some(e) => error_row(Service::Calendar, e),
            None => loading_row(Service::Calendar),
        }];
    };
    let mut rows = calendar::rows(events, text, now);
    if rows.is_empty() {
        let title = if text.is_empty() {
            format!("Nothing else on your calendar {}", period(days))
        } else {
            format!("No upcoming events match “{text}”")
        };
        rows.push(calendar_web_row(title).with_score(0.5));
    } else if text.is_empty() {
        rows.push(calendar_web_row("Open Google Calendar".into()).with_score(0.0));
    }
    rows
}

/// `mail …` rows from what's cached.
pub fn mail_rows(
    fetched: &Fetched<Inbox>,
    text: &str,
    email: Option<&str>,
    now_ms: i64,
) -> Vec<ResultItem> {
    match (&fetched.data, &fetched.error) {
        (Some(inbox), _) => mail::rows(inbox, text, email, now_ms),
        (None, Some(e)) => vec![error_row(Service::Gmail, e)],
        (None, None) => vec![loading_row(Service::Gmail)],
    }
}

pub struct GoogleProvider {
    google: Arc<Google>,
}

impl GoogleProvider {
    pub fn new(google: Arc<Google>) -> Self {
        Self { google }
    }

    async fn drive_rows(&self, query: &str) -> Vec<ResultItem> {
        let google = &self.google;
        let settings = google.settings();
        let account = &google.account;
        if !settings.drive_in_search
            || !drive::wanted(query)
            || !account.connected()
            || !account.granted().drive
        {
            return Vec::new();
        }
        let files = google.drive.search(&google.api, query).await;
        drive::rows(&files, query, now_ms())
    }

    fn calendar_event(&self, id: &str) -> Option<Event> {
        let events = self.google.cache.calendar().data?;
        events.into_iter().find(|e| e.id == id)
    }
}

fn notice(message: &str) -> Activation {
    Activation::Notice {
        message: message.into(),
    }
}

#[async_trait]
impl SearchProvider for GoogleProvider {
    fn name(&self) -> &'static str {
        "google"
    }

    async fn query(&self, query: &str, _limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let google = &self.google;
        let settings = google.settings();
        if !settings.enabled {
            return Ok(Vec::new());
        }
        let connected = google.account.connected();
        let rows = match mode(query) {
            Mode::Calendar(text) | Mode::Mail(text) if !connected => {
                // A hint only for the bare word, under any app it names.
                if !text.is_empty() {
                    return Ok(Vec::new());
                }
                let service = match mode(query) {
                    Mode::Calendar(_) => Service::Calendar,
                    _ => Service::Gmail,
                };
                vec![connect_row(service)]
            }
            Mode::Calendar(text) => {
                google.cache.refresh_if_stale();
                calendar_rows(
                    &google.cache.calendar(),
                    text,
                    settings.calendar_days,
                    &chrono::Local::now(),
                )
            }
            Mode::Mail(text) => {
                google.cache.refresh_if_stale();
                let email = google.account.email();
                mail_rows(&google.cache.mail(), text, email.as_deref(), now_ms())
            }
            Mode::Search => self.drive_rows(query).await,
        };
        Ok(rows)
    }

    /// `cal` and `mail` are this feature's alone while it's connected.
    fn claims(&self, query: &str) -> bool {
        mode(query) != Mode::Search && self.google.active()
    }

    fn owns(&self, id: &str) -> bool {
        keeps_order(id) || id.starts_with(drive::ID_PREFIX)
    }

    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError> {
        let open = |url: String| Activation::OpenUrl { url };
        if let Some(event_id) = id.strip_prefix(calendar::ID_PREFIX) {
            let Some(event) = self.calendar_event(event_id) else {
                return Ok(notice("That event isn't in your calendar any more."));
            };
            return Ok(match (action, &event.meeting, event.open_url()) {
                (Action::Copy, Some(meeting), _) => Activation::Copy {
                    text: meeting.url.clone(),
                },
                (Action::Copy, None, _) => notice("This event has no meeting link."),
                (_, _, Some(url)) => open(url.to_owned()),
                (_, _, None) => notice("This event has no link to open."),
            });
        }
        if let Some(thread) = id.strip_prefix(mail::ID_PREFIX) {
            return Ok(open(mail::thread_url(thread)));
        }
        if let Some(file_id) = id.strip_prefix(drive::ID_PREFIX) {
            let link = self
                .google
                .drive
                .file(file_id)
                .map(|f| f.link)
                .unwrap_or_else(|| drive::fallback_link(file_id));
            return Ok(match action {
                Action::Copy => Activation::Copy { text: link },
                _ => open(link),
            });
        }
        if let Some(text) = id.strip_prefix(mail::SEARCH_ID) {
            return Ok(open(mail::search_url(text)));
        }
        if let Some(link) = id.strip_prefix(ENABLE_PREFIX) {
            let link = if link.starts_with("https://console.") {
                link
            } else {
                API_LIBRARY
            };
            return Ok(open(link.to_owned()));
        }
        Ok(match id {
            mail::INBOX_ID => open(mail::inbox_url()),
            CALENDAR_WEB_ID => open(calendar::WEB_URL.to_owned()),
            CONNECT_ID => Activation::OpenSettings,
            REFRESH_ID => {
                if !self.google.active() {
                    return Ok(Activation::OpenSettings);
                }
                self.google.cache.refresh_and_wait(REFRESH_WAIT).await;
                Activation::Refresh
            }
            _ => {
                return Err(SearchError::Provider {
                    provider: self.name(),
                    message: format!("{id} can't be opened"),
                })
            }
        })
    }
}

#[cfg(test)]
mod tests;
