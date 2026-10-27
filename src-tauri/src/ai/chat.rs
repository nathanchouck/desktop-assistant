//! The live conversations: normally one, or two side by side while
//! comparing Claude and ChatGPT. Each is one CLI process (Claude Code or
//! Codex), fed user messages, its output turned into `AiEvent`s for the
//! UI, which tell them apart by conversation id. A conversation's provider
//! is fixed when it starts.
//!
//! The process stays up between turns so follow-ups start instantly. After a
//! while without use it is shut down to free memory; the next follow-up
//! starts a fresh process that resumes the same session.
//!
//! - `session.rs`: how a process starts (`Setup`) and the running one.
//! - `spares.rs`: processes started ahead of a question (`warm`).

mod session;
mod spares;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use futures::future::join_all;
use tokio::sync::Mutex;

use super::message::Message;
use super::{locked, AiError, AiEvent, ProviderId};
use session::Session;
use spares::Spare;

pub use super::process::EventSink;
pub use session::Setup;

pub struct Chat {
    sink: EventSink,
    /// The live conversations, in the order they were asked for: one, or
    /// two while comparing.
    current: Mutex<Vec<Conversation>>,
    /// Started ahead of the next question (one per AI at most).
    spare: Mutex<Vec<Spare>>,
    next_id: AtomicU32,
}

/// A conversation's provider, folder and session (or thread) id: enough to
/// carry it on after it's closed (conversation history keeps it).
#[derive(Clone)]
pub struct Resumable {
    pub provider: ProviderId,
    pub cwd: PathBuf,
    /// Filled in once the AI reports it; a resumed process may replace it.
    session: Arc<StdMutex<Option<String>>>,
}

impl Resumable {
    pub fn new(provider: ProviderId, cwd: PathBuf, session: Option<String>) -> Self {
        Self {
            provider,
            cwd,
            session: Arc::new(StdMutex::new(session)),
        }
    }

    /// The session (thread) id, once known.
    pub fn session(&self) -> Option<String> {
        locked(&self.session).clone()
    }
}

/// A conversation, possibly without a live process (after an idle shutdown).
struct Conversation {
    id: u32,
    provider: ProviderId,
    session: Option<Session>,
    /// Remembered so a new process can resume the same session.
    resume: Arc<StdMutex<Option<String>>>,
    /// The folder it started in; a resumed process starts there too.
    cwd: PathBuf,
    last_used: Instant,
}

impl Conversation {
    /// Ends its process in the background; the conversation is over.
    fn end_soon(self) {
        if let Some(session) = self.session {
            session.process().alive.store(false, Ordering::Release);
            session.end_soon();
        }
    }
}

impl Chat {
    pub fn new(sink: EventSink) -> Self {
        Self {
            sink,
            current: Mutex::new(Vec::new()),
            spare: Mutex::new(Vec::new()),
            next_id: AtomicU32::new(1),
        }
    }

    /// Starts a new conversation (ending any current ones). Returns its id.
    pub async fn start(&self, message: impl Into<Message>, setup: Setup) -> Result<u32, AiError> {
        let message = message.into();
        self.close().await;
        let id = self.new_id();
        let spare = self.take_spare(&setup).await;
        let conversation = self.open(id, &setup, spare, &message).await?;
        self.replace(vec![conversation]).await;
        Ok(id)
    }

    /// Starts one conversation per side at once, each with its own message
    /// and setup (the same question to Claude and ChatGPT), ending any
    /// current ones. `before_start` hears their ids before any of them can
    /// send an event, so whoever records them knows they belong together.
    /// Returns each side's id, or why it couldn't start, in order; the ones
    /// that started are the live conversations.
    pub async fn compare(
        &self,
        sides: Vec<(Message, Setup)>,
        before_start: impl FnOnce(&[u32]),
    ) -> Vec<Result<u32, AiError>> {
        self.close().await;
        let ids: Vec<u32> = sides.iter().map(|_| self.new_id()).collect();
        before_start(&ids);
        let setups: Vec<&Setup> = sides.iter().map(|(_, setup)| setup).collect();
        let spares = self.take_spares(&setups).await;
        let opening = sides
            .iter()
            .zip(&ids)
            .zip(spares)
            .map(|(((message, setup), &id), spare)| self.open(id, setup, spare, message));
        let mut live = Vec::new();
        let mut results = Vec::new();
        for (opened, &id) in join_all(opening).await.into_iter().zip(&ids) {
            results.push(opened.map(|conversation| {
                live.push(conversation);
                id
            }));
        }
        self.replace(live).await;
        results
    }

