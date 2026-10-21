//! The current conversation: one Claude Code process, fed user messages on
//! stdin, its output turned into `AiEvent`s for the UI.
//!
//! The process stays up between turns so follow-ups start instantly. After a
//! while without use it is shut down to free memory; the next follow-up
//! starts a fresh process that resumes the same session.
//!
//! A spare process can be started ahead of a question (`warm`) so the
//! first answer comes about a second sooner; the next new conversation
//! with the same options claims it. Unclaimed, it's closed after a while.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, Mutex};

use super::claude::{self, protocol, protocol::Incoming, Launch, Options};
use super::{AiError, AiEvent};

/// Where events go: (conversation id, event).
pub type EventSink = mpsc::UnboundedSender<(u32, AiEvent)>;

/// Lines of stderr kept for error messages.
const STDERR_LINES: usize = 20;

pub struct Chat {
    sink: EventSink,
    current: Mutex<Option<Conversation>>,
    /// Started ahead of the next question.
    spare: Mutex<Option<Spare>>,
    next_id: AtomicU32,
    next_request: AtomicU32,
}

struct Spare {
    process: Arc<Process>,
    session: Arc<StdMutex<Option<String>>>,
    /// A question with other options (folder, model...) can't use it.
    options: Options,
    born: Instant,
}

/// Which conversation a process belongs to. `id` 0: an unclaimed spare,
/// whose events are held until a conversation claims it.
#[derive(Default)]
struct Owner {
    id: u32,
    held: Vec<AiEvent>,
}

/// A conversation, possibly without a live process (after an idle shutdown).
struct Conversation {
    id: u32,
    process: Option<Arc<Process>>,
    /// Remembered so a new process can resume the same session.
    session: Arc<StdMutex<Option<String>>>,
    /// Claude Code keeps sessions per working folder, so a resumed process
    /// must start in the same one.
    cwd: std::path::PathBuf,
    last_used: Instant,
}

struct Process {
    owner: StdMutex<Owner>,
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    /// Open permission requests: request id → tool input to echo on allow.
    pending: StdMutex<HashMap<String, Value>>,
    /// A turn is running (between sending a message and its result).
    busy: AtomicBool,
    /// You asked to stop this turn; its error result means "stopped".
    stopping: AtomicBool,
    alive: AtomicBool,
}

impl Process {
    /// Sends `event` to its conversation, or holds it while unclaimed.
    fn emit(&self, sink: &EventSink, event: AiEvent) {
        let Ok(mut owner) = self.owner.lock() else {
            return;
        };
        if owner.id == 0 {
            owner.held.push(event);
        } else {
            let _ = sink.send((owner.id, event));
        }
    }

    /// Hands a spare to conversation `id`, with what it said so far.
    fn adopt(&self, id: u32, sink: &EventSink) {
        if let Ok(mut owner) = self.owner.lock() {
            owner.id = id;
            for event in owner.held.drain(..) {
                let _ = sink.send((id, event));
            }
        }
    }

    async fn kill(&self) {
        self.alive.store(false, Ordering::Release);
        let _ = self.child.lock().await.kill().await;
    }

    /// Marks a new turn as running and sends its message.
    async fn send_turn(&self, text: &str) -> Result<(), AiError> {
        self.stopping.store(false, Ordering::Release);
        self.busy.store(true, Ordering::Release);
        self.send_line(&protocol::user_message(text)).await
    }
}

impl Process {
    async fn send_line(&self, line: &str) -> Result<(), AiError> {
        let mut stdin = self.stdin.lock().await;
        let write = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await
        };
        write
            .await
            .map_err(|e| AiError::Process(format!("could not send to Claude Code: {e}")))
    }
}

impl Chat {
    pub fn new(sink: EventSink) -> Self {
        Self {
            sink,
            current: Mutex::new(None),
            spare: Mutex::new(None),
            next_id: AtomicU32::new(1),
            next_request: AtomicU32::new(1),
        }
    }

