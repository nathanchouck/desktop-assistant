//! One `codex app-server` process: the start-up handshake, the thread, and
//! each turn. Turns are queued and sent in order by a driver task, so a
//! question can be asked (or a spare claimed) before Codex is ready; any
//! failure on the way arrives as an `AiEvent::Error`, like a crash would.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::parser::{Incoming, Parser};
use super::protocol::{self, ApprovalKind};
use super::{command, ApiKey, Launch, Options, NAME};
use crate::ai::locked as lock_or_recover;
use crate::ai::process::{self, EventSink, Process};
use crate::ai::{AiError, AiEvent, BillingMode};

/// Starting a 300 MB exe the first time (with a virus scan) can be slow.
const START_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long Codex gets to exit by itself before it's killed.
const EXIT_GRACE: Duration = Duration::from_secs(2);

/// Numbers each process, for approval ids (see `Parser::approval_id`).
static GENERATION: AtomicU32 = AtomicU32::new(1);

type Waiter = oneshot::Sender<Result<Value, String>>;

pub struct CodexSession {
    pub process: Arc<Process>,
    next_rpc: AtomicU64,
    /// Our requests waiting for their reply, by id.
    waiters: StdMutex<HashMap<u64, Waiter>>,
    /// The conversation's thread (kept by the conversation across
    /// processes, so a new one can resume it).
    thread: Arc<StdMutex<Option<String>>>,
    /// The running turn, for interrupting it.
    turn: StdMutex<Option<String>>,
    /// Stop was asked for before the turn had an id.
    stop_requested: AtomicBool,
    /// Open approvals: UI id → (Codex's request id, kind).
    pending: StdMutex<HashMap<String, (Value, ApprovalKind)>>,
    turns: mpsc::UnboundedSender<String>,
}

impl CodexSession {
    /// Starts Codex for `conversation` (0: a spare). `thread` holds the
    /// thread to resume (`options.resume`) and receives the one opened.
    pub fn spawn(
        launch: &Launch,
        options: &Options,
        conversation: u32,
        sink: EventSink,
        thread: Arc<StdMutex<Option<String>>>,
    ) -> Result<Arc<Self>, AiError> {
        let key = match options.billing {
            BillingMode::ApiKey => Some(options.api_key.clone().ok_or(AiError::MissingOpenAiKey)?),
            BillingMode::Subscription => None,
        };
        let (process, stdout, tail) =
            process::spawn(NAME, command(launch, options), conversation, sink)?;
        let (turns, queued) = mpsc::unbounded_channel();
        let this = Arc::new(Self {
            process,
            next_rpc: AtomicU64::new(1),
            waiters: StdMutex::new(HashMap::new()),
            thread,
            turn: StdMutex::new(None),
            stop_requested: AtomicBool::new(false),
            pending: StdMutex::new(HashMap::new()),
            turns,
        });

        let reader = Arc::clone(&this);
        let prefix = GENERATION.fetch_add(1, Ordering::Relaxed).to_string();
        tokio::spawn(async move {
            let mut parser = Parser::new(prefix);
            let mut lines = process::lines(stdout);
            while let Ok(Some(line)) = lines.next_line().await {
                let main = lock_or_recover(&reader.thread).clone();
                for incoming in parser.feed(&line, main.as_deref()) {
                    reader.handle(&parser, incoming).await;
                }
            }
            // Gone: requests still waiting fail now rather than time out.
            reader.process.alive.store(false, Ordering::Release);
            lock_or_recover(&reader.waiters).clear();
            reader.process.exited(tail).await;
        });

        let driver = Arc::clone(&this);
        let options = options.clone();
        tokio::spawn(async move { driver.drive(options, key, queued).await });
        Ok(this)
    }

    /// Marks a new turn as running and queues its message.
    pub async fn send_turn(&self, text: &str) -> Result<(), AiError> {
        self.stop_requested.store(false, Ordering::Release);
        self.process.busy.store(true, Ordering::Release);
        self.turns.send(text.to_owned()).map_err(|_| {
            self.process.busy.store(false, Ordering::Release);
            AiError::Process(NAME, "it isn't running any more".into())
        })
    }

