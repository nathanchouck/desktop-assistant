//! One Claude Code process speaking stream-json: user messages in, events
//! out, permission requests answered over stdin.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use serde_json::Value;

use super::protocol::{self, Incoming};
use super::{command, Launch, Options, NAME};
use crate::ai::message::Message;
use crate::ai::process::{self, EventSink, Process};
use crate::ai::{AiError, AiEvent};

pub struct ClaudeSession {
    pub process: Arc<Process>,
    /// Open permission requests: request id → tool input to echo on allow.
    pending: StdMutex<HashMap<String, Value>>,
    /// You asked to stop this turn; its error result means "stopped".
    stopping: AtomicBool,
    next_request: AtomicU32,
}

impl ClaudeSession {
    /// Starts Claude Code for `conversation` (0: a spare). Its session id
    /// goes into `session` once known, so a later process can resume it.
    pub fn spawn(
        launch: &Launch,
        options: &Options,
        conversation: u32,
        sink: EventSink,
        session: Arc<StdMutex<Option<String>>>,
    ) -> Result<Arc<Self>, AiError> {
        let (process, stdout, tail) =
            process::spawn(NAME, command(launch, options)?, conversation, sink)?;
        let this = Arc::new(Self {
            process,
            pending: StdMutex::new(HashMap::new()),
            stopping: AtomicBool::new(false),
            next_request: AtomicU32::new(1),
        });
        let reader = Arc::clone(&this);
        tokio::spawn(async move {
            let mut lines = process::lines(stdout);
            while let Ok(Some(line)) = lines.next_line().await {
                for incoming in protocol::parse_line(&line) {
                    reader.handle(&session, incoming).await;
                }
            }
            reader.process.exited(tail).await;
        });
        Ok(this)
    }

    /// Marks a new turn as running and sends its message.
    pub async fn send_turn(&self, message: &Message) -> Result<(), AiError> {
        self.stopping.store(false, Ordering::Release);
        self.process.busy.store(true, Ordering::Release);
        self.process
            .send_line(&protocol::user_message(message))
            .await
    }

    /// Answers a permission request.
    pub async fn respond(&self, request_id: &str, allow: bool) -> Result<(), AiError> {
        let input = self
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(request_id))
            .ok_or(AiError::UnknownApproval)?;
        self.process
            .send_line(&protocol::permission_response(request_id, allow, &input))
            .await
    }

    /// Interrupts the running turn, if any.
    pub async fn stop(&self) {
        if !self.process.is_busy() {
            return;
        }
        self.stopping.store(true, Ordering::Release);
        let id = format!("stop-{}", self.next_request.fetch_add(1, Ordering::Relaxed));
        if let Err(e) = self.process.send_line(&protocol::interrupt(&id)).await {
            tracing::warn!("{e}");
        }
    }

    async fn handle(&self, session: &StdMutex<Option<String>>, incoming: Incoming) {
        match incoming {
            Incoming::Event(event) => {
                let event = match event {
                    AiEvent::Done | AiEvent::Error { .. } => {
                        self.process.busy.store(false, Ordering::Release);
                        // An interrupted turn ends in an error result; it isn't one.
                        let stopped = self.stopping.swap(false, Ordering::AcqRel);
                        if stopped && matches!(event, AiEvent::Error { .. }) {
                            AiEvent::Stopped
                        } else {
                            event
                        }
                    }
                    other => other,
                };
                self.process.emit(event);
            }
            Incoming::Session { id } => {
                if let Ok(mut s) = session.lock() {
                    *s = Some(id);
                }
            }
            Incoming::Permission {
                request_id,
                input,
                event,
            } => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.insert(request_id, input);
                }
                self.process.emit(event);
            }
            Incoming::PermissionWithdrawn { request_id } => {
                let was_pending = self
                    .pending
                    .lock()
                    .is_ok_and(|mut p| p.remove(&request_id).is_some());
                if was_pending {
                    self.process.emit(AiEvent::ApprovalResolved {
                        id: request_id,
                        allowed: false,
                    });
                }
            }
            Incoming::Unsupported { request_id } => {
                let _ = self
                    .process
                    .send_line(&protocol::error_response(
                        &request_id,
                        "not supported by the launcher",
                    ))
                    .await;
            }
        }
    }
}