    /// Starts a new conversation (ending any current one). Returns its id.
    pub async fn start(
        &self,
        text: &str,
        launch: &Launch,
        options: Options,
    ) -> Result<u32, AiError> {
        self.close().await;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (process, session) = match self.take_spare(&options).await {
            Some(spare) => {
                tracing::info!("conversation {id} uses the warm Claude Code process");
                spare.process.adopt(id, &self.sink);
                (spare.process, spare.session)
            }
            None => {
                let session = Arc::new(StdMutex::new(None));
                let process = self.spawn(id, launch, &options, Arc::clone(&session))?;
                (process, session)
            }
        };
        process.send_turn(text).await?;
        *self.current.lock().await = Some(Conversation {
            id,
            process: Some(process),
            session,
            cwd: options.cwd.clone(),
            last_used: Instant::now(),
        });
        Ok(id)
    }

    /// Sends a follow-up in the current conversation, restarting its process
    /// (resuming the session) if it was shut down.
    pub async fn follow_up(
        &self,
        text: &str,
        launch: &Launch,
        mut options: Options,
    ) -> Result<u32, AiError> {
        let mut current = self.current.lock().await;
        let conversation = current.as_mut().ok_or(AiError::NoConversation)?;
        let live = conversation
            .process
            .as_ref()
            .filter(|p| p.alive.load(Ordering::Acquire))
            .cloned();
        let process = match live {
            Some(process) => process,
            None => {
                options.resume = conversation.session.lock().ok().and_then(|s| s.clone());
                options.cwd = conversation.cwd.clone();
                let process = self.spawn(
                    conversation.id,
                    launch,
                    &options,
                    Arc::clone(&conversation.session),
                )?;
                conversation.process = Some(Arc::clone(&process));
                process
            }
        };
        process.send_turn(text).await?;
        conversation.last_used = Instant::now();
        Ok(conversation.id)
    }

