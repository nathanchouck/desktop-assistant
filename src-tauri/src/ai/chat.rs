//! The current conversation: one CLI process (Claude Code or Codex), fed
//! user messages, its output turned into `AiEvent`s for the UI. A
//! conversation's provider is fixed when it starts.
//!
//! The process stays up between turns so follow-ups start instantly. After a
//! while without use it is shut down to free memory; the next follow-up
//! starts a fresh process that resumes the same session.
//!
//! A spare process can be started ahead of a question (`warm`) so the
//! first answer comes about a second sooner; the next new conversation
//! with the same setup claims it. Unclaimed, it's closed after a while.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use super::claude::{self, session::ClaudeSession};
use super::codex::{self, session::CodexSession};
use super::launch::Launch;
use super::process::Process;
use super::{locked, AiError, AiEvent, ProviderId};

pub use super::process::EventSink;

/// How a conversation's process starts: the provider and its options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    Claude(Launch, claude::Options),
    Codex(Launch, codex::Options),
}

impl Setup {
    pub fn provider(&self) -> ProviderId {
        match self {
            Setup::Claude(..) => ProviderId::Claude,
            Setup::Codex(..) => ProviderId::Chatgpt,
        }
    }

    pub fn cwd(&self) -> &Path {
        match self {
            Setup::Claude(_, options) => &options.cwd,
            Setup::Codex(_, options) => &options.cwd,
        }
    }

    /// Where the conversation works (Explorer's folder, else home).
    pub fn set_cwd(&mut self, cwd: PathBuf) {
        match self {
            Setup::Claude(_, options) => options.cwd = cwd,
            Setup::Codex(_, options) => options.cwd = cwd,
        }
    }

    /// The same, continuing session `resume` in the conversation's folder.
    /// (Claude Code keeps sessions per folder; Codex doesn't mind, but its
    /// sandbox should cover the same folder as before.)
    fn resuming(mut self, resume: Option<String>, cwd: PathBuf) -> Self {
        match &mut self {
            Setup::Claude(_, options) => options.resume = resume,
            Setup::Codex(_, options) => options.resume = resume,
        }
        self.set_cwd(cwd);
        self
    }
}

/// A running process of either provider.
#[derive(Clone)]
enum Session {
    Claude(Arc<ClaudeSession>),
    Codex(Arc<CodexSession>),
}

impl Session {
    /// `conversation` 0 starts an unclaimed spare. `resume` receives the
    /// session (thread) id once known.
    fn spawn(
        setup: &Setup,
        conversation: u32,
        sink: EventSink,
        resume: Arc<StdMutex<Option<String>>>,
    ) -> Result<Self, AiError> {
        Ok(match setup {
            Setup::Claude(launch, options) => Session::Claude(ClaudeSession::spawn(
                launch,
                options,
                conversation,
                sink,
                resume,
            )?),
            Setup::Codex(launch, options) => Session::Codex(CodexSession::spawn(
                launch,
                options,
                conversation,
                sink,
                resume,
            )?),
        })
    }

    fn process(&self) -> &Process {
        match self {
            Session::Claude(s) => &s.process,
            Session::Codex(s) => &s.process,
        }
    }

    async fn send_turn(&self, text: &str) -> Result<(), AiError> {
        match self {
            Session::Claude(s) => s.send_turn(text).await,
            Session::Codex(s) => s.send_turn(text).await,
        }
    }

    async fn respond(&self, request_id: &str, allow: bool) -> Result<(), AiError> {
        match self {
            Session::Claude(s) => s.respond(request_id, allow).await,
            Session::Codex(s) => s.respond(request_id, allow).await,
        }
    }

    async fn stop(&self) {
        match self {
            Session::Claude(s) => s.stop().await,
            Session::Codex(s) => s.stop().await,
        }
    }

    /// Ends the process: Claude Code is killed; Codex is asked to exit
    /// first so it can take its own child processes down with it.
    async fn end(&self) {
        match self {
            Session::Claude(s) => s.process.kill().await,
            Session::Codex(s) => s.end().await,
        }
    }

    /// Ends the process in the background.
    fn end_soon(self) {
        tokio::spawn(async move { self.end().await });
    }
}

pub struct Chat {
    sink: EventSink,
    current: Mutex<Option<Conversation>>,
    /// Started ahead of the next question.
    spare: Mutex<Option<Spare>>,
    next_id: AtomicU32,
}

struct Spare {
    session: Session,
    resume: Arc<StdMutex<Option<String>>>,
    /// A question with another setup (provider, folder, model...) can't
    /// use it.
    setup: Setup,
    born: Instant,
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

impl Chat {
    pub fn new(sink: EventSink) -> Self {
        Self {
            sink,
            current: Mutex::new(None),
            spare: Mutex::new(None),
            next_id: AtomicU32::new(1),
        }
    }

