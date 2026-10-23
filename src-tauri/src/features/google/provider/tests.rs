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
    assert!(failed[0]
        .id
        .starts_with("google:enable:https://console.developers.google.com/"));
    let dead = calendar_rows(
        &fetched(
            None,
            Some(ApiError::Reconnect("the sign-in expired".into())),
        ),
        "",
        2,
        &now,
    );
    assert_eq!(
        (dead[0].id.as_str(), dead[0].subtitle.as_str()),
        (CONNECT_ID, "The sign-in expired")
    );

    let events = vec![
        event(
            "a",
            "Standup",
            "2026-10-23T10:30:00+02:00",
            "2026-10-23T10:45:00+02:00",
            true,
        ),
        event(
            "b",
            "Review",
            "2026-10-23T14:00:00+02:00",
            "2026-10-23T15:00:00+02:00",
            false,
        ),
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
    let stale = calendar_rows(
        &fetched(Some(events), Some(ApiError::NotConnected)),
        "",
        2,
        &now,
    );
    assert_eq!(stale[0].id, "gcal:a");
}

#[test]
fn mail_rows_by_state() {
    assert_eq!(
        mail_rows(&fetched(None, None), "", None, 0)[0].title,
        "Loading your inbox…"
    );
    let failed = mail_rows(
        &fetched(None, Some(ApiError::NotAllowed(Service::Gmail))),
        "",
        None,
        0,
    );
    assert_eq!(
        failed[0].subtitle,
        "Gmail access wasn't allowed. Connect again and tick it"
    );
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
    assert!(
        p.query("budget", 32).await.unwrap().is_empty(),
        "no Drive while disconnected"
    );
    assert_eq!(
        p.activate(CONNECT_ID, Action::Open).await.unwrap(),
        Activation::OpenSettings
    );

    let (p, google) = provider(true);
    assert!(p.claims("cal") && p.claims("mail lunch"));
    assert!(!p.claims("calc") && !p.claims("budget"));
    assert_eq!(
        p.query("mail", 32).await.unwrap()[0].title,
        "Loading your inbox…"
    );
    // Drive answers from its cache at once.
    google.drive.put(
        "budget",
        drive::parse_files(br#"{"files":[{"id":"f1","name":"Budget","webViewLink":"https://docs.google.com/d/f1"}]}"#).unwrap(),
    );
    let rows = p.query("budget", 32).await.unwrap();
    assert_eq!(rows[0].id, "gdrive:f1");
    assert!(
        p.query("bu", 32).await.unwrap().is_empty(),
        "too short for Drive"
    );
    assert_eq!(
        p.activate("gdrive:f1", Action::Copy).await.unwrap(),
        Activation::Copy {
            text: "https://docs.google.com/d/f1".into()
        }
    );
    assert_eq!(
        p.activate("gdrive:gone", Action::Open).await.unwrap(),
        Activation::OpenUrl {
            url: "https://drive.google.com/open?id=gone".into()
        }
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
        p.activate("google:mail-search:lunch ana", Action::Open)
            .await
            .unwrap(),
        open("https://mail.google.com/mail/u/0/#search/lunch%20ana")
    );
    assert_eq!(
        p.activate(CALENDAR_WEB_ID, Action::Open).await.unwrap(),
        open(calendar::WEB_URL)
    );
    assert_eq!(
        p.activate("google:enable:https://evil.example/x", Action::Open)
            .await
            .unwrap(),
        open(API_LIBRARY)
    );
    assert!(matches!(
        p.activate("gcal:missing", Action::Open).await.unwrap(),
        Activation::Notice { .. }
    ));
    google.cache.put_calendar(vec![
        event(
            "a",
            "Standup",
            "2026-10-23T10:30:00+02:00",
            "2026-10-23T10:45:00+02:00",
            true,
        ),
        event(
            "b",
            "Review",
            "2026-10-23T14:00:00+02:00",
            "2026-10-23T15:00:00+02:00",
            false,
        ),
    ]);
    assert_eq!(
        p.activate("gcal:a", Action::Open).await.unwrap(),
        open("https://meet.google.com/abc-defg-hij")
    );
    assert_eq!(
        p.activate("gcal:a", Action::Copy).await.unwrap(),
        Activation::Copy {
            text: "https://meet.google.com/abc-defg-hij".into()
        }
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