    /// Sends a follow-up in the current conversation (the first one while
    /// comparing), restarting its process (resuming the session) if it was
    /// shut down. `setup` gives today's setup for the conversation's
    /// provider, used only for a restart.
    pub async fn follow_up(
        &self,
        message: impl Into<Message>,
        setup: impl FnOnce(ProviderId) -> Result<Setup, AiError>,
    ) -> Result<u32, AiError> {
        let mut current = self.current.lock().await;
        let conversation = current.first_mut().ok_or(AiError::NoConversation)?;
        self.send(conversation, &message.into(), setup).await
    }

    /// The same, in live conversation `id` (one side of a comparison).
    pub async fn follow_up_in(
        &self,
        id: u32,
        message: impl Into<Message>,
        setup: impl FnOnce(ProviderId) -> Result<Setup, AiError>,
    ) -> Result<u32, AiError> {
        let mut current = self.current.lock().await;
        let conversation = current
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(AiError::NoConversation)?;
        self.send(conversation, &message.into(), setup).await
    }

    /// Which provider the current conversation (the first while comparing)
    /// uses.
    pub async fn provider(&self) -> Option<ProviderId> {
        self.current.lock().await.first().map(|c| c.provider)
    }

    /// The live conversations' ids and providers, in order.
    pub async fn conversations(&self) -> Vec<(u32, ProviderId)> {
        let current = self.current.lock().await;
        current.iter().map(|c| (c.id, c.provider)).collect()
    }

    /// Conversation `id`'s provider, folder and session, while it's live.
    pub async fn resumable(&self, id: u32) -> Option<Resumable> {
        let current = self.current.lock().await;
        let conversation = current.iter().find(|c| c.id == id)?;
        Some(Resumable {
            provider: conversation.provider,
            cwd: conversation.cwd.clone(),
            session: Arc::clone(&conversation.resume),
        })
    }

    /// Makes a saved conversation the current one (ending any other), idle
    /// with no process: its next follow-up starts the AI resuming the
    /// session in the conversation's folder, as after an idle shutdown.
    /// Returns its id.
    pub async fn adopt(&self, saved: Resumable) -> u32 {
        self.close().await;
        let id = self.new_id();
        self.replace(vec![Conversation {
            id,
            provider: saved.provider,
            session: None,
            resume: saved.session,
            cwd: saved.cwd,
            last_used: Instant::now(),
        }])
        .await;
        id
    }

    /// Carries on with conversation `id` alone (one side of a comparison):
    /// the others end.
    pub async fn keep(&self, id: u32) -> Result<(), AiError> {
        let mut current = self.current.lock().await;
        if !current.iter().any(|c| c.id == id) {
            return Err(AiError::NoConversation);
        }
        let (kept, others) = std::mem::take(&mut *current)
            .into_iter()
            .partition(|c| c.id == id);
        *current = kept;
        for conversation in others {
            conversation.end_soon();
        }
        Ok(())
    }

