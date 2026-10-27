//! Two live conversations side by side (comparing Claude and ChatGPT),
//! with the stand-in CLIs from `tests.rs`.

use std::collections::HashMap;

use super::tests::{
    claude_setup, codex_setup, counting_claude, fake_codex, received, script, starts, SIGNED_IN,
};
use super::*;
use crate::ai::launch::Launch;
use std::path::Path;
use tokio::sync::mpsc;

type Events = mpsc::UnboundedReceiver<(u32, AiEvent)>;

/// A stand-in `claude` that answers every message with a recorded reply
/// (and an interrupt with Claude Code's error result), noting its
/// arguments in `claude-args.log`.
fn replying_claude(dir: &Path) -> Launch {
    let fixture = format!(
        "{}/tests/fixtures/claude/text_reply.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let log = dir.join("claude-args.log");
    script(
        dir,
        "claude",
        &format!(
            r#"echo "$*" >> '{log}'
while IFS= read -r line; do
  case "$line" in
    *interrupt*) echo '{{"type":"result","subtype":"error_during_execution","is_error":true}}' ;;
    *SLOW*) ;;
    *) cat '{fixture}' ;;
  esac
done
"#,
            log = log.display()
        ),
    )
}

fn ends_turn(event: &AiEvent) -> bool {
    matches!(
        event,
        AiEvent::Done | AiEvent::Error { .. } | AiEvent::Stopped
    )
}

/// Each conversation's events until every one of `ids` has ended its turn.
async fn until_all_done(rx: &mut Events, ids: &[u32]) -> HashMap<u32, Vec<AiEvent>> {
    let mut events: HashMap<u32, Vec<AiEvent>> = HashMap::new();
    let finished = |events: &HashMap<u32, Vec<AiEvent>>| {
        ids.iter()
            .all(|id| events.get(id).and_then(|e| e.last()).is_some_and(ends_turn))
    };
    while !finished(&events) {
        let Ok(Some((id, event))) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await
        else {
            break;
        };
        events.entry(id).or_default().push(event);
    }
    events
}

/// For each of `ids`, its first event that `pick` accepts (other events
/// are passed over).
async fn first_of_each<T>(
    rx: &mut Events,
    ids: &[u32],
    pick: impl Fn(&AiEvent) -> Option<T>,
) -> Vec<T> {
    let mut found: Vec<Option<T>> = ids.iter().map(|_| None).collect();
    while found.iter().any(Option::is_none) {
        let (from, event) = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("an event in time")
            .expect("the chat is still sending");
        if let Some(at) = ids.iter().position(|&id| id == from) {
            if found[at].is_none() {
                found[at] = pick(&event);
            }
        }
    }
    found.into_iter().flatten().collect()
}

/// Waits for an event of conversation `id` that `pick` accepts.
async fn wait_for<T>(rx: &mut Events, id: u32, pick: impl Fn(&AiEvent) -> Option<T>) -> T {
    let mut found = first_of_each(rx, &[id], pick).await;
    found.remove(0)
}

fn both(
    claude: &Launch,
    codex: &Launch,
    dir: &Path,
    claude_text: &str,
    codex_text: &str,
) -> Vec<(Message, Setup)> {
    vec![
        (claude_text.into(), claude_setup(claude, dir)),
        (codex_text.into(), codex_setup(codex, dir)),
    ]
}

#[tokio::test]
async fn compares_two_conversations_side_by_side() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let claude = replying_claude(dir.path());
    let codex = fake_codex(dir.path(), SIGNED_IN);

    let mut told = Vec::new();
    let ids = chat
        .compare(both(&claude, &codex, dir.path(), "hi", "hi"), |ids| {
            told = ids.to_vec()
        })
        .await;
    let ids: Vec<u32> = ids.into_iter().map(Result::unwrap).collect();
    assert_eq!(told, ids, "told before they start");
    assert_ne!(ids[0], ids[1]);
    assert_eq!(
        chat.conversations().await,
        [(ids[0], ProviderId::Claude), (ids[1], ProviderId::Chatgpt)]
    );
    assert_eq!(chat.provider().await, Some(ProviderId::Claude));

    let events = until_all_done(&mut rx, &ids).await;
    assert!(events[&ids[0]].contains(&AiEvent::TextDelta {
        text: "hello from the launcher".into()
    }));
    assert_eq!(events[&ids[0]].last(), Some(&AiEvent::Done));
    assert_eq!(
        events[&ids[1]],
        [
            AiEvent::Started {
                model: "gpt-test".into()
            },
            AiEvent::TextDelta {
                text: "hello from codex".into()
            },
            AiEvent::Done,
        ]
    );
    for id in &ids {
        assert!(chat.resumable(*id).await.is_some(), "both are live");
    }

    // A follow-up to each side goes to its own process.
    for &id in &ids {
        let sent = chat
            .follow_up_in(id, "more", |_| unreachable!("still running"))
            .await
            .unwrap();
        assert_eq!(sent, id);
    }
    let events = until_all_done(&mut rx, &ids).await;
    assert!(ids
        .iter()
        .all(|id| events[id].last() == Some(&AiEvent::Done)));
    let codex_sent = received(dir.path());
    assert_eq!(codex_sent.matches("\"method\":\"turn/start\"").count(), 2);
    assert_eq!(codex_sent.matches("\"method\":\"thread/start\"").count(), 1);
}