    /// Starts a new conversation (ending any current one). Returns its id.
    pub async fn start(&self, text: &str, setup: Setup) -> Result<u32, AiError> {
        self.close().await;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (session, resume) = match self.take_spare(&setup).await {
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
                let session = Session::spawn(&setup, id, self.sink.clone(), Arc::clone(&resume))?;
                (session, resume)
            }
        };
        session.send_turn(text).await?;
        *self.current.lock().await = Some(Conversation {
            id,
            provider: setup.provider(),
            session: Some(session),
            resume,
            cwd: setup.cwd().to_path_buf(),
            last_used: Instant::now(),
        });
        Ok(id)
    }

    /// Sends a follow-up in the current conversation, restarting its process
    /// (resuming the session) if it was shut down. `setup` gives today's
    /// setup for the conversation's provider, used only for a restart.
    pub async fn follow_up(
        &self,
        text: &str,
        setup: impl FnOnce(ProviderId) -> Result<Setup, AiError>,
    ) -> Result<u32, AiError> {
        let mut current = self.current.lock().await;
        let conversation = current.as_mut().ok_or(AiError::NoConversation)?;
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
        session.send_turn(text).await?;
        conversation.last_used = Instant::now();
        Ok(conversation.id)
    }

    /// Which provider the current conversation uses.
    pub async fn provider(&self) -> Option<ProviderId> {
        self.current.lock().await.as_ref().map(|c| c.provider)
    }

    /// Conversation `id`'s provider, folder and session, while it's the
    /// current one.
    pub async fn resumable(&self, id: u32) -> Option<Resumable> {
        let current = self.current.lock().await;
        let conversation = current.as_ref().filter(|c| c.id == id)?;
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
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        *self.current.lock().await = Some(Conversation {
            id,
            provider: saved.provider,
            session: None,
            resume: saved.session,
            cwd: saved.cwd,
            last_used: Instant::now(),
        });
        id
    }

    /// Answers a permission request.
    pub async fn respond(&self, request_id: &str, allow: bool) -> Result<(), AiError> {
        let current = self.current.lock().await;
        let conversation = current.as_ref().ok_or(AiError::NoConversation)?;
        let session = conversation
            .session
            .as_ref()
            .ok_or(AiError::UnknownApproval)?;
        session.respond(request_id, allow).await?;
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
        if let Some(session) = current.as_ref().and_then(|c| c.session.as_ref()) {
            session.stop().await;
        }
    }

    /// Ends the conversation and its process.
    pub async fn close(&self) {
        let conversation = self.current.lock().await.take();
        if let Some(session) = conversation.and_then(|c| c.session) {
            session.process().alive.store(false, Ordering::Release);
            session.end_soon();
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

    /// Starts a spare process for the next question, unless a matching one
    /// is already waiting.
    pub async fn warm(&self, setup: Setup) -> Result<(), AiError> {
        let mut spare = self.spare.lock().await;
        if let Some(existing) = spare.as_ref() {
            if existing.setup == setup && existing.session.process().is_alive() {
                return Ok(());
            }
        }
        if let Some(old) = spare.take() {
            old.session.end().await;
        }
        let resume = Arc::new(StdMutex::new(None));
        let session = Session::spawn(&setup, 0, self.sink.clone(), Arc::clone(&resume))?;
        *spare = Some(Spare {
            session,
            resume,
            setup,
            born: Instant::now(),
        });
        Ok(())
    }

    /// The spare, if it's alive and was started with `setup`. A spare that
    /// doesn't match is closed: the user's setup has moved on.
    async fn take_spare(&self, setup: &Setup) -> Option<Spare> {
        let spare = self.spare.lock().await.take()?;
        if spare.setup == *setup && spare.session.process().is_alive() {
            return Some(spare);
        }
        spare.session.end_soon();
        None
    }

    /// Closes a spare nobody claimed within `max_age`.
    pub async fn drop_stale_spare(&self, max_age: Duration) {
        let mut spare = self.spare.lock().await;
        if spare.as_ref().is_some_and(|s| s.born.elapsed() > max_age) {
            if let Some(old) = spare.take() {
                tracing::info!(
                    "closing the unused warm {} process",
                    old.session.process().name
                );
                old.session.end().await;
            }
        }
    }

    /// Ends everything: the conversation and any spare (app exit).
    pub async fn shut_down(&self) {
        let conversation = self.current.lock().await.take();
        if let Some(session) = conversation.and_then(|c| c.session) {
            session.end().await;
        }
        if let Some(spare) = self.spare.lock().await.take() {
            spare.session.end().await;
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
