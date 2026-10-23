//! How a conversation's process starts (`Setup`) and the running process
//! of either provider (`Session`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use crate::ai::claude::{self, session::ClaudeSession};
use crate::ai::codex::{self, session::CodexSession};
use crate::ai::launch::Launch;
use crate::ai::message::Message;
use crate::ai::process::{EventSink, Process};
use crate::ai::{AiError, ProviderId};

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
    pub(super) fn resuming(mut self, resume: Option<String>, cwd: PathBuf) -> Self {
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
pub(super) enum Session {
    Claude(Arc<ClaudeSession>),
    Codex(Arc<CodexSession>),
}

impl Session {
    /// `conversation` 0 starts an unclaimed spare. `resume` receives the
    /// session (thread) id once known.
    pub(super) fn spawn(
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

    pub(super) fn process(&self) -> &Process {
        match self {
            Session::Claude(s) => &s.process,
            Session::Codex(s) => &s.process,
        }
    }

    pub(super) async fn send_turn(&self, message: &Message) -> Result<(), AiError> {
        match self {
            Session::Claude(s) => s.send_turn(message).await,
            Session::Codex(s) => s.send_turn(message).await,
        }
    }

    pub(super) async fn respond(&self, request_id: &str, allow: bool) -> Result<(), AiError> {
        match self {
            Session::Claude(s) => s.respond(request_id, allow).await,
            Session::Codex(s) => s.respond(request_id, allow).await,
        }
    }

    pub(super) async fn stop(&self) {
        match self {
            Session::Claude(s) => s.stop().await,
            Session::Codex(s) => s.stop().await,
        }
    }

    /// Ends the process: Claude Code is killed; Codex is asked to exit
    /// first so it can take its own child processes down with it.
    pub(super) async fn end(&self) {
        match self {
            Session::Claude(s) => s.process.kill().await,
            Session::Codex(s) => s.end().await,
        }
    }

    /// Ends the process in the background.
    pub(super) fn end_soon(self) {
        tokio::spawn(async move { self.end().await });
    }
}
