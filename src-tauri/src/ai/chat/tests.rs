use super::*;
use crate::ai::{Approvals, BillingMode};
use tokio::sync::mpsc;

/// A stand-in CLI. Run by `sh` rather than exec'd itself: exec'ing a file
/// just written fails now and then ("text file busy") while another test
/// thread is starting a process.
fn script(dir: &Path, name: &str, body: &str) -> Launch {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    Launch::Node {
        node: PathBuf::from("/bin/sh"),
        cli: path,
    }
}

/// A stand-in `claude` that ignores its arguments, prints a recorded
/// session, then waits on stdin like the real one.
fn fake_claude(dir: &Path, fixture: &str) -> Launch {
    let fixture = format!(
        "{}/tests/fixtures/claude/{fixture}.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    script(
        dir,
        "claude",
        &format!("cat '{fixture}'\ncat > /dev/null\n"),
    )
}

fn options(dir: &Path) -> claude::Options {
    claude::Options {
        model: "haiku".into(),
        approvals: Approvals::Ask,
        billing: BillingMode::Subscription,
        cwd: dir.to_path_buf(),
        resume: None,
        system_prompt: String::new(),
        mcp_config: None,
        settings: None,
    }
}

fn claude_setup(launch: &Launch, dir: &Path) -> Setup {
    Setup::Claude(launch.clone(), options(dir))
}

async fn collect_until_done(rx: &mut mpsc::UnboundedReceiver<(u32, AiEvent)>) -> Vec<AiEvent> {
    let mut events = Vec::new();
    while let Ok(Some((_, event))) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await
    {
        let done = matches!(
            event,
            AiEvent::Done | AiEvent::Error { .. } | AiEvent::Stopped
        );
        events.push(event);
        if done {
            break;
        }
    }
    events
}

#[tokio::test]
async fn streams_a_conversation_and_answers_approvals() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_claude(dir.path(), "write_with_permission");

    let id = chat
        .start("make a file", claude_setup(&launch, dir.path()))
        .await
        .unwrap();
    assert_eq!(chat.provider().await, Some(ProviderId::Claude));
    let events = collect_until_done(&mut rx).await;
    let approval = events
        .iter()
        .find_map(|e| match e {
            AiEvent::ApprovalRequested { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("approval requested");
    assert!(matches!(events.last(), Some(AiEvent::Done)));

    chat.respond(&approval, true).await.unwrap();
    let (conv, resolved) = rx.recv().await.unwrap();
    assert_eq!(conv, id);
    assert_eq!(
        resolved,
        AiEvent::ApprovalResolved {
            id: approval.clone(),
            allowed: true
        }
    );
    assert!(matches!(
        chat.respond(&approval, true).await,
        Err(AiError::UnknownApproval)
    ));

    chat.close().await;
    assert!(matches!(
        chat.follow_up("more", |_| Ok(claude_setup(&launch, dir.path())))
            .await,
        Err(AiError::NoConversation)
    ));
}

#[tokio::test]
async fn resumes_after_an_idle_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_claude(dir.path(), "text_reply");

    let id = chat
        .start("hi", claude_setup(&launch, dir.path()))
        .await
        .unwrap();
    collect_until_done(&mut rx).await;
    chat.shut_down_if_idle(Duration::ZERO).await;

    // Follow-ups pass today's options; the restart must still use the
    // conversation's own folder or Claude Code can't find the session.
    let elsewhere = tempfile::tempdir().unwrap();
    let again = chat
        .follow_up("again", |provider| {
            assert_eq!(provider, ProviderId::Claude);
            Ok(claude_setup(&launch, elsewhere.path()))
        })
        .await
        .unwrap();
    assert_eq!(again, id, "same conversation");
    let cwd = chat.current.lock().await.as_ref().unwrap().cwd.clone();
    assert_eq!(cwd, dir.path());
    let events = collect_until_done(&mut rx).await;
    assert!(events.contains(&AiEvent::TextDelta {
        text: "hello from the launcher".into()
    }));
}

#[tokio::test]
async fn reports_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let launch = script(
        dir.path(),
        "claude",
        "echo 'boom: not signed in' >&2\nexit 1\n",
    );
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    // The write may race the exit; either way the crash must surface.
    let _ = chat.start("hi", claude_setup(&launch, dir.path())).await;
    let events = collect_until_done(&mut rx).await;
    assert!(matches!(events.last(), Some(AiEvent::Error { message }) if message.contains("boom")));
}