    /// Answers an approval request.
    pub async fn respond(&self, id: &str, allow: bool) -> Result<(), AiError> {
        let (rpc_id, kind) = lock_or_recover(&self.pending)
            .remove(id)
            .ok_or(AiError::UnknownApproval)?;
        self.process
            .send_line(&protocol::approval_response(&rpc_id, kind, allow))
            .await
    }

    /// Interrupts the running turn; it ends with "interrupted" (`Stopped`).
    pub async fn stop(&self) {
        if !self.process.is_busy() {
            return;
        }
        let thread = lock_or_recover(&self.thread).clone();
        let turn = lock_or_recover(&self.turn).clone();
        match (thread, turn) {
            (Some(thread), Some(turn)) => self.interrupt(&thread, &turn).await,
            // Not sent yet (Codex is still starting): it won't be.
            _ => self.stop_requested.store(true, Ordering::Release),
        }
    }

    /// Asks Codex to exit (closing stdin), then kills it if it doesn't.
    pub async fn end(&self) {
        self.process.close(EXIT_GRACE).await;
    }

    async fn interrupt(&self, thread: &str, turn: &str) {
        let id = self.next_rpc.fetch_add(1, Ordering::Relaxed);
        // Its reply only says it arrived; the turn's end says "interrupted".
        let line = protocol::turn_interrupt(id, thread, turn);
        if let Err(e) = self.process.send_line(&line).await {
            tracing::warn!("{e}");
        }
    }

    /// Sends request `build(id)` and waits for its reply.
    async fn request(
        &self,
        build: impl FnOnce(u64) -> String,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_rpc.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock_or_recover(&self.waiters).insert(id, tx);
        if let Err(e) = self.process.send_line(&build(id)).await {
            lock_or_recover(&self.waiters).remove(&id);
            return Err(e.to_string());
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Codex stopped".into()),
            Err(_) => {
                lock_or_recover(&self.waiters).remove(&id);
                Err("Codex didn't answer in time".into())
            }
        }
    }

    /// Start-up, then each queued turn in order. The thread opens with the
    /// first turn, so an unclaimed spare never leaves an empty one behind.
    async fn drive(
        self: Arc<Self>,
        options: Options,
        key: Option<ApiKey>,
        mut queued: mpsc::UnboundedReceiver<String>,
    ) {
        if let Err(message) = self.handshake(key).await {
            return self.fail(message).await;
        }
        let mut open = false;
        while let Some(text) = queued.recv().await {
            if !open {
                if let Err(message) = self.open_thread(&options).await {
                    return self.fail(message).await;
                }
                open = true;
            }
            self.run_turn(&text).await;
        }
    }

    /// `initialize`, then the sign-in check (or the API key).
    async fn handshake(&self, key: Option<ApiKey>) -> Result<(), String> {
        let init = self
            .request(protocol::initialize, START_TIMEOUT)
            .await
            .map_err(|e| format!("Codex didn't start: {e}"))?;
        match init["userAgent"].as_str().and_then(protocol::version) {
            Some(version) if version < protocol::MIN_VERSION => tracing::warn!(
                "Codex {version:?} is older than the {:?} the launcher was tested with",
                protocol::MIN_VERSION
            ),
            Some(version) => tracing::info!("Codex {version:?} is ready"),
            None => {}
        }
        self.process
            .send_line(&protocol::initialized())
            .await
            .map_err(|e| e.to_string())?;
        match key {
            Some(key) => {
                self.request(
                    |id| protocol::login_api_key(id, key.expose()),
                    REQUEST_TIMEOUT,
                )
                .await
                .map_err(|e| format!("Codex didn't accept the API key: {e}"))?;
            }
            None => {
                let status = self
                    .request(protocol::get_auth_status, REQUEST_TIMEOUT)
                    .await
                    .map_err(|e| format!("Couldn't check Codex's sign-in: {e}"))?;
                if let Some(problem) = protocol::auth_problem(&status) {
                    return Err(problem.into());
                }
            }
        }
        Ok(())
    }