#[tokio::test]
async fn approvals_go_back_to_the_side_that_asked() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let fixture = format!(
        "{}/tests/fixtures/claude/write_with_permission.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let claude = script(
        dir.path(),
        "claude",
        &format!("cat '{fixture}'\ncat > /dev/null\n"),
    );
    let codex = fake_codex(dir.path(), SIGNED_IN);
    let ids: Vec<u32> = chat
        .compare(
            both(&claude, &codex, dir.path(), "make a file", "RUN it"),
            |_| {},
        )
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();

    let asked = |event: &AiEvent| match event {
        AiEvent::ApprovalRequested { id, .. } => Some(id.clone()),
        _ => None,
    };
    let mut asks = first_of_each(&mut rx, &ids, asked).await;
    let (claude_asks, codex_asks) = (asks.remove(0), asks.remove(0));

    // ChatGPT's first: the answer reaches Codex, whichever side is first.
    chat.respond(&codex_asks, true).await.unwrap();
    let resolved = wait_for(&mut rx, ids[1], |e| {
        matches!(e, AiEvent::ApprovalResolved { .. }).then(|| e.clone())
    })
    .await;
    assert_eq!(
        resolved,
        AiEvent::ApprovalResolved {
            id: codex_asks.clone(),
            allowed: true
        }
    );
    // Codex finishes its turn once it has the answer.
    wait_for(&mut rx, ids[1], |e| (e == &AiEvent::Done).then_some(())).await;
    assert!(received(dir.path()).contains(r#"{"id":0,"result":{"decision":"accept"}}"#));

    chat.respond(&claude_asks, false).await.unwrap();
    let resolved = wait_for(&mut rx, ids[0], |e| {
        matches!(e, AiEvent::ApprovalResolved { .. }).then(|| e.clone())
    })
    .await;
    assert_eq!(
        resolved,
        AiEvent::ApprovalResolved {
            id: claude_asks.clone(),
            allowed: false
        }
    );
    assert!(matches!(
        chat.respond(&claude_asks, true).await,
        Err(AiError::UnknownApproval)
    ));
}

#[tokio::test]
async fn stop_stops_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let claude = replying_claude(dir.path());
    let codex = fake_codex(dir.path(), SIGNED_IN);
    let ids: Vec<u32> = chat
        .compare(
            both(&claude, &codex, dir.path(), "SLOW story", "SLOW story"),
            |_| {},
        )
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    // Codex can only be interrupted once its turn has an id.
    wait_for(&mut rx, ids[1], |e| {
        matches!(e, AiEvent::Started { .. }).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    chat.stop().await;
    let events = until_all_done(&mut rx, &ids).await;
    for id in &ids {
        assert_eq!(events[id].last(), Some(&AiEvent::Stopped), "{events:?}");
    }
    assert_eq!(chat.conversations().await.len(), 2, "stopping keeps them");
}

#[tokio::test]
async fn keeping_one_side_ends_the_other() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let claude = replying_claude(dir.path());
    let codex = fake_codex(dir.path(), SIGNED_IN);
    let ids: Vec<u32> = chat
        .compare(both(&claude, &codex, dir.path(), "hi", "hi"), |_| {})
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    until_all_done(&mut rx, &ids).await;

    assert!(matches!(
        chat.keep(ids[1] + 1).await,
        Err(AiError::NoConversation)
    ));
    chat.keep(ids[1]).await.unwrap();
    assert_eq!(chat.conversations().await, [(ids[1], ProviderId::Chatgpt)]);
    assert!(chat.resumable(ids[0]).await.is_none());
    // The single-chat API now carries on with ChatGPT.
    assert_eq!(chat.provider().await, Some(ProviderId::Chatgpt));
    let id = chat
        .follow_up("more", |_| unreachable!("still running"))
        .await
        .unwrap();
    assert_eq!(id, ids[1]);
    let events = until_all_done(&mut rx, &[id]).await;
    assert_eq!(events[&id].last(), Some(&AiEvent::Done));

    // Keeping Claude instead in a new comparison ends Codex, which exits.
    let ids: Vec<u32> = chat
        .compare(both(&claude, &codex, dir.path(), "hi", "hi"), |_| {})
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    until_all_done(&mut rx, &ids).await;
    let codex_session = match chat.current.lock().await[1].session.as_ref() {
        Some(Session::Codex(session)) => Arc::downgrade(session),
        _ => panic!("a Codex session"),
    };
    chat.keep(ids[0]).await.unwrap();
    for _ in 0..50 {
        if codex_session.upgrade().is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the other side's process is still running");
}

#[tokio::test]
async fn both_sides_resume_after_an_idle_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let claude = replying_claude(dir.path());
    let codex = fake_codex(dir.path(), SIGNED_IN);
    let ids: Vec<u32> = chat
        .compare(both(&claude, &codex, dir.path(), "hi", "hi"), |_| {})
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    until_all_done(&mut rx, &ids).await;
    chat.shut_down_if_idle(Duration::ZERO).await;
    assert!(chat
        .current
        .lock()
        .await
        .iter()
        .all(|c| c.session.is_none()));

    for (&id, provider) in ids.iter().zip([ProviderId::Claude, ProviderId::Chatgpt]) {
        chat.follow_up_in(id, "again", |asked| {
            assert_eq!(asked, provider);
            Ok(match asked {
                ProviderId::Claude => claude_setup(&claude, dir.path()),
                ProviderId::Chatgpt => codex_setup(&codex, dir.path()),
            })
        })
        .await
        .unwrap();
    }
    let events = until_all_done(&mut rx, &ids).await;
    assert!(ids
        .iter()
        .all(|id| events[id].last() == Some(&AiEvent::Done)));
    let args = std::fs::read_to_string(dir.path().join("claude-args.log")).unwrap();
    assert!(
        args.contains("--resume 00000000-0000-4000-8000-000000000000"),
        "{args}"
    );
    assert!(received(dir.path()).contains(r#""method":"thread/resume""#));
}

#[tokio::test]
async fn warm_spares_for_both_sides_are_claimed() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let claude = counting_claude(dir.path());
    let codex = fake_codex(dir.path(), SIGNED_IN);
    let setups = || {
        vec![
            claude_setup(&claude, dir.path()),
            codex_setup(&codex, dir.path()),
        ]
    };
    chat.warm_all(setups()).await.unwrap();
    // Warming again keeps both; a single warm-up keeps only its own.
    chat.warm_all(setups()).await.unwrap();
    assert_eq!(chat.spare.lock().await.len(), 2);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(rx.try_recv().is_err(), "spares say nothing until claimed");

    let ids: Vec<u32> = chat
        .compare(both(&claude, &codex, dir.path(), "hi", "hi"), |_| {})
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    let events = until_all_done(&mut rx, &ids).await;
    assert!(ids
        .iter()
        .all(|id| events[id].last() == Some(&AiEvent::Done)));
    assert_eq!(starts(dir.path()), 1, "no second Claude Code");
    assert_eq!(
        received(dir.path())
            .matches("\"method\":\"initialize\"")
            .count(),
        1,
        "no second Codex"
    );
    assert!(chat.spare.lock().await.is_empty());

    chat.warm_all(setups()).await.unwrap();
    chat.warm(codex_setup(&codex, dir.path())).await.unwrap();
    assert_eq!(
        chat.spare.lock().await.len(),
        1,
        "the Claude spare is closed"
    );
    chat.drop_stale_spare(Duration::ZERO).await;
    assert!(chat.spare.lock().await.is_empty());
}

#[tokio::test]
async fn a_side_that_cannot_start_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let claude = Launch::Node {
        node: dir.path().join("no-such-node"),
        cli: dir.path().join("claude"),
    };
    let codex = fake_codex(dir.path(), SIGNED_IN);
    let results = chat
        .compare(both(&claude, &codex, dir.path(), "hi", "hi"), |_| {})
        .await;
    assert!(matches!(results[0], Err(AiError::Spawn(..))), "{results:?}");
    let codex_id = *results[1].as_ref().unwrap();
    assert_eq!(
        chat.conversations().await,
        [(codex_id, ProviderId::Chatgpt)]
    );
    let events = until_all_done(&mut rx, &[codex_id]).await;
    assert_eq!(events[&codex_id].last(), Some(&AiEvent::Done));
}

#[tokio::test]
async fn closing_or_asking_one_ai_ends_the_comparison() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let claude = replying_claude(dir.path());
    let codex = fake_codex(dir.path(), SIGNED_IN);
    let ids: Vec<u32> = chat
        .compare(both(&claude, &codex, dir.path(), "hi", "hi"), |_| {})
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    until_all_done(&mut rx, &ids).await;

    let id = chat
        .start("just you", claude_setup(&claude, dir.path()))
        .await
        .unwrap();
    assert_eq!(chat.conversations().await, [(id, ProviderId::Claude)]);
    until_all_done(&mut rx, &[id]).await;

    let ids: Vec<u32> = chat
        .compare(both(&claude, &codex, dir.path(), "hi", "hi"), |_| {})
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert!(chat.resumable(id).await.is_none(), "the old chat ended");
    chat.close().await;
    assert!(chat.conversations().await.is_empty());
    assert!(matches!(
        chat.follow_up_in(ids[0], "more", |_| unreachable!()).await,
        Err(AiError::NoConversation)
    ));
}
