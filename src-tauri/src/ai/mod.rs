//! AI chat through the official CLIs: Claude Code (`claude/`) and ChatGPT
//! through OpenAI's Codex (`codex/`). A conversation is a long-running CLI
//! process (`process.rs`); its output becomes `AiEvent`s for the UI.

pub mod chat;
pub mod claude;
pub mod codex;
pub mod launch;
pub mod process;
pub mod status;

use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Claude,
    Chatgpt,
}

impl ProviderId {
    /// What the UI calls it.
    pub fn name(self) -> &'static str {
        match self {
            ProviderId::Claude => "Claude",
            ProviderId::Chatgpt => "ChatGPT",
        }
    }
}

/// `c <question>` forces Claude and `g <question>` ChatGPT, whatever the
/// default. Returns the provider and the question without the prefix.
pub fn forced_provider(query: &str) -> Option<(ProviderId, &str)> {
    let (prefix, rest) = query.trim_start().split_once(char::is_whitespace)?;
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    match prefix {
        "c" | "C" => Some((ProviderId::Claude, rest)),
        "g" | "G" => Some((ProviderId::Chatgpt, rest)),
        _ => None,
    }
}

/// Locks `mutex`, carrying on with the data if a panic poisoned it: the AI
/// path never panics over a lock.
pub(crate) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// How a provider is paid for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    /// Use the CLI's own login (Claude Pro/Max, ChatGPT Plus/Pro). The API key
    /// env vars are stripped from the child process so they can't take over.
    #[default]
    Subscription,
    /// Use the API key saved in the settings menu (Credential Manager),
    /// else the one in the environment (`ANTHROPIC_API_KEY` for Claude,
    /// `OPENAI_API_KEY` for ChatGPT).
    ApiKey,
}

/// Who approves actions that change things (writing files, running commands).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Approvals {
    /// You do: an Approve / Deny card in the launcher.
    #[default]
    Ask,
    /// The CLI's own automatic mode: Claude Code's auto mode decides and asks
    /// only when unsure; Codex works freely inside its sandbox (the folder
    /// you came from) and asks for anything outside it.
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
    /// The AI started using a tool, e.g. "Write · notes.txt".
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
    /// The approval was answered (by you) or withdrawn (by the AI).
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
        "Claude Code isn't installed. Close this chat and pick \"Install Claude Code\" \
         in the launcher, then sign in."
    )]
    NotInstalled,
    #[error(
        "Codex isn't installed, so ChatGPT can't answer. Close this chat and pick \
         \"Install Codex\" in the launcher, then sign in to ChatGPT."
    )]
    CodexNotInstalled,
    #[error(
        "Claude's billing is set to API key, but no Anthropic API key is saved. Add one in \
         Settings → Integrations (or set ANTHROPIC_API_KEY)."
    )]
    MissingApiKey,
    #[error(
        "ChatGPT's billing is set to API key, but no OpenAI API key is saved. Add one in \
         Settings → Integrations (or set OPENAI_API_KEY)."
    )]
    MissingOpenAiKey,
    /// (program, reason)
    #[error("could not start {0}: {1}")]
    Spawn(&'static str, String),
    /// (program, reason)
    #[error("{0} stopped: {1}")]
    Process(&'static str, String),
    #[error("there's no conversation to continue")]
    NoConversation,
    #[error("that approval is no longer pending")]
    UnknownApproval,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_picks_the_provider() {
        assert_eq!(
            forced_provider("c sort my downloads"),
            Some((ProviderId::Claude, "sort my downloads"))
        );
        assert_eq!(
            forced_provider("  g  what's 2+2 "),
            Some((ProviderId::Chatgpt, "what's 2+2"))
        );
        assert_eq!(forced_provider("g "), None, "nothing to ask yet");
        assert_eq!(forced_provider("gh repo"), None);
        assert_eq!(forced_provider("chrome"), None);
        assert_eq!(forced_provider(r"c:\users"), None);
    }

    #[test]
    fn provider_names() {
        assert_eq!(ProviderId::Claude.name(), "Claude");
        assert_eq!(ProviderId::Chatgpt.name(), "ChatGPT");
    }
}