    /// Starts the thread, or resumes the conversation's. If the old one
    /// can't be found, a new thread starts and the chat says so.
    async fn open_thread(&self, options: &Options) -> Result<(), String> {
        let mut result = self
            .request(|id| protocol::open_thread(id, options), START_TIMEOUT)
            .await;
        if let (Err(e), Some(_)) = (&result, &options.resume) {
            tracing::warn!("could not resume the ChatGPT thread ({e}); starting a new one");
            self.process.emit(AiEvent::TextDelta {
                text: "_The earlier conversation couldn't be reopened, so this answer starts \
                       fresh._\n\n"
                    .into(),
            });
            let fresh = Options {
                resume: None,
                ..options.clone()
            };
            result = self
                .request(|id| protocol::open_thread(id, &fresh), START_TIMEOUT)
                .await;
        }
        let result = result.map_err(|e| format!("Codex couldn't open a conversation: {e}"))?;
        let thread = result["thread"]["id"]
            .as_str()
            .ok_or("Codex didn't say which conversation it opened")?;
        *lock_or_recover(&self.thread) = Some(thread.to_owned());
        self.process.emit(AiEvent::Started {
            model: result["model"].as_str().unwrap_or_default().into(),
        });
        Ok(())
    }

    async fn run_turn(&self, text: &str) {
        if self.stop_requested.swap(false, Ordering::AcqRel) {
            self.process.busy.store(false, Ordering::Release);
            self.process.emit(AiEvent::Stopped);
            return;
        }
        let Some(thread) = lock_or_recover(&self.thread).clone() else {
            return;
        };
        let reply = self
            .request(
                |id| protocol::turn_start(id, &thread, text),
                REQUEST_TIMEOUT,
            )
            .await;
        match reply {
            Ok(result) => {
                if let Some(turn) = result["turn"]["id"].as_str() {
                    if self.process.is_busy() {
                        *lock_or_recover(&self.turn) = Some(turn.to_owned());
                    }
                    if self.stop_requested.swap(false, Ordering::AcqRel) {
                        self.interrupt(&thread, turn).await;
                    }
                }
            }
            Err(message) => {
                self.process.busy.store(false, Ordering::Release);
                self.process.emit(AiEvent::Error {
                    message: format!("ChatGPT couldn't take the message: {message}"),
                });
            }
        }
    }

    /// Start-up went wrong: say why and let the process go.
    async fn fail(&self, message: String) {
        if !self.process.is_alive() {
            // It exited; `Process::exited` reports that with its last words.
            return;
        }
        self.process.busy.store(false, Ordering::Release);
        self.process.emit(AiEvent::Error { message });
        self.end().await;
    }

    async fn handle(&self, parser: &Parser, incoming: Incoming) {
        match incoming {
            Incoming::Reply { id, result } => {
                let waiter = lock_or_recover(&self.waiters).remove(&id);
                match (waiter, result) {
                    (Some(waiter), result) => {
                        let _ = waiter.send(result);
                    }
                    (None, Err(e)) => tracing::debug!("Codex: {e}"),
                    (None, Ok(_)) => {}
                }
            }
            Incoming::TurnStarted { turn_id } => {
                *lock_or_recover(&self.turn) = Some(turn_id);
            }
            Incoming::Event(event) => {
                if matches!(
                    event,
                    AiEvent::Done | AiEvent::Stopped | AiEvent::Error { .. }
                ) {
                    self.process.busy.store(false, Ordering::Release);
                    self.stop_requested.store(false, Ordering::Release);
                    *lock_or_recover(&self.turn) = None;
                }
                self.process.emit(event);
            }
            Incoming::Approval {
                rpc_id,
                kind,
                event,
            } => {
                if let AiEvent::ApprovalRequested { id, .. } = &event {
                    lock_or_recover(&self.pending).insert(id.clone(), (rpc_id, kind));
                }
                self.process.emit(event);
            }
            Incoming::Resolved { rpc_id } => {
                let id = parser.approval_id(&rpc_id);
                if lock_or_recover(&self.pending).remove(&id).is_some() {
                    self.process
                        .emit(AiEvent::ApprovalResolved { id, allowed: false });
                }
            }
            Incoming::Answer(line) => {
                if let Err(e) = self.process.send_line(&line).await {
                    tracing::debug!("{e}");
                }
            }
        }
    }
}