    /// Answers a permission request.
    pub async fn respond(&self, request_id: &str, allow: bool) -> Result<(), AiError> {
        let current = self.current.lock().await;
        let conversation = current.as_ref().ok_or(AiError::NoConversation)?;
        let process = conversation
            .process
            .as_ref()
            .ok_or(AiError::UnknownApproval)?;
        let input = process
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(request_id))
            .ok_or(AiError::UnknownApproval)?;
        process
            .send_line(&protocol::permission_response(request_id, allow, &input))
            .await?;
        let _ = self.sink.send((
            conversation.id,
            AiEvent::ApprovalResolved {
                id: request_id.into(),
                allowed: allow,
            },
        ));
        Ok(())
    }

    /// Stops the current turn; the conversation stays open.
    pub async fn stop(&self) {
        let current = self.current.lock().await;
        let Some(process) = current.as_ref().and_then(|c| c.process.as_ref()) else {
            return;
        };
        if process.busy.load(Ordering::Acquire) {
            process.stopping.store(true, Ordering::Release);
            let id = format!("stop-{}", self.next_request.fetch_add(1, Ordering::Relaxed));
            if let Err(e) = process.send_line(&protocol::interrupt(&id)).await {
                tracing::warn!("{e}");
            }
        }
    }

    /// Ends the conversation and its process.
    pub async fn close(&self) {
        if let Some(conversation) = self.current.lock().await.take() {
            if let Some(process) = conversation.process {
                let _ = process.child.lock().await.kill().await;
            }
        }
    }

    /// Frees the process of a conversation idle for longer than `max_idle`,
    /// keeping the conversation so a follow-up can resume it.
    pub async fn shut_down_if_idle(&self, max_idle: Duration) {
        let mut current = self.current.lock().await;
        let Some(conversation) = current.as_mut() else {
            return;
        };
        let idle = conversation.last_used.elapsed() > max_idle;
        let busy = conversation
            .process
            .as_ref()
            .is_some_and(|p| p.busy.load(Ordering::Acquire));
        if idle && !busy {
            if let Some(process) = conversation.process.take() {
                tracing::info!("closing idle Claude Code process");
                process.alive.store(false, Ordering::Release);
                let _ = process.child.lock().await.kill().await;
            }
        }
    }

    /// Starts a spare process for the next question, unless a matching one
    /// is already waiting.
    pub async fn warm(&self, launch: &Launch, options: Options) -> Result<(), AiError> {
        let mut spare = self.spare.lock().await;
        if let Some(existing) = spare.as_ref() {
            if existing.options == options && existing.process.alive.load(Ordering::Acquire) {
                return Ok(());
            }
        }
        if let Some(old) = spare.take() {
            old.process.kill().await;
        }
        let session = Arc::new(StdMutex::new(None));
        let process = self.spawn(0, launch, &options, Arc::clone(&session))?;
        *spare = Some(Spare {
            process,
            session,
            options,
            born: Instant::now(),
        });
        Ok(())
    }

    /// The spare, if it's alive and was started with `options`. A spare
    /// that doesn't match is closed: the user's setup has moved on.
    async fn take_spare(&self, options: &Options) -> Option<Spare> {
        let spare = self.spare.lock().await.take()?;
        if spare.options == *options && spare.process.alive.load(Ordering::Acquire) {
            return Some(spare);
        }
        spare.process.kill().await;
        None
    }

    /// Closes a spare nobody claimed within `max_age`.
    pub async fn drop_stale_spare(&self, max_age: Duration) {
        let mut spare = self.spare.lock().await;
        if spare.as_ref().is_some_and(|s| s.born.elapsed() > max_age) {
            if let Some(old) = spare.take() {
                tracing::info!("closing the unused warm Claude Code process");
                old.process.kill().await;
            }
        }
    }

    /// Ends everything: the conversation and any spare (app exit).
    pub async fn shut_down(&self) {
        self.close().await;
        if let Some(spare) = self.spare.lock().await.take() {
            spare.process.kill().await;
        }
    }

    /// `conversation` 0 starts an unclaimed spare.
    fn spawn(
        &self,
        conversation: u32,
        launch: &Launch,
        options: &Options,
        session: Arc<StdMutex<Option<String>>>,
    ) -> Result<Arc<Process>, AiError> {
        let mut child = claude::command(launch, options)?
            .spawn()
            .map_err(|e| AiError::Spawn(e.to_string()))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(AiError::Spawn("missing stdio pipes".into()));
        };
        if conversation == 0 {
            tracing::info!("started a warm Claude Code process");
        } else {
            tracing::info!("started Claude Code for conversation {conversation}");
        }

        let process = Arc::new(Process {
            owner: StdMutex::new(Owner {
                id: conversation,
                held: Vec::new(),
            }),
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending: StdMutex::new(HashMap::new()),
            busy: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            alive: AtomicBool::new(true),
        });

        let stderr_tail = Arc::new(StdMutex::new(VecDeque::new()));
        let tail = Arc::clone(&stderr_tail);
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!("claude stderr: {line}");
                if let Ok(mut tail) = tail.lock() {
                    if tail.len() == STDERR_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            }
        });

        let reader = Arc::clone(&process);
        let sink = self.sink.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                for incoming in protocol::parse_line(&line) {
                    handle(&reader, &sink, &session, incoming).await;
                }
            }
            reader.alive.store(false, Ordering::Release);
            // Let the stderr reader catch up so a crash message is complete.
            let _ = tokio::time::timeout(Duration::from_millis(500), stderr_task).await;
            if reader.busy.swap(false, Ordering::AcqRel) {
                let detail = stderr_tail
                    .lock()
                    .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
                    .unwrap_or_default();
                let message = if detail.trim().is_empty() {
                    "Claude Code exited unexpectedly.".to_owned()
                } else {
                    format!("Claude Code exited unexpectedly: {}", detail.trim())
                };
                reader.emit(&sink, AiEvent::Error { message });
            }
        });
        Ok(process)
    }
}

