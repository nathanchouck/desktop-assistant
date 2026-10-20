//! AI providers (Claude Code, Codex). Adapters arrive in milestones 4 and 5;
//! this module holds the shared types and the trait they implement.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

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
    /// env var is stripped from the child process so it can't take over.
    #[default]
    Subscription,
    /// Inject an API key from Windows Credential Manager.
    ApiKey,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct ProviderStatus {
    pub installed: bool,
    pub signed_in: bool,
    pub billing: BillingMode,
    /// Human-readable problem, if the provider can't be used right now.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, specta::Type)]
pub struct ConversationId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, specta::Type)]
pub struct ApprovalId(pub u32);

/// What an adapter emits while answering. Sent to the UI as events.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiEvent {
    TextDelta {
        text: String,
    },
    ToolStarted {
        name: String,
        summary: String,
    },
    ApprovalRequested {
        id: ApprovalId,
        tool: String,
        detail: String,
    },
    Done,
    Error {
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("{0} is not installed")]
    NotInstalled(&'static str),
    #[error("{0} is not signed in")]
    NotSignedIn(&'static str),
    #[error("provider process failed: {0}")]
    Process(String),
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    async fn status(&self) -> ProviderStatus;
    /// Sends a prompt; events stream into `tx` until `AiEvent::Done` or `Error`.
    async fn send(
        &self,
        conversation: ConversationId,
        prompt: String,
        tx: mpsc::Sender<AiEvent>,
    ) -> Result<(), AiError>;
    async fn respond_approval(&self, id: ApprovalId, allow: bool) -> Result<(), AiError>;
    async fn cancel(&self, conversation: ConversationId) -> Result<(), AiError>;
}