    /// Answers a permission request, in whichever live conversation asked.
    pub async fn respond(&self, request_id: &str, allow: bool) -> Result<(), AiError> {
        let current = self.current.lock().await;
        if current.is_empty() {
            return Err(AiError::NoConversation);
        }
        for conversation in current.iter() {
            let Some(session) = conversation.session.as_ref() else {
                continue;
            };
            match session.respond(request_id, allow).await {
                Ok(()) => {
                    let _ = self.sink.send((
                        conversation.id,
                        AiEvent::ApprovalResolved {
                            id: request_id.into(),
                            allowed: allow,
                        },
                    ));
                    return Ok(());
                }
                Err(AiError::UnknownApproval) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(AiError::UnknownApproval)
    }

    /// Stops the current turns (both, while comparing); the conversations
    /// stay open.
    pub async fn stop(&self) {
        let current = self.current.lock().await;
        let sessions = current.iter().filter_map(|c| c.session.as_ref());
        join_all(sessions.map(Session::stop)).await;
    }

    /// Ends the conversations and their processes.
    pub async fn close(&self) {
        let ended = std::mem::take(&mut *self.current.lock().await);
        for conversation in ended {
            conversation.end_soon();
        }
    }

    /// Frees the process of each conversation idle for longer than
    /// `max_idle`, keeping the conversation so a follow-up can resume it.
    pub async fn shut_down_if_idle(&self, max_idle: Duration) {
        let mut current = self.current.lock().await;
        for conversation in current.iter_mut() {
            let idle = conversation.last_used.elapsed() > max_idle;
            let busy = conversation
                .session
                .as_ref()
                .is_some_and(|s| s.process().is_busy());
            if idle && !busy {
                if let Some(session) = conversation.session.take() {
                    tracing::info!("closing idle {} process", session.process().name);
                    session.process().alive.store(false, Ordering::Release);
                    session.end_soon();
                }
            }
        }
    }

    /// Ends everything: the conversations and any spares (app exit).
    pub async fn shut_down(&self) {
        let ended = std::mem::take(&mut *self.current.lock().await);
        for session in ended.into_iter().filter_map(|c| c.session) {
            session.end().await;
        }
        self.end_spares().await;
    }

    fn new_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Starts conversation `id` with `setup` (on `spare` if there is one)
    /// and sends its first message.
    async fn open(
        &self,
        id: u32,
        setup: &Setup,
        spare: Option<Spare>,
        message: &Message,
    ) -> Result<Conversation, AiError> {
        let (session, resume) = match spare {
            Some(spare) => {
                tracing::info!(
                    "conversation {id} uses the warm {} process",
                    spare.session.process().name
                );
                spare.session.process().adopt(id);
                (spare.session, spare.resume)
            }
            None => {
                let resume = Arc::new(StdMutex::new(None));
                let session = Session::spawn(setup, id, self.sink.clone(), Arc::clone(&resume))?;
                (session, resume)
            }
        };
        session.send_turn(message).await?;
        Ok(Conversation {
            id,
            provider: setup.provider(),
            session: Some(session),
            resume,
            cwd: setup.cwd().to_path_buf(),
            last_used: Instant::now(),
        })
    }

    /// Sends `message` in `conversation`, restarting its process first if
    /// it was shut down.
    async fn send(
        &self,
        conversation: &mut Conversation,
        message: &Message,
        setup: impl FnOnce(ProviderId) -> Result<Setup, AiError>,
    ) -> Result<u32, AiError> {
        let live = conversation
            .session
            .as_ref()
            .filter(|s| s.process().is_alive())
            .cloned();
        let session = match live {
            Some(session) => session,
            None => {
                let resume = locked(&conversation.resume).clone();
                let setup =
                    setup(conversation.provider)?.resuming(resume, conversation.cwd.clone());
                let session = Session::spawn(
                    &setup,
                    conversation.id,
                    self.sink.clone(),
                    Arc::clone(&conversation.resume),
                )?;
                conversation.session = Some(session.clone());
                session
            }
        };
        session.send_turn(message).await?;
        conversation.last_used = Instant::now();
        Ok(conversation.id)
    }

    /// Makes `live` the current conversations, ending any that started in
    /// the meantime (two questions racing).
    async fn replace(&self, live: Vec<Conversation>) {
        let raced = std::mem::replace(&mut *self.current.lock().await, live);
        for conversation in raced {
            conversation.end_soon();
        }
    }
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(all(test, unix))]
mod compare_tests;
