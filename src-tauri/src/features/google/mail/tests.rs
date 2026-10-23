use super::*;

const LIST: &str = include_str!("../../../../tests/fixtures/google/threads.json");
const THREAD: &str = include_str!("../../../../tests/fixtures/google/thread.json");
const LABEL: &str = include_str!("../../../../tests/fixtures/google/inbox-label.json");
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
    assert_eq!(
        sender_name("\"Silva, Ana\" <ana@example.com>"),
        "Silva, Ana"
    );
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
    assert_eq!(
        thread_url("19a1"),
        "https://mail.google.com/mail/u/0/#inbox/19a1"
    );
    assert_eq!(
        search_url("from:ana lunch"),
        "https://mail.google.com/mail/u/0/#search/from%3Aana%20lunch"
    );
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
    assert_eq!(
        super::rows(&Inbox::default(), "", None, NOW_MS)[0].title,
        "No unread mail in Inbox"
    );
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
    assert_eq!(
        ids,
        ["gmail:t3", "gmail:t1", "google:mail-search:LUNCH fri"]
    );
    assert_eq!(rows[1].accessory.as_deref(), Some("Unread"));
    assert_eq!(rows[0].accessory, None);
    assert_eq!(rows[2].title, "Search Gmail for “LUNCH fri”");
    let by_sender = super::rows(&inbox, "acme", None, NOW_MS);
    assert_eq!(by_sender[0].id, "gmail:t4");
    let nothing = super::rows(&inbox, "zebra", None, NOW_MS);
    assert_eq!(nothing.len(), 1, "just the search row");
}
