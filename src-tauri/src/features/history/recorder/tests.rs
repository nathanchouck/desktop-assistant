use super::*;
use crate::features::history::transcript::ChatEntry;

fn meta(record: i64) -> Meta {
    let resumable = Resumable::new(ProviderId::Claude, PathBuf::from("/work"), None);
    Meta::new(record, resumable, "  sort my\n downloads ", 100)
}

fn delta(text: &str) -> AiEvent {
    AiEvent::TextDelta { text: text.into() }
}

fn user(text: &str) -> ChatEntry {
    ChatEntry::User {
        text: text.into(),
        context: None,
    }
}

fn assistant(text: &str) -> ChatEntry {
    ChatEntry::Assistant { text: text.into() }
}

#[test]
fn saves_on_the_question_and_at_each_turn_end() {
    let mut r = Recorder::default();
    let saved = r.begin(1, meta(7), "sort my downloads", Some("Downloads"), 200);
    assert_eq!(saved.len(), 1, "the question is saved at once");
    assert_eq!(saved[0].id, 7);
    assert_eq!(saved[0].title, "sort my downloads");
    assert_eq!(saved[0].created_ms, 100);
    assert_eq!(saved[0].cwd, "/work");

    assert!(r.event(1, &delta("Moving"), 300).is_empty(), "not mid-turn");
    let saved = r.event(1, &AiEvent::Done, 400).pop().unwrap();
    assert_eq!(saved.updated_ms, 400);
    assert_eq!(
        saved.entries,
        [
            ChatEntry::User {
                text: "sort my downloads".into(),
                context: Some("Downloads".into()),
            },
            assistant("Moving"),
        ]
    );

    let saved = r.question(1, "thanks", None, 500).unwrap();
    assert_eq!(saved.entries.last(), Some(&user("thanks")));
    assert_eq!(r.record_of(1), Some(7));
}

#[test]
fn events_before_the_question_are_kept() {
    let mut r = Recorder::default();
    r.event(
        1,
        &AiEvent::Started {
            model: "claude-haiku-5-5".into(),
        },
        0,
    );
    r.event(1, &delta("Hi"), 0);
    let saved = r.begin(1, meta(1), "hello", None, 10);
    assert_eq!(saved[0].model, "claude-haiku-5-5");
    assert_eq!(saved[0].entries, [user("hello"), assistant("Hi")]);
}

#[test]
fn a_new_conversation_finishes_the_last_one() {
    let mut r = Recorder::default();
    r.begin(1, meta(1), "first", None, 0);
    r.event(1, &delta("half an ans"), 0);
    // A new question while the old answer was still coming.
    let saved = r.begin(2, meta(2), "second", None, 10);
    let ids: Vec<i64> = saved.iter().map(|s| s.id).collect();
    assert_eq!(ids, [1, 2]);
    assert_eq!(saved[0].entries.last(), Some(&assistant("half an ans")));
    // Late events of the old one are dropped.
    assert!(r.event(1, &AiEvent::Done, 20).is_empty());
    assert_eq!(r.record_of(1), None);
    // An old conversation that was saved and is clean isn't saved again.
    r.event(2, &AiEvent::Done, 30);
    assert!(r.finish(40).is_empty());
    assert_eq!(r.record_of(2), None);
}

#[test]
fn a_newer_conversation_ends_the_older_ones() {
    let mut r = Recorder::default();
    r.begin(1, meta(1), "first", None, 0);
    r.event(1, &delta("half an answer"), 0);
    // Conversation 2 isn't recorded (history was off when it was
    // asked): its first event ends 1, saved with what came.
    let saved = r.event(2, &delta("not kept"), 10);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].entries.last(), Some(&assistant("half an answer")));
    assert_eq!(r.record_of(1), None);
    // 3 ends 2 without saving anything: only one is held at a time.
    assert!(r.event(3, &delta("Hi"), 20).is_empty());
    assert_eq!(r.recordings.len(), 1);
}

#[test]
fn closing_mid_turn_saves_what_came() {
    let mut r = Recorder::default();
    r.begin(3, meta(3), "long story", None, 0);
    r.event(3, &delta("Once upon"), 0);
    let saved = r.finish(50);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].entries.last(), Some(&assistant("Once upon")));
}

#[test]
fn unrecorded_conversations_stay_unrecorded() {
    let mut r = Recorder::default();
    // History was off when this one started: its follow-ups aren't kept.
    assert!(r.question(4, "more", None, 0).is_none());
    // Events for a conversation never asked about are held, not saved.
    assert!(r.event(5, &AiEvent::Done, 0).is_empty());
    assert!(r.question(5, "more", None, 0).is_none());
    assert!(r.finish(0).is_empty());
}

#[test]
fn a_reopened_conversation_carries_on_in_its_record() {
    let mut r = Recorder::default();
    let saved = Transcript::from_saved(vec![user("hi"), assistant("Hello")], "gpt-6".into());
    assert!(r.adopt(9, meta(42), saved, 0).is_empty());
    r.question(9, "and then?", None, 10);
    let record = r.event(9, &AiEvent::Done, 20).pop().unwrap();
    assert_eq!(record.id, 42);
    assert_eq!(record.model, "gpt-6");
    assert_eq!(record.entries.len(), 3);
}

#[test]
fn forgotten_records_are_not_saved_again() {
    let mut r = Recorder::default();
    r.begin(1, meta(1), "secret", None, 0);
    r.forget(|record| record == 1);
    assert!(r.event(1, &AiEvent::Done, 10).is_empty());
    assert!(r.question(1, "more", None, 20).is_none());
    assert!(r.finish(30).is_empty());
}

#[test]
fn claude_starts_over_without_its_folder_or_session() {
    let home = PathBuf::from("/home/me");
    let all = Found {
        folder: true,
        session: true,
    };
    let claude = Resumable::new(ProviderId::Claude, "/gone".into(), Some("s".into()));
    let (plan, note) = resume_plan(claude.clone(), all, home.clone());
    assert_eq!(
        (plan.session(), plan.cwd, note),
        (Some("s".into()), "/gone".into(), None)
    );

    let no_folder = Found {
        folder: false,
        ..all
    };
    let (plan, note) = resume_plan(claude.clone(), no_folder, home.clone());
    assert_eq!((plan.session(), plan.cwd), (None, home.clone()));
    assert!(note.unwrap().contains("/gone"));

    let no_session = Found {
        session: false,
        ..all
    };
    let (plan, note) = resume_plan(claude, no_session, home.clone());
    assert_eq!((plan.session(), plan.cwd), (None, "/gone".into()));
    assert!(note.unwrap().contains("no longer has"));

    let codex = Resumable::new(ProviderId::Chatgpt, "/gone".into(), Some("t".into()));
    let (plan, note) = resume_plan(codex, no_folder, home.clone());
    assert_eq!(
        (plan.session(), plan.cwd, note),
        (Some("t".into()), home, None)
    );
}
