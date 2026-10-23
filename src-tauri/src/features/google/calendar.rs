//! `cal`: the next events from your primary calendar. Parsing Calendar's
//! `events.list` answer and turning events into rows (pure, tested with
//! recorded answers).

use chrono::{DateTime, Days, FixedOffset, NaiveDate, SecondsFormat, TimeZone};
use serde::Deserialize;

use super::meeting::{self, Meeting};
use crate::net;
use crate::search::{glyph, Action, ItemKind, ResultItem};

pub const HOST: &str = "www.googleapis.com";
pub const ID_PREFIX: &str = "gcal:";
/// Google Calendar in the browser (the "Open Google Calendar" row).
pub const WEB_URL: &str = "https://calendar.google.com/calendar/r";
/// More can't fit in a few days of a sane calendar.
const MAX_EVENTS: &str = "50";
/// Only what the rows need (descriptions are read for call links).
const FIELDS: &str = "items(id,status,summary,location,description,htmlLink,hangoutLink,\
conferenceData/entryPoints(entryPointType,uri),start,end,attendees(self,responseStatus),eventType)";
/// An event starting this soon says so ("in 25 min").
const SOON_MINUTES: i64 = 60;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub when: When,
    pub location: Option<String>,
    /// The event in Google Calendar.
    pub html_link: Option<String>,
    pub meeting: Option<Meeting>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum When {
    Timed {
        start: DateTime<FixedOffset>,
        end: DateTime<FixedOffset>,
    },
    /// `end` is the day after the last one, as Calendar sends it.
    AllDay { start: NaiveDate, end: NaiveDate },
}

impl Event {
    /// What Enter opens: the call if there is one, else the event.
    pub fn open_url(&self) -> Option<&str> {
        self.meeting
            .as_ref()
            .map(|m| m.url.as_str())
            .or(self.html_link.as_deref())
    }

    /// Not over yet at `now`.
    pub fn upcoming<Tz: TimeZone>(&self, now: &DateTime<Tz>) -> bool {
        match &self.when {
            When::Timed { end, .. } => *end > now.fixed_offset(),
            When::AllDay { end, .. } => *end > now.date_naive(),
        }
    }
}

/// The events request: from `now` (events still going count, since
/// `timeMin` bounds the end) to the end of the `days`th day.
pub fn events_path<Tz: TimeZone>(now: &DateTime<Tz>, days: u32) -> String {
    let days = days.clamp(1, 7);
    let time_min = stamp(&now.fixed_offset());
    let time_max = now
        .date_naive()
        .checked_add_days(Days::new(u64::from(days)))
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .and_then(|midnight| now.timezone().from_local_datetime(&midnight).earliest())
        .map(|end| stamp(&end.fixed_offset()))
        .unwrap_or_else(|| stamp(&(now.fixed_offset() + chrono::Duration::days(i64::from(days)))));
    let query = net::form(&[
        ("timeMin", &time_min),
        ("timeMax", &time_max),
        ("singleEvents", "true"),
        ("orderBy", "startTime"),
        ("maxResults", MAX_EVENTS),
        ("fields", FIELDS),
    ]);
    format!("/calendar/v3/calendars/primary/events?{query}")
}

/// Whole seconds: Calendar ignores fractions, and may refuse nanoseconds.
fn stamp(time: &DateTime<FixedOffset>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, false)
}

