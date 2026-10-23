use super::*;

const FIXTURE: &str = include_str!("../../../../tests/fixtures/google/events.json");

/// 10:00 on Friday 23 October 2026, two hours east of UTC.
fn now() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-10-23T10:00:00+02:00").unwrap()
}

fn events() -> Vec<Event> {
    parse_events(FIXTURE.as_bytes()).unwrap()
}

#[test]
fn parses_the_recorded_answer() {
    let events = events();
    let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
    // Cancelled, declined and working-location entries are gone.
    assert_eq!(
        titles,
        [
            "Company offsite",
            "Breakfast",
            "Standup",
            "Design review",
            "(No title)",
            "Dinner with Ana",
            "Planning",
        ]
    );
    let standup = &events[2];
    assert_eq!(standup.id, "abc123_20261023T083000Z");
    assert_eq!(
        standup.meeting.as_ref().map(|m| m.url.as_str()),
        Some("https://meet.google.com/abc-defg-hij")
    );
    assert_eq!(
        standup.open_url(),
        Some("https://meet.google.com/abc-defg-hij")
    );
    let review = &events[3];
    assert_eq!(review.meeting.as_ref().map(|m| m.service), Some("Zoom"));
    let dinner = &events[5];
    assert_eq!(dinner.meeting, None);
    assert_eq!(
        dinner.open_url(),
        Some("https://www.google.com/calendar/event?eid=ZGlubmVy")
    );
    assert!(parse_events(b"{}").unwrap().is_empty());
    assert!(parse_events(b"not json").is_err());
}

#[test]
fn describes_when() {
    let now = now();
    let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap();
    let timed = |a: &str, b: &str| When::Timed {
        start: at(a),
        end: at(b),
    };
    assert_eq!(
        describe_when(
            &timed("2026-10-23T14:00:00+02:00", "2026-10-23T15:00:00+02:00"),
            &now
        ),
        "Today 14:00–15:00"
    );
    // Times show in your zone, whatever zone the event was made in.
    assert_eq!(
        describe_when(&timed("2026-10-24T07:30:00Z", "2026-10-24T08:00:00Z"), &now),
        "Tomorrow 09:30–10:00"
    );
    assert_eq!(
        describe_when(
            &timed("2026-10-23T23:00:00+02:00", "2026-10-24T01:00:00+02:00"),
            &now
        ),
        "Today 23:00 – Tomorrow 01:00"
    );
    assert_eq!(
        describe_when(
            &timed("2026-10-26T09:00:00+01:00", "2026-10-26T10:00:00+01:00"),
            &now
        ),
        "Mon 26 Oct 10:00–11:00"
    );
    let day = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
    assert_eq!(
        describe_when(
            &When::AllDay {
                start: day("2026-10-24"),
                end: day("2026-10-25")
            },
            &now
        ),
        "Tomorrow · All day"
    );
    assert_eq!(
        describe_when(
            &When::AllDay {
                start: day("2026-10-22"),
                end: day("2026-10-25")
            },
            &now
        ),
        "Yesterday – Tomorrow · All day"
    );
}

#[test]
fn says_when_its_soon() {
    let now = now();
    let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap();
    let timed = |a: &str, b: &str| When::Timed {
        start: at(a),
        end: at(b),
    };
    assert_eq!(
        soon(
            &timed("2026-10-23T09:30:00+02:00", "2026-10-23T10:30:00+02:00"),
            &now
        )
        .as_deref(),
        Some("Now")
    );
    assert_eq!(
        soon(
            &timed("2026-10-23T10:25:00+02:00", "2026-10-23T11:00:00+02:00"),
            &now
        )
        .as_deref(),
        Some("in 25 min")
    );
    assert_eq!(
        soon(
            &timed("2026-10-23T10:00:30+02:00", "2026-10-23T11:00:00+02:00"),
            &now
        )
        .as_deref(),
        Some("in 1 min")
    );
    assert_eq!(
        soon(
            &timed("2026-10-23T11:00:00+02:00", "2026-10-23T12:00:00+02:00"),
            &now
        ),
        None
    );
    assert_eq!(
        soon(
            &timed("2026-10-23T08:00:00+02:00", "2026-10-23T09:00:00+02:00"),
            &now
        ),
        None
    );
}

#[test]
fn rows_skip_whats_over_and_follow_the_filter() {
    let now = now();
    let rows = rows(&events(), "", &now);
    let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    // Breakfast (08:00–09:00) is over; the offsite runs all week.
    assert_eq!(
        titles,
        [
            "Company offsite",
            "Standup",
            "Design review",
            "(No title)",
            "Dinner with Ana",
            "Planning"
        ]
    );
    let standup = &rows[1];
    assert_eq!(standup.id, "gcal:abc123_20261023T083000Z");
    assert_eq!(standup.subtitle, "Today 10:30–10:45 · Google Meet");
    assert_eq!(standup.accessory.as_deref(), Some("in 30 min"));
    assert_eq!(standup.actions, [Action::Open, Action::Copy]);
    assert_eq!(standup.icon.as_deref(), Some("glyph:calendar"));
    assert_eq!(standup.kind, ItemKind::Link);
    let review = &rows[2];
    assert_eq!(review.subtitle, "Today 14:00–15:00 · Room 4 · Zoom");
    let dinner = &rows[4];
    assert_eq!(dinner.subtitle, "Tomorrow 19:30–21:00 · Trattoria Roma");
    assert_eq!(dinner.actions, [Action::Open]);
    assert!(
        rows.windows(2).all(|w| w[0].score > w[1].score),
        "time order"
    );

    let filtered: Vec<String> = super::rows(&events(), "REVIEW des", &now)
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(filtered, ["Design review"]);
    assert!(super::rows(&events(), "nothing like this", &now).is_empty());
}

#[test]
fn asks_for_the_right_days() {
    let path = events_path(&now(), 2);
    assert!(path.starts_with("/calendar/v3/calendars/primary/events?"));
    assert!(
        path.contains("timeMin=2026-10-23T10%3A00%3A00%2B02%3A00"),
        "{path}"
    );
    assert!(
        path.contains("timeMax=2026-10-25T00%3A00%3A00%2B02%3A00"),
        "{path}"
    );
    assert!(path.contains("singleEvents=true&orderBy=startTime"));
    // Out-of-range settings are clamped.
    assert!(events_path(&now(), 0).contains("timeMax=2026-10-24T00"));
    assert!(events_path(&now(), 30).contains("timeMax=2026-10-30T00"));
}
