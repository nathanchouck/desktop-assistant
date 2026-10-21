//! AI chat: Claude Code today (`claude/`), Codex in M5. A conversation is a
//! long-running CLI process; its output becomes `AiEvent`s for the UI.

pub mod chat;
pub mod claude;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Claude,
    Chatgpt,
}

/// How a provider is paid for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    /// Use the CLI's own login (Claude Pro/Max, ChatGPT Plus/Pro). The API key
    /// env vars are stripped from the child process so they can't take over.
    #[default]
    Subscription,
    /// Use the API key from the environment (`ANTHROPIC_API_KEY`).
    ApiKey,
}

/// Who approves actions that change things (writing files, running commands).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Approvals {
    /// You do: an Approve / Deny card in the launcher.
    #[default]
    Ask,
    /// Claude Code's auto mode decides, and asks you only when it isn't sure.
    Auto,
}

/// What the UI hears about a conversation, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiEvent {
    /// The model is connected and answering.
    Started {
        model: String,
    },
    /// More answer text (append to the current reply).
    TextDelta {
        text: String,
    },
    /// Claude started using a tool, e.g. "Write · notes.txt".
    ToolStarted {
        id: String,
        name: String,
        summary: String,
    },
    ToolFinished {
        id: String,
        ok: bool,
    },
    /// Waiting for Approve / Deny.
    ApprovalRequested {
        id: String,
        tool: String,
        summary: String,
        detail: String,
    },
    /// The approval was answered (by you) or withdrawn (by Claude).
    ApprovalResolved {
        id: String,
        allowed: bool,
    },
    /// The turn finished; a follow-up can be sent.
    Done,
    /// You stopped the turn; a follow-up can be sent.
    Stopped,
    Error {
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error(
        "Claude Code isn't installed (or isn't on PATH). Install it from \
         https://code.claude.com, run `claude` once to sign in, then try again."
    )]
    NotInstalled,
    #[error("API key mode is on but ANTHROPIC_API_KEY isn't set")]
    MissingApiKey,
    #[error("could not start Claude Code: {0}")]
    Spawn(String),
    #[error("Claude Code stopped: {0}")]
    Process(String),
    #[error("there's no conversation to continue")]
    NoConversation,
    #[error("that approval is no longer pending")]
    UnknownApproval,
}