#[derive(Deserialize)]
struct EventList {
    #[serde(default)]
    items: Vec<EventJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventJson {
    id: String,
    status: Option<String>,
    summary: Option<String>,
    location: Option<String>,
    description: Option<String>,
    html_link: Option<String>,
    hangout_link: Option<String>,
    conference_data: Option<ConferenceJson>,
    start: Option<TimeJson>,
    end: Option<TimeJson>,
    #[serde(default)]
    attendees: Vec<AttendeeJson>,
    event_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConferenceJson {
    #[serde(default)]
    entry_points: Vec<EntryPointJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryPointJson {
    entry_point_type: Option<String>,
    uri: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimeJson {
    date_time: Option<String>,
    date: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttendeeJson {
    #[serde(rename = "self", default)]
    is_self: bool,
    response_status: Option<String>,
}

/// Reads `events.list`. Cancelled events, ones you declined and "working
/// location" markers are left out; an event that can't be read is skipped.
pub fn parse_events(body: &[u8]) -> Result<Vec<Event>, String> {
    let list: EventList = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    Ok(list.items.into_iter().filter_map(event).collect())
}

fn event(json: EventJson) -> Option<Event> {
    if json.status.as_deref() == Some("cancelled")
        || json.event_type.as_deref() == Some("workingLocation")
        || json
            .attendees
            .iter()
            .any(|a| a.is_self && a.response_status.as_deref() == Some("declined"))
    {
        return None;
    }
    let when = when(json.start.as_ref()?, json.end.as_ref()?)?;
    let entry_points: Vec<(String, String)> = json
        .conference_data
        .map(|c| c.entry_points)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| Some((p.entry_point_type?, p.uri?)))
        .collect();
    let meeting = meeting::find(
        &entry_points,
        json.hangout_link.as_deref(),
        json.location.as_deref(),
        json.description.as_deref(),
    );
    let title = json
        .summary
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(No title)".into());
    Some(Event {
        id: json.id,
        title,
        when,
        location: json
            .location
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty()),
        html_link: json.html_link.filter(|l| l.starts_with("https://")),
        meeting,
    })
}

fn when(start: &TimeJson, end: &TimeJson) -> Option<When> {
    if let (Some(s), Some(e)) = (&start.date_time, &end.date_time) {
        return Some(When::Timed {
            start: DateTime::parse_from_rfc3339(s).ok()?,
            end: DateTime::parse_from_rfc3339(e).ok()?,
        });
    }
    let day = |d: &Option<String>| NaiveDate::parse_from_str(d.as_deref()?, "%Y-%m-%d").ok();
    Some(When::AllDay {
        start: day(&start.date)?,
        end: day(&end.date)?,
    })
}

/// "Today", "Tomorrow", "Yesterday" or "Sat 24 Oct".
fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    match (day - today).num_days() {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        _ => day.format("%a %-d %b").to_string(),
    }
}

/// "Today 14:00–15:00", "Today 23:00 – Tomorrow 01:00", "Tomorrow · All day".
pub fn describe_when<Tz: TimeZone>(when: &When, now: &DateTime<Tz>) -> String {
    let today = now.date_naive();
    match when {
        When::Timed { start, end } => {
            let tz = now.timezone();
            let (start, end) = (start.with_timezone(&tz), end.with_timezone(&tz));
            let (from, to) = (start.date_naive(), end.date_naive());
            let time = |t: &DateTime<Tz>| t.naive_local().format("%H:%M").to_string();
            if from == to {
                format!("{} {}–{}", day_label(from, today), time(&start), time(&end))
            } else {
                format!(
                    "{} {} – {} {}",
                    day_label(from, today),
                    time(&start),
                    day_label(to, today),
                    time(&end)
                )
            }
        }
        When::AllDay { start, end } => {
            let last = end.pred_opt().filter(|d| d > start).unwrap_or(*start);
            if last == *start {
                format!("{} · All day", day_label(*start, today))
            } else {
                format!(
                    "{} – {} · All day",
                    day_label(*start, today),
                    day_label(last, today)
                )
            }
        }
    }
}

/// "Now" while it's on, "in 25 min" when it starts within the hour.
pub fn soon<Tz: TimeZone>(when: &When, now: &DateTime<Tz>) -> Option<String> {
    let When::Timed { start, end } = when else {
        return None;
    };
    let now = now.fixed_offset();
    if *start <= now && now < *end {
        return Some("Now".into());
    }
    let minutes = (*start - now).num_minutes();
    (0..SOON_MINUTES)
        .contains(&minutes)
        .then(|| format!("in {} min", minutes.max(1)))
}

/// A location worth showing: not just the call's address again.
fn place(event: &Event) -> Option<&str> {
    let location = event.location.as_deref()?;
    (!location.starts_with("https://") && !location.starts_with("http://")).then_some(location)
}

/// Whether every word of `filter` is in the title.
pub fn matches(event: &Event, filter: &str) -> bool {
    let title = event.title.to_lowercase();
    filter
        .split_whitespace()
        .all(|word| title.contains(&word.to_lowercase()))
}

/// Rows for the events still to come, in time order, filtered by title.
pub fn rows<Tz: TimeZone>(events: &[Event], filter: &str, now: &DateTime<Tz>) -> Vec<ResultItem> {
    events
        .iter()
        .filter(|e| e.upcoming(now) && matches(e, filter))
        .enumerate()
        .map(|(i, event)| row(event, now).with_score(1.0 - i as f32 * 0.001))
        .collect()
}

fn row<Tz: TimeZone>(event: &Event, now: &DateTime<Tz>) -> ResultItem {
    let mut subtitle = describe_when(&event.when, now);
    for part in [place(event), event.meeting.as_ref().map(|m| m.service)]
        .into_iter()
        .flatten()
    {
        subtitle.push_str(" · ");
        subtitle.push_str(part);
    }
    let mut actions = vec![Action::Open];
    if event.meeting.is_some() {
        // Ctrl+Shift+C copies the call's link.
        actions.push(Action::Copy);
    }
    let item = ResultItem::new(
        format!("{ID_PREFIX}{}", event.id),
        ItemKind::Link,
        &event.title,
    )
    .with_subtitle(subtitle)
    .with_icon(glyph("calendar"))
    .with_actions(actions);
    match soon(&event.when, now) {
        Some(text) => item.with_accessory(text),
        None => item,
    }
}

#[cfg(test)]
mod tests;