async fn handle(
    process: &Process,
    sink: &EventSink,
    session: &StdMutex<Option<String>>,
    incoming: Incoming,
) {
    match incoming {
        Incoming::Event(event) => {
            let event = match event {
                AiEvent::Done | AiEvent::Error { .. } => {
                    process.busy.store(false, Ordering::Release);
                    // An interrupted turn ends in an error result; it isn't one.
                    let stopped = process.stopping.swap(false, Ordering::AcqRel);
                    if stopped && matches!(event, AiEvent::Error { .. }) {
                        AiEvent::Stopped
                    } else {
                        event
                    }
                }
                other => other,
            };
            process.emit(sink, event);
        }
        Incoming::Session { id: session_id } => {
            if let Ok(mut s) = session.lock() {
                *s = Some(session_id);
            }
        }
        Incoming::Permission {
            request_id,
            input,
            event,
        } => {
            if let Ok(mut pending) = process.pending.lock() {
                pending.insert(request_id, input);
            }
            process.emit(sink, event);
        }
        Incoming::PermissionWithdrawn { request_id } => {
            let was_pending = process
                .pending
                .lock()
                .is_ok_and(|mut p| p.remove(&request_id).is_some());
            if was_pending {
                process.emit(
                    sink,
                    AiEvent::ApprovalResolved {
                        id: request_id,
                        allowed: false,
                    },
                );
            }
        }
        Incoming::Unsupported { request_id } => {
            let _ = process
                .send_line(&protocol::error_response(
                    &request_id,
                    "not supported by the launcher",
                ))
                .await;
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::ai::{Approvals, BillingMode};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// A stand-in `claude` that ignores its arguments, prints a recorded
    /// session, then waits on stdin like the real one.
    fn fake_claude(dir: &Path, fixture: &str) -> Launch {
        let fixture = format!(
            "{}/tests/fixtures/claude/{fixture}.jsonl",
            env!("CARGO_MANIFEST_DIR")
        );
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!("#!/bin/sh\ncat '{fixture}'\ncat > /dev/null\n"),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Launch::Exe(script)
    }

    fn options(dir: &Path) -> Options {
        Options {
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

    async fn collect_until_done(rx: &mut mpsc::UnboundedReceiver<(u32, AiEvent)>) -> Vec<AiEvent> {
        let mut events = Vec::new();
        while let Ok(Some((_, event))) =
            tokio::time::timeout(Duration::from_secs(10), rx.recv()).await
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
            .start("make a file", &launch, options(dir.path()))
            .await
            .unwrap();
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
            chat.follow_up("more", &launch, options(dir.path())).await,
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
            .start("hi", &launch, options(dir.path()))
            .await
            .unwrap();
        collect_until_done(&mut rx).await;
        chat.shut_down_if_idle(Duration::ZERO).await;

        // Follow-ups pass today's options; the restart must still use the
        // conversation's own folder or Claude Code can't find the session.
        let elsewhere = tempfile::tempdir().unwrap();
        let again = chat
            .follow_up("again", &launch, options(elsewhere.path()))
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
        let script = dir.path().join("claude");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'boom: not signed in' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let chat = Chat::new(tx);
        // The write may race the exit; either way the crash must surface.
        let _ = chat
            .start("hi", &Launch::Exe(script), options(dir.path()))
            .await;
        let events = collect_until_done(&mut rx).await;
        assert!(
            matches!(events.last(), Some(AiEvent::Error { message }) if message.contains("boom"))
        );
    }

    #[tokio::test]
    async fn a_stopped_turn_reads_as_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("claude");
        // Answers the interrupt the way Claude Code does: an error result.
        std::fs::write(
            &script,
            "#!/bin/sh\nwhile read -r line; do\n  case \"$line\" in\n    *interrupt*) echo '{\"type\":\"result\",\"subtype\":\"error_during_execution\",\"is_error\":true}' ;;\n  esac\ndone\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let chat = Chat::new(tx);
        chat.start("long story", &Launch::Exe(script), options(dir.path()))
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
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho started >> '{}'\ncat '{fixture}'\ncat > /dev/null\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Launch::Exe(script)
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
        chat.warm(&launch, options(dir.path())).await.unwrap();
        // Warming again with the same options keeps the one spare.
        chat.warm(&launch, options(dir.path())).await.unwrap();
        // Let the spare print before it's claimed: that output is held.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            rx.try_recv().is_err(),
            "nothing sent before a conversation claims it"
        );

        let id = chat
            .start("hi", &launch, options(dir.path()))
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
        chat.warm(&launch, options(elsewhere.path())).await.unwrap();
        // Let the spare start before it's passed over.
        tokio::time::sleep(Duration::from_millis(300)).await;
        chat.start("hi", &launch, options(dir.path()))
            .await
            .unwrap();
        collect_until_done(&mut rx).await;
        assert_eq!(starts(dir.path()), 2);
        assert!(chat.spare.lock().await.is_none());

        chat.warm(&launch, options(dir.path())).await.unwrap();
        chat.drop_stale_spare(Duration::ZERO).await;
        assert!(chat.spare.lock().await.is_none(), "an old spare is closed");
    }
}