#[tokio::test]
async fn a_stopped_turn_reads_as_stopped() {
    let dir = tempfile::tempdir().unwrap();
    // Answers the interrupt the way Claude Code does: an error result.
    let launch = script(
        dir.path(),
        "claude",
        "while read -r line; do\n  case \"$line\" in\n    *interrupt*) echo '{\"type\":\"result\",\"subtype\":\"error_during_execution\",\"is_error\":true}' ;;\n  esac\ndone\n",
    );
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    chat.start("long story", claude_setup(&launch, dir.path()))
        .await
        .unwrap();
    chat.stop().await;
    let events = collect_until_done(&mut rx).await;
    assert_eq!(events.last(), Some(&AiEvent::Stopped));
}

/// A stand-in `claude` that notes each start in `starts.log`.
fn counting_claude(dir: &Path) -> Launch {
    let fixture = format!(
        "{}/tests/fixtures/claude/text_reply.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let log = dir.join("starts.log");
    script(
        dir,
        "claude",
        &format!(
            "echo started >> '{}'\ncat '{fixture}'\ncat > /dev/null\n",
            log.display()
        ),
    )
}

fn starts(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("starts.log"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

#[tokio::test]
async fn a_question_uses_the_warm_process() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = counting_claude(dir.path());
    chat.warm(claude_setup(&launch, dir.path())).await.unwrap();
    // Warming again with the same options keeps the one spare.
    chat.warm(claude_setup(&launch, dir.path())).await.unwrap();
    // Let the spare print before it's claimed: that output is held.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        rx.try_recv().is_err(),
        "nothing sent before a conversation claims it"
    );

    let id = chat
        .start("hi", claude_setup(&launch, dir.path()))
        .await
        .unwrap();
    let (conversation, first) = rx.recv().await.unwrap();
    assert_eq!(conversation, id);
    assert!(matches!(first, AiEvent::Started { .. }), "{first:?}");
    let events = collect_until_done(&mut rx).await;
    assert!(matches!(events.last(), Some(AiEvent::Done)));
    assert_eq!(starts(dir.path()), 1, "no second process");
}

#[tokio::test]
async fn a_spare_for_another_folder_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = counting_claude(dir.path());
    chat.warm(claude_setup(&launch, elsewhere.path()))
        .await
        .unwrap();
    // Let the spare start before it's passed over.
    tokio::time::sleep(Duration::from_millis(300)).await;
    chat.start("hi", claude_setup(&launch, dir.path()))
        .await
        .unwrap();
    collect_until_done(&mut rx).await;
    assert_eq!(starts(dir.path()), 2);
    assert!(chat.spare.lock().await.is_none());

    chat.warm(claude_setup(&launch, dir.path())).await.unwrap();
    chat.drop_stale_spare(Duration::ZERO).await;
    assert!(chat.spare.lock().await.is_none(), "an old spare is closed");
}

// --- Codex -----------------------------------------------------------------

/// A stand-in `codex app-server`: answers each request by method (echoing
/// its id), logs what it receives to `in.log` and its arguments to
/// `args.log`. "RUN" asks to approve a command; "SLOW" waits for an
/// interrupt. `auth` is what `getAuthStatus` reports. With a `gone` file
/// in `dir`, resuming fails like a deleted thread.
fn fake_codex(dir: &Path, auth: &str) -> Launch {
    let log = dir.join("in.log");
    let body = format!(
        r#"echo "$*" >> '{args}'
env | grep -q '^OPENAI_API_KEY=' && echo 'key leaked' >> '{args}'
reply() {{ printf '{{"id":%s,"result":%s}}\n' "$1" "$2"; }}
note() {{ printf '{{"method":"%s","params":%s}}\n' "$1" "$2"; }}
done_turn() {{ note turn/completed "{{\"threadId\":\"t1\",\"turn\":{{\"id\":\"u1\",\"status\":\"$1\",\"error\":null}}}}"; }}
while IFS= read -r line; do
  echo "$line" >> '{log}'
  id=$(printf '%s' "$line" | sed -n 's/^{{"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) reply "$id" '{{"userAgent":"desktop_assistant/0.163.0 (test)"}}' ;;
    *'"method":"getAuthStatus"'*) reply "$id" '{auth}' ;;
    *'"method":"account/login/start"'*) reply "$id" '{{"type":"apiKey"}}' ;;
    *'"method":"thread/resume"'*)
      if [ -e '{gone}' ]; then
        printf '{{"id":%s,"error":{{"code":-32600,"message":"no rollout found for thread id t1"}}}}\n' "$id"
      else
        reply "$id" '{{"thread":{{"id":"t1"}},"model":"gpt-test"}}'
      fi ;;
    *'"method":"thread/start"'*) reply "$id" '{{"thread":{{"id":"t1"}},"model":"gpt-test"}}' ;;
    *'"method":"turn/interrupt"'*) reply "$id" '{{}}'; done_turn interrupted ;;
    *'"method":"turn/start"'*)
      reply "$id" '{{"turn":{{"id":"u1","status":"inProgress"}}}}'
      note turn/started '{{"threadId":"t1","turn":{{"id":"u1"}}}}'
      case "$line" in
        *SLOW*) ;;
        *RUN*)
          note item/started '{{"threadId":"t1","item":{{"type":"commandExecution","id":"c1","command":"touch x","commandActions":[{{"type":"unknown","command":"touch x"}}],"cwd":"/w","status":"inProgress"}}}}'
          printf '%s\n' '{{"id":0,"method":"item/commandExecution/requestApproval","params":{{"threadId":"t1","itemId":"c1","command":"touch x","cwd":"/w"}}}}' ;;
        *)
          note item/agentMessage/delta '{{"threadId":"t1","itemId":"m1","delta":"hello from codex"}}'
          done_turn completed ;;
      esac ;;
    *'"result":{{"decision":"accept"}}'*)
      note serverRequest/resolved '{{"threadId":"t1","requestId":0}}'
      note item/completed '{{"threadId":"t1","item":{{"type":"commandExecution","id":"c1","status":"completed","exitCode":0}}}}'
      done_turn completed ;;
  esac
