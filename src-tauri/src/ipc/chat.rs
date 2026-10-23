//! What the `ai_*` commands share: a conversation's setup for either AI,
//! the desktop context to send, and which row stands for asking it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ai::chat::Setup;
use crate::ai::claude::ClaudeStatus;
use crate::ai::message::Message;
use crate::ai::codex::{self, CodexStatus};
use crate::ai::{claude, AiError, ProviderId};
use crate::desktop::DesktopContext;
use crate::error::IpcError;
use crate::features::ai_commands::{self, Attachment};
use crate::integrations;
use crate::search::AskRow;
use crate::secrets::Secret;
use crate::state::AppState;

/// How long asking waits for File Explorer's folder and selection.
pub(super) const CONTEXT_WAIT: Duration = Duration::from_millis(800);

/// The desktop context to send now: `None` when it's off for `provider`,
/// or when it's the same as the last one sent in this conversation
/// (`fresh: false`).
pub(super) async fn context_to_send(
    state: &AppState,
    provider: ProviderId,
    fresh: bool,
) -> Option<DesktopContext> {
    if !state.settings().ai.desktop_context(provider) {
        return None;
    }
    let context = state.desktop.current(CONTEXT_WAIT).await;
    context.window.as_ref()?;
    let mut sent = state.sent_context.lock().ok()?;
    if !fresh && sent.as_ref() == Some(&context) {
        return None;
    }
    *sent = Some(context.clone());
    Some(context)
}

/// Today's setup for `provider`: its settings, the launcher's tools and the
/// MCP registry's servers (with secrets from Credential Manager).
pub(super) fn setup_for(state: &AppState, provider: ProviderId) -> Result<Setup, AiError> {
    let settings = state.settings();
    let tools = state.tool_files.get();
    Ok(match provider {
        ProviderId::Claude => {
            let (launch, mut options) = claude::prepare(&settings.ai.claude)?;
            let servers = integrations::servers_now(&settings.mcp);
            // Written now, so a change made since (settings synced from
            // another PC, a secret) is in the file this process reads.
            options.mcp_config = integrations::claude_config_file(
                &state.paths.data_dir,
                tools.map(|files| &files.endpoint),
                &servers,
            );
            options.mcp_servers = servers;
            options.settings = tools.map(|files| files.settings.clone());
            Setup::Claude(launch, options)
        }
        ProviderId::Chatgpt => {
            let (launch, mut options) = codex::prepare(&settings.ai.chatgpt)?;
            options.mcp = tools.map(|files| codex::McpServer {
                url: files.endpoint.url(),
                token: Secret::new(files.endpoint.token.clone()),
            });
            options.servers = integrations::servers_now(&settings.mcp);
            Setup::Codex(launch, options)
        }
    })
}

/// A new conversation's setup: Explorer's folder (from `context`) becomes
/// the working folder.
pub(super) fn new_conversation(
    state: &AppState,
    provider: ProviderId,
    context: Option<&DesktopContext>,
) -> Result<Setup, AiError> {
    let mut setup = setup_for(state, provider)?;
    if let Some(folder) = context.and_then(DesktopContext::folder) {
        if Path::new(folder).is_dir() {
            setup.set_cwd(PathBuf::from(folder));
        }
    }
    Ok(setup)
}

/// Whether `provider` looks able to answer (an unchecked one is assumed so).
pub(super) fn ready(state: &AppState, provider: ProviderId) -> bool {
    match provider {
        ProviderId::Claude => matches!(
            state.claude_status.get(),
            ClaudeStatus::Ready | ClaudeStatus::Unknown
        ),
        ProviderId::Chatgpt => matches!(
            state.codex_status.get(),
            CodexStatus::Ready | CodexStatus::Unknown
        ),
    }
}

/// The last search row for `provider`: "Ask …", or the fix that's needed
/// first (install, sign in).
pub(super) fn ask_row(state: &AppState, provider: ProviderId) -> AskRow {
    match provider {
        ProviderId::Claude => match state.claude_status.get() {
            ClaudeStatus::NotInstalled => AskRow::InstallClaude,
            ClaudeStatus::SignedOut => AskRow::SignInToClaude,
            _ => AskRow::Ask(provider),
        },
        ProviderId::Chatgpt => match state.codex_status.get() {
            CodexStatus::NotInstalled => AskRow::InstallCodex,
            CodexStatus::SignedOut => AskRow::SignInToChatgpt,
            _ => AskRow::Ask(provider),
        },
    }
}

/// A question's message: the desktop context, the attachments' blocks,
/// the question, and the attached pictures.
pub(super) async fn message(
    state: &AppState,
    text: &str,
    attachments: &[Attachment],
    context: Option<&DesktopContext>,
) -> Result<Message, IpcError> {
    ai_commands::message(&state.paths.data_dir, text, attachments, context)
        .await
        .map_err(|message| IpcError { message })
}

/// The "Sees: …" line under the question: the attachments, then the screen.
pub(super) fn caption(
    attachments: &[Attachment],
    context: Option<&DesktopContext>,
) -> Option<String> {
    let screen = context.and_then(DesktopContext::summary);
    ai_commands::attachments::caption(attachments, screen.as_deref())
}
