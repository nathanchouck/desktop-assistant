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
    ResultItem::new(CONNECT_ID, ItemKind::Command, format!("Connect Google to see your {what}"))
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
    ResultItem::new(REFRESH_ID, ItemKind::Command, format!("Loading your {what}…"))
        .with_subtitle("Press Enter to check again")
        .with_icon(glyph(if service == Service::Calendar { "calendar" } else { "mail" }))
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
                .with_subtitle(capitalized(&error.to_string()))
        }
        _ => ResultItem::new(
            REFRESH_ID,
            ItemKind::Command,
            format!("Couldn't read your {}", service.short_name()),
        )
        .with_subtitle(capitalized(&error.to_string())),
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

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
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
pub fn mail_rows(fetched: &Fetched<Inbox>, text: &str, email: Option<&str>, now_ms: i64) -> Vec<ResultItem> {
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
mod tests {
    use super::*;
    use crate::features::google::account::AccountInfo;
    use crate::features::google::meeting::Meeting;
    use crate::settings::Settings;
    use chrono::FixedOffset;
    use std::sync::RwLock;

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-23T10:00:00+02:00").unwrap()
    }

    fn event(id: &str, title: &str, start: &str, end: &str, meeting: bool) -> Event {
        Event {
            id: id.into(),
            title: title.into(),
            when: calendar::When::Timed {
                start: DateTime::parse_from_rfc3339(start).unwrap(),
                end: DateTime::parse_from_rfc3339(end).unwrap(),
            },
            location: None,
            html_link: Some(format!("https://www.google.com/calendar/event?eid={id}")),
            meeting: meeting.then(|| Meeting {
                url: "https://meet.google.com/abc-defg-hij".into(),
                service: "Google Meet",
            }),
        }
    }

    fn fetched<T>(data: Option<T>, error: Option<ApiError>) -> Fetched<T> {
        Fetched {
            data,
            error,
            tried: None,
        }
    }

    #[test]
    fn modes() {
        assert_eq!(mode("cal"), Mode::Calendar(""));
        assert_eq!(mode("CAL  standup "), Mode::Calendar("standup"));
        assert_eq!(mode("calc"), Mode::Search);
        assert_eq!(mode("calendar"), Mode::Search);
        assert_eq!(mode("mail"), Mode::Mail(""));
        assert_eq!(mode("Mail invoice"), Mode::Mail("invoice"));
        assert_eq!(mode("mailbox"), Mode::Search);
        assert_eq!(mode("ca"), Mode::Search);
        assert_eq!(mode("é"), Mode::Search);
        assert_eq!(mode("budget"), Mode::Search);
    }

    #[test]
    fn calendar_rows_by_state() {
        let now = now();
        let loading = calendar_rows(&fetched(None, None), "", 2, &now);
        assert_eq!(loading[0].id, REFRESH_ID);
        let off = ApiError::Disabled {
            service: Service::Calendar,
            link: Some("https://console.developers.google.com/apis/api/calendar-json.googleapis.com/overview?project=1".into()),
        };
        let failed = calendar_rows(&fetched(None, Some(off)), "", 2, &now);
        assert_eq!(failed[0].title, "Turn on the Google Calendar API");
        assert!(failed[0].id.starts_with("google:enable:https://console.developers.google.com/"));
        let dead = calendar_rows(&fetched(None, Some(ApiError::Reconnect("the sign-in expired".into()))), "", 2, &now);
        assert_eq!((dead[0].id.as_str(), dead[0].subtitle.as_str()), (CONNECT_ID, "The sign-in expired"));

        let events = vec![
            event("a", "Standup", "2026-10-23T10:30:00+02:00", "2026-10-23T10:45:00+02:00", true),
            event("b", "Review", "2026-10-23T14:00:00+02:00", "2026-10-23T15:00:00+02:00", false),
        ];
        let rows = calendar_rows(&fetched(Some(events.clone()), None), "", 2, &now);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["gcal:a", "gcal:b", CALENDAR_WEB_ID]);
        assert!(rows.iter().all(|r| keeps_order(&r.id)));
        let none = calendar_rows(&fetched(Some(events.clone()), None), "lunch", 2, &now);
        assert_eq!(none[0].title, "No upcoming events match “lunch”");
        let empty = calendar_rows(&fetched(Some(Vec::new()), None), "", 1, &now);
        assert_eq!(empty[0].title, "Nothing else on your calendar today");
        assert_eq!(period(5), "in the next 5 days");
        // Stale data still shows when the last refresh failed.
        let stale = calendar_rows(&fetched(Some(events), Some(ApiError::NotConnected)), "", 2, &now);
        assert_eq!(stale[0].id, "gcal:a");
    }

    #[test]
    fn mail_rows_by_state() {
        assert_eq!(mail_rows(&fetched(None, None), "", None, 0)[0].title, "Loading your inbox…");
        let failed = mail_rows(
            &fetched(None, Some(ApiError::NotAllowed(Service::Gmail))),
            "",
            None,
            0,
        );
        assert_eq!(failed[0].subtitle, "Gmail access wasn't allowed. Connect again and tick it");
        let rows = mail_rows(&fetched(Some(Inbox::default()), None), "", None, 0);
        assert_eq!(rows[0].id, mail::INBOX_ID);
    }

    fn provider(connected: bool) -> (GoogleProvider, Arc<Google>) {
        let mut settings = Settings::default();
        settings.google.client_id = "id.apps".into();
        let dir = tempfile::tempdir().unwrap();
        let google = Arc::new(Google::new(dir.path(), Arc::new(RwLock::new(settings))));
        if connected {
            google.account.set_info(Some(AccountInfo {
                email: Some("me@example.com".into()),
                client_id: "id.apps".into(),
                scope: None,
            }));
        }
        (GoogleProvider::new(Arc::clone(&google)), google)
    }

    #[tokio::test]
    async fn claims_only_while_connected() {
        let (p, _) = provider(false);
        assert!(!p.claims("cal"));
        let rows = p.query("cal", 32).await.unwrap();
        assert_eq!(rows[0].id, CONNECT_ID);
        assert_eq!(rows[0].title, "Connect Google to see your calendar");
        assert!(p.query("mail invoice", 32).await.unwrap().is_empty());
        assert!(p.query("budget", 32).await.unwrap().is_empty(), "no Drive while disconnected");
        assert_eq!(p.activate(CONNECT_ID, Action::Open).await.unwrap(), Activation::OpenSettings);

        let (p, google) = provider(true);
        assert!(p.claims("cal") && p.claims("mail lunch"));
        assert!(!p.claims("calc") && !p.claims("budget"));
        assert_eq!(p.query("mail", 32).await.unwrap()[0].title, "Loading your inbox…");
        // Drive answers from its cache at once.
        google.drive.put(
            "budget",
            drive::parse_files(br#"{"files":[{"id":"f1","name":"Budget","webViewLink":"https://docs.google.com/d/f1"}]}"#).unwrap(),
        );
        let rows = p.query("budget", 32).await.unwrap();
        assert_eq!(rows[0].id, "gdrive:f1");
        assert!(p.query("bu", 32).await.unwrap().is_empty(), "too short for Drive");
        assert_eq!(
            p.activate("gdrive:f1", Action::Copy).await.unwrap(),
            Activation::Copy { text: "https://docs.google.com/d/f1".into() }
        );
        assert_eq!(
            p.activate("gdrive:gone", Action::Open).await.unwrap(),
            Activation::OpenUrl { url: "https://drive.google.com/open?id=gone".into() }
        );
        // Turned off: nothing, and nothing claimed.
        let (p, google) = provider(true);
        google.settings.write().unwrap().google.enabled = false;
        assert!(!p.claims("cal"));
        assert!(p.query("cal", 32).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn activations() {
        let (p, google) = provider(true);
        let open = |url: &str| Activation::OpenUrl { url: url.into() };
        assert_eq!(
            p.activate("gmail:19a1", Action::Open).await.unwrap(),
            open("https://mail.google.com/mail/u/0/#inbox/19a1")
        );
        assert_eq!(
            p.activate(mail::INBOX_ID, Action::Open).await.unwrap(),
            open("https://mail.google.com/mail/u/0/#inbox")
        );
        assert_eq!(
            p.activate("google:mail-search:lunch ana", Action::Open).await.unwrap(),
            open("https://mail.google.com/mail/u/0/#search/lunch%20ana")
        );
        assert_eq!(
            p.activate(CALENDAR_WEB_ID, Action::Open).await.unwrap(),
            open(calendar::WEB_URL)
        );
        assert_eq!(
            p.activate("google:enable:https://evil.example/x", Action::Open).await.unwrap(),
            open(API_LIBRARY)
        );
        assert!(matches!(
            p.activate("gcal:missing", Action::Open).await.unwrap(),
            Activation::Notice { .. }
        ));
        google.cache.put_calendar(vec![
            event("a", "Standup", "2026-10-23T10:30:00+02:00", "2026-10-23T10:45:00+02:00", true),
            event("b", "Review", "2026-10-23T14:00:00+02:00", "2026-10-23T15:00:00+02:00", false),
        ]);
        assert_eq!(
            p.activate("gcal:a", Action::Open).await.unwrap(),
            open("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(
            p.activate("gcal:a", Action::Copy).await.unwrap(),
            Activation::Copy { text: "https://meet.google.com/abc-defg-hij".into() }
        );
        assert_eq!(
            p.activate("gcal:b", Action::Open).await.unwrap(),
            open("https://www.google.com/calendar/event?eid=b")
        );
        assert!(matches!(
            p.activate("gcal:b", Action::Copy).await.unwrap(),
            Activation::Notice { .. }
        ));
        assert!(p.activate("google:nope", Action::Open).await.is_err());
        assert!(p.owns("gcal:x") && p.owns("gdrive:x") && p.owns("google:inbox"));
        assert!(!p.owns("file:x") && !keeps_order("gdrive:x"));
    }
}
