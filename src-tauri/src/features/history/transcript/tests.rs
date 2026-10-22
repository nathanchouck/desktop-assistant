use super::*;

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
fn builds_what_the_chat_pane_shows() {
    let mut t = Transcript::default();
    assert!(!t.apply(&AiEvent::Started {
        model: "claude-sonnet-5-5".into()
    }));
    t.user_message("sort my downloads", Some("Downloads · 2 selected"));
    t.apply(&delta("Sure, "));
    t.apply(&delta("moving them."));
    t.apply(&AiEvent::ToolStarted {
        id: "t1".into(),
        name: "Bash".into(),
        summary: "mv a b".into(),
    });
    t.apply(&AiEvent::ApprovalRequested {
        id: "r1".into(),
        tool: "Bash".into(),
        summary: "mv a b".into(),
        detail: "mv a b".into(),
    });
    t.apply(&AiEvent::ApprovalResolved {
        id: "r1".into(),
        allowed: true,
    });
    t.apply(&AiEvent::ToolFinished {
        id: "t1".into(),
        ok: true,
    });
    t.apply(&delta("Done."));
    assert!(t.apply(&AiEvent::Done));
    assert_eq!(t.model, "claude-sonnet-5-5");
    assert_eq!(
        t.entries,
        [
            ChatEntry::User {
                text: "sort my downloads".into(),
                context: Some("Downloads · 2 selected".into()),
            },
            assistant("Sure, moving them."),
            ChatEntry::Tool {
                id: "t1".into(),
                name: "Bash".into(),
                summary: "mv a b".into(),
                state: ToolState::Ok,
            },
            ChatEntry::Approval {
                id: "r1".into(),
                tool: "Bash".into(),
                summary: "mv a b".into(),
                detail: "mv a b".into(),
                state: ApprovalState::Allowed,
            },
            assistant("Done."),
        ]
    );
    assert_eq!(message_count(&t.entries), 2);
}

#[test]
fn stops_and_errors_end_a_turn_with_a_line() {
    let mut t = Transcript::default();
    t.user_message("long story", None);
    t.apply(&delta("Once"));
    assert!(t.apply(&AiEvent::Stopped));
    t.user_message("again", None);
    assert!(t.apply(&AiEvent::Error {
        message: "Claude Code exited".into()
    }));
    assert_eq!(
        t.entries,
        [
            user("long story"),
            assistant("Once"),
            ChatEntry::Note {
                text: "Stopped".into()
            },
            user("again"),
            ChatEntry::Error {
                text: "Claude Code exited".into()
            },
        ]
    );
}

#[test]
fn a_reply_that_beats_its_question_goes_after_it() {
    let mut t = Transcript::default();
    t.user_message("hi", None);
    t.apply(&delta("Hello"));
    t.apply(&AiEvent::Done);
    // The follow-up's answer arrives before the follow-up is recorded.
    t.apply(&delta("Sure"));
    t.user_message("more", None);
    t.apply(&delta(" thing"));
    assert_eq!(
        t.entries,
        [
            user("hi"),
            assistant("Hello"),
            user("more"),
            assistant("Sure thing"),
        ]
    );

    assert!(t.running());

    // Even an error from start-up, before the first question: that
    // question's turn is over once it's recorded.
    let mut t = Transcript::default();
    t.apply(&AiEvent::Error {
        message: "signed out".into(),
    });
    t.user_message("hi", None);
    assert_eq!(t.entries[0], user("hi"));
    assert!(!t.running());
    t.user_message("again", None);
    assert!(t.running(), "the next question runs as usual");
}

#[test]
fn saved_conversations_settle_what_was_cut_off() {
    let entries = vec![
        ChatEntry::Tool {
            id: "1".into(),
            name: "Read".into(),
            summary: String::new(),
            state: ToolState::Running,
        },
        ChatEntry::Approval {
            id: "2".into(),
            tool: "Bash".into(),
            summary: String::new(),
            detail: "rm x".into(),
            state: ApprovalState::Pending,
        },
    ];
    let t = Transcript::from_saved(entries, "gpt-6".into());
    assert!(matches!(
        t.entries[0],
        ChatEntry::Tool {
            state: ToolState::Failed,
            ..
        }
    ));
    assert!(matches!(
        t.entries[1],
        ChatEntry::Approval {
            state: ApprovalState::Denied,
            ..
        }
    ));
}

#[test]
fn counts_questions_and_answered_turns() {
    let tool = ChatEntry::Tool {
        id: "1".into(),
        name: "Read".into(),
        summary: String::new(),
        state: ToolState::Ok,
    };
    let entries = [
        user("a"),
        assistant("checking"),
        tool,
        assistant("done"),
        user("b"),
        ChatEntry::Error { text: "x".into() },
    ];
    assert_eq!(message_count(&entries), 3);
    assert_eq!(message_count(&[]), 0);
}

#[test]
fn search_text_is_lowercase_and_capped() {
    let entries = [user("Sort My Downloads"), assistant("Moved **3** files")];
    assert_eq!(
        search_text(&entries),
        "sort my downloads\nmoved **3** files\n"
    );
    let long = [assistant(&"é".repeat(SEARCH_CHARS))];
    let text = search_text(&long);
    assert!(text.len() <= SEARCH_CHARS && text.starts_with('é'));
}

#[test]
fn titles_are_one_short_line() {
    assert_eq!(title("  what's\n the   weather? "), "what's the weather?");
    let long = title(&"word ".repeat(100));
    assert_eq!(long.chars().count(), TITLE_CHARS);
    assert!(long.ends_with('…'));
}

#[test]
fn entries_round_trip_through_json() {
    let entries = vec![
        ChatEntry::User {
            text: "hi".into(),
            context: Some("Desktop".into()),
        },
        assistant("**hello**"),
        ChatEntry::Approval {
            id: "r1".into(),
            tool: "Write".into(),
            summary: "notes.txt".into(),
            detail: "text".into(),
            state: ApprovalState::Denied,
        },
        ChatEntry::Note {
            text: "Stopped".into(),
        },
    ];
    let json = to_json(&entries);
    assert!(json.contains(r#""kind":"user""#) && json.contains(r#""state":"denied""#));
    assert_eq!(from_json(&json), entries);
}

#[test]
fn unknown_lines_are_skipped_not_fatal() {
    let json = r#"[{"kind":"user","text":"hi","context":null},{"kind":"hologram"},{"kind":"note","text":"Stopped"}]"#;
    assert_eq!(
        from_json(json),
        [
            user("hi"),
            ChatEntry::Note {
                text: "Stopped".into()
            }
        ]
    );
    assert!(from_json("not json").is_empty());
}