done
"#,
        args = dir.join("args.log").display(),
        log = log.display(),
        gone = dir.join("gone").display(),
    );
    script(dir, "codex", &body)
}

fn codex_options(dir: &Path) -> codex::Options {
    codex::Options {
        model: String::new(),
        approvals: Approvals::Ask,
        billing: BillingMode::Subscription,
        api_key: None,
        cwd: dir.to_path_buf(),
        home: dir.to_path_buf(),
        resume: None,
        instructions: "be brief".into(),
        mcp: None,
        screen_tools: true,
        windows_sandbox: false,
    }
}

fn codex_setup(launch: &Launch, dir: &Path) -> Setup {
    Setup::Codex(launch.clone(), codex_options(dir))
}

const SIGNED_IN: &str = r#"{"authMethod":"chatgpt","requiresOpenaiAuth":true}"#;

fn received(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("in.log")).unwrap_or_default()
}

#[tokio::test]
async fn codex_answers_a_question() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_codex(dir.path(), SIGNED_IN);

    let id = chat
        .start("hi", codex_setup(&launch, dir.path()))
        .await
        .unwrap();
    assert_eq!(chat.provider().await, Some(ProviderId::Chatgpt));
    let events = collect_until_done(&mut rx).await;
    assert_eq!(
        events,
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
    let sent = received(dir.path());
    let order: Vec<&str> = [
        "initialize",
        "initialized",
        "getAuthStatus",
        "thread/start",
        "turn/start",
    ]
    .into_iter()
    .filter(|m| sent.contains(&format!("\"method\":\"{m}\"")))
    .collect();
    assert_eq!(order.len(), 5, "{sent}");
    assert!(sent.find("thread/start") < sent.find("turn/start"));
    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_eq!(args.trim(), "app-server");

    // A follow-up goes to the same process and thread.
    assert_eq!(
        chat.follow_up("more", |_| unreachable!("the process is alive"))
            .await
            .unwrap(),
        id
    );
    let events = collect_until_done(&mut rx).await;
    assert_eq!(events.last(), Some(&AiEvent::Done));
    assert_eq!(received(dir.path()).matches("thread/start").count(), 1);
}

#[tokio::test]
async fn codex_asks_before_running_a_command() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_codex(dir.path(), SIGNED_IN);
    chat.start("RUN it", codex_setup(&launch, dir.path()))
        .await
        .unwrap();
    let mut approval = None;
    while approval.is_none() {
        let (_, event) = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap()
            .unwrap();
        if let AiEvent::ApprovalRequested { id, summary, .. } = event {
            assert_eq!(summary, "touch x");
            approval = Some(id);
        }
    }
    let approval = approval.unwrap();
    chat.respond(&approval, true).await.unwrap();
    let events = collect_until_done(&mut rx).await;
    assert_eq!(
        events[0],
        AiEvent::ApprovalResolved {
            id: approval.clone(),
            allowed: true
        }
    );
    assert!(events.contains(&AiEvent::ToolFinished {
        id: "c1".into(),
        ok: true
    }));
    assert_eq!(events.last(), Some(&AiEvent::Done));
    assert!(received(dir.path()).contains(r#"{"id":0,"result":{"decision":"accept"}}"#));
    // Codex's "resolved" after our answer doesn't add a second verdict.
    assert!(!events[1..]
        .iter()
        .any(|e| matches!(e, AiEvent::ApprovalResolved { .. })));
}

#[tokio::test]
async fn codex_stops_a_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_codex(dir.path(), SIGNED_IN);
    chat.start("SLOW story", codex_setup(&launch, dir.path()))
        .await
        .unwrap();
    // Wait until the turn is running, then stop it.
    let (_, started) = rx.recv().await.unwrap();
    assert!(matches!(started, AiEvent::Started { .. }));
    tokio::time::sleep(Duration::from_millis(200)).await;
    chat.stop().await;
    let events = collect_until_done(&mut rx).await;
    assert_eq!(events.last(), Some(&AiEvent::Stopped));
    assert!(received(dir.path()).contains(r#""method":"turn/interrupt""#));
}

#[tokio::test]
async fn codex_resumes_its_thread_after_an_idle_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_codex(dir.path(), SIGNED_IN);
    let id = chat
        .start("hi", codex_setup(&launch, dir.path()))
        .await
        .unwrap();
    collect_until_done(&mut rx).await;
    chat.shut_down_if_idle(Duration::ZERO).await;

    let again = chat
        .follow_up("again", |provider| {
            assert_eq!(provider, ProviderId::Chatgpt);
            Ok(codex_setup(&launch, dir.path()))
        })
        .await
        .unwrap();
    assert_eq!(again, id);
    let events = collect_until_done(&mut rx).await;
    assert_eq!(events.last(), Some(&AiEvent::Done));
    let sent = received(dir.path());
    assert!(
        sent.contains(r#""method":"thread/resume","params":{"#)
            && sent.contains(r#""threadId":"t1""#),
        "{sent}"
    );
}

#[tokio::test]
async fn codex_starts_fresh_when_the_thread_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_codex(dir.path(), SIGNED_IN);
    chat.start("hi", codex_setup(&launch, dir.path()))
        .await
        .unwrap();
    collect_until_done(&mut rx).await;
    chat.shut_down_if_idle(Duration::ZERO).await;
    std::fs::write(dir.path().join("gone"), b"").unwrap();

    chat.follow_up("again", |_| Ok(codex_setup(&launch, dir.path())))
        .await
        .unwrap();
    let events = collect_until_done(&mut rx).await;
    assert!(
        matches!(&events[0], AiEvent::TextDelta { text } if text.contains("starts fresh")),
        "{events:?}"
    );
    assert_eq!(events.last(), Some(&AiEvent::Done));
    let sent = received(dir.path());
    assert!(sent.contains("thread/resume"));
    assert_eq!(sent.matches("\"method\":\"thread/start\"").count(), 2);
}

#[tokio::test]
async fn codex_signed_out_says_how_to_sign_in() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_codex(
        dir.path(),
        r#"{"authMethod":null,"requiresOpenaiAuth":true}"#,
    );
    chat.start("hi", codex_setup(&launch, dir.path()))
        .await
        .unwrap();
    let events = collect_until_done(&mut rx).await;
    assert!(
        matches!(events.last(), Some(AiEvent::Error { message }) if message.contains("Sign In to ChatGPT")),
        "{events:?}"
    );
    assert!(!received(dir.path()).contains("turn/start"));
}

#[tokio::test]
async fn codex_warm_spare_opens_no_thread_until_claimed() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_codex(dir.path(), SIGNED_IN);
    chat.warm(codex_setup(&launch, dir.path())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let sent = received(dir.path());
    assert!(sent.contains("getAuthStatus"), "started up ahead: {sent}");
    assert!(
        !sent.contains("thread/start"),
        "no empty thread left behind"
    );

    // A Claude question doesn't take a Codex spare.
    assert!(chat
        .take_spare(&claude_setup(&launch, dir.path()))
        .await
        .is_none());

    chat.warm(codex_setup(&launch, dir.path())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    chat.start("hi", codex_setup(&launch, dir.path()))
        .await
        .unwrap();
    let events = collect_until_done(&mut rx).await;
    assert_eq!(events.last(), Some(&AiEvent::Done));
}

#[tokio::test]
async fn codex_api_key_goes_over_stdin_only() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat = Chat::new(tx);
    let launch = fake_codex(dir.path(), SIGNED_IN);
    let setup = Setup::Codex(
        launch.clone(),
        codex::Options {
            billing: BillingMode::ApiKey,
            api_key: Some(codex::ApiKey::new("sk-test-launcher")),
            ..codex_options(dir.path())
        },
    );
    chat.start("hi", setup).await.unwrap();
    let events = collect_until_done(&mut rx).await;
    assert_eq!(events.last(), Some(&AiEvent::Done));
    let sent = received(dir.path());
    assert!(sent.contains(r#""apiKey":"sk-test-launcher""#));
    assert!(!sent.contains("getAuthStatus"));
    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert!(args.contains("cli_auth_credentials_store"), "{args}");
    assert!(!args.contains("sk-test"), "never on the command line");
    assert!(!args.contains("key leaked"), "never in its environment");
}
