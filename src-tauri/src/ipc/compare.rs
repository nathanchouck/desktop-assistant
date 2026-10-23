//! Asking Claude and ChatGPT the same question side by side: the
//! `ai_compare*` commands and the search row that offers it. Both AIs get
//! the same question, attachments, desktop context and working folder
//! (each AI's own screen-context switch still applies) and run as two live
//! conversations in `Chat`. History records each as its own chat.

use serde::Serialize;
use tauri::{AppHandle, State};

use super::{ai_err, chat};
use crate::ai::chat::Setup;
use crate::ai::claude::ClaudeStatus;
use crate::ai::codex::CodexStatus;
use crate::ai::message::Message;
use crate::ai::{self, AiError, ProviderId};
use crate::desktop::DesktopContext;
use crate::error::IpcError;
use crate::features::ai_commands::Attachment;
use crate::features::history;
use crate::search::{self, glyph, Action, AskRow, ItemKind, ResultItem};
use crate::state::AppState;
use crate::window;

/// Who answers, left to right (Ctrl+1 and Ctrl+2 carry on with one).
const SIDES: [ProviderId; 2] = [ProviderId::Claude, ProviderId::Chatgpt];
/// The comparison row's id; the UI knows it (`COMPARE_ROW_ID`).
const ROW_ID: &str = "ai-compare";

/// One AI's part of a comparison.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct CompareSide {
    pub provider: ProviderId,
    /// Its conversation; `None` when it couldn't be started (`error` says
    /// why).
    pub conversation: Option<u32>,
    /// What it was told about the screen, in a few words (as
    /// `AskReply::context`).
    pub context: Option<String>,
    /// Why this AI didn't get the question.
    pub error: Option<String>,
}

/// A question sent to both AIs.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct CompareReply {
    /// One per AI, Claude first.
    pub sides: Vec<CompareSide>,
}

impl CompareSide {
    fn asked(provider: ProviderId, conversation: u32, context: Option<String>) -> Self {
        Self {
            provider,
            conversation: Some(conversation),
            context,
            error: None,
        }
    }

    fn failed(provider: ProviderId, conversation: Option<u32>, error: String) -> Self {
        Self {
            provider,
            conversation,
            context: None,
            error: Some(error),
        }
    }
}

/// Asks Claude and ChatGPT the same question at once, side by side,
/// ending any current chat. An AI that can't be asked (not installed, an
/// attachment gone) says why in its side; the other still answers.
#[tauri::command]
#[specta::specta]
pub async fn ai_compare(
    state: State<'_, AppState>,
    text: String,
    attachments: Vec<Attachment>,
) -> Result<CompareReply, IpcError> {
    let context = chat::context_for_any(&state, &SIDES, true).await;
    let settings = state.settings().ai;
    let mut asking = Vec::new();
    let mut ready = Vec::new();
    let mut sides = Vec::new();
    for provider in SIDES {
        let seen = context
            .as_ref()
            .filter(|_| settings.desktop_context(provider));
        match prepare(&state, provider, &text, &attachments, seen).await {
            Ok(side) => {
                asking.push((provider, chat::caption(&attachments, seen)));
                ready.push(side);
            }
            Err(message) => sides.push(CompareSide::failed(provider, None, message)),
        }
    }
    if !ready.is_empty() {
        let started = state.chat.compare(ready, history::comparing).await;
        for ((provider, caption), result) in asking.into_iter().zip(started) {
            sides.push(match result {
                Ok(conversation) => {
                    history::asked(&state.chat, conversation, &text, caption.as_deref(), true)
                        .await;
                    CompareSide::asked(provider, conversation, caption)
                }
                Err(e) => CompareSide::failed(provider, None, ai_err(e).message),
            });
        }
    }
    sides.sort_by_key(|side| SIDES.iter().position(|&p| p == side.provider));
    Ok(CompareReply { sides })
}

/// A side's first message and setup (Explorer's folder, if it may see it).
async fn prepare(
    state: &AppState,
    provider: ProviderId,
    text: &str,
    attachments: &[Attachment],
    context: Option<&DesktopContext>,
) -> Result<(Message, Setup), String> {
    let setup = chat::new_conversation(state, provider, context).map_err(|e| ai_err(e).message)?;
    let message = chat::message(state, text, attachments, context)
        .await
        .map_err(|e| e.message)?;
    Ok((message, setup))
}

/// Sends a follow-up to both sides of the comparison (each AI that's still
/// in it), with the desktop context if it changed and any attachments.
#[tauri::command]
#[specta::specta]
pub async fn ai_compare_follow_up(
    state: State<'_, AppState>,
    text: String,
    attachments: Vec<Attachment>,
) -> Result<CompareReply, IpcError> {
    let live = state.chat.conversations().await;
    if live.is_empty() {
        return Err(ai_err(AiError::NoConversation));
    }
    let providers: Vec<ProviderId> = live.iter().map(|&(_, provider)| provider).collect();
    let context = chat::context_for_any(&state, &providers, false).await;
    let settings = state.settings().ai;
    let mut sides = Vec::new();
    for (id, provider) in live {
        let seen = context
            .as_ref()
            .filter(|_| settings.desktop_context(provider));
        let sent = async {
            let message = chat::message(&state, &text, &attachments, seen)
                .await
                .map_err(|e| e.message)?;
            state
                .chat
                .follow_up_in(id, message, |provider| chat::setup_for(&state, provider))
                .await
                .map_err(|e| ai_err(e).message)
        }
        .await;
        let caption = chat::caption(&attachments, seen);
        sides.push(match sent {
            Ok(conversation) => {
                history::asked(&state.chat, conversation, &text, caption.as_deref(), false).await;
                CompareSide::asked(provider, conversation, caption)
            }
            Err(message) => CompareSide::failed(provider, Some(id), message),
        });
    }
    Ok(CompareReply { sides })
}

/// Carries on with one side of the comparison as a normal chat; the other
/// AI's conversation ends (it stays in history).
#[tauri::command]
#[specta::specta]
pub async fn ai_compare_keep(state: State<'_, AppState>, conversation: u32) -> Result<(), IpcError> {
    state.chat.keep(conversation).await.map_err(ai_err)?;
    history::kept(conversation);
    Ok(())
}

/// Starts both AIs ahead of a comparison you seem to be typing (each one
/// that starts early and can answer), so their answers start sooner.
#[tauri::command]
#[specta::specta]
pub async fn ai_compare_warm(state: State<'_, AppState>) -> Result<(), IpcError> {
    let settings = state.settings().ai;
    let providers: Vec<ProviderId> = SIDES
        .into_iter()
        .filter(|&p| settings.warm_start(p) && chat::ready(&state, p))
        .collect();
    if providers.is_empty() {
        return Ok(());
    }
    let context = if providers.iter().any(|&p| settings.desktop_context(p)) {
        Some(state.desktop.current(chat::CONTEXT_WAIT).await)
    } else {
        None
    };
    let mut setups = Vec::new();
    for provider in providers {
        let seen = context
            .as_ref()
            .filter(|_| settings.desktop_context(provider));
        match chat::new_conversation(&state, provider, seen) {
            Ok(setup) => setups.push(setup),
            Err(e) => tracing::debug!("could not warm up {}: {e}", provider.name()),
        }
    }
    if let Err(e) = state.chat.warm_all(setups).await {
        tracing::debug!("could not warm up for a comparison: {e}");
    }
    Ok(())
}

/// Widens the launcher while two answers are side by side, or brings it
/// back to its usual width.
#[tauri::command]
#[specta::specta]
pub fn launcher_set_wide(app: AppHandle, wide: bool) {
    window::set_wide(&app, wide);
}

/// `both <question>` (or `vs …`): the comparison row alone, or the fix
/// one of the AIs needs first (install, sign in). `None` without the
/// prefix.
pub(super) fn prefixed_rows(
    app: &AppHandle,
    state: &AppState,
    query: &str,
) -> Option<Vec<ResultItem>> {
    let question = ai::compare_prefix(query)?;
    crate::check_claude_status(app, false);
    crate::check_codex_status(app, false);
    let fix = SIDES
        .into_iter()
        .map(|provider| chat::ask_row(state, provider))
        .find(|row| !matches!(row, AskRow::Ask(_)));
    Some(vec![match fix {
        Some(fix) => search::ask_ai_item(question, fix),
        None => row(question),
    }])
}

/// The comparison row offered under the "Ask" row, while both AIs are
/// installed and signed in.
pub(super) fn offered_row(state: &AppState, query: &str) -> Option<ResultItem> {
    let query = query.trim();
    let both_ready = matches!(state.claude_status.get(), ClaudeStatus::Ready)
        && matches!(state.codex_status.get(), CodexStatus::Ready);
    (both_ready && !query.is_empty()).then(|| row(query))
}

/// "Compare Claude and ChatGPT: <question>". Enter (or Ctrl+Tab anywhere
/// in search) asks both.
fn row(question: &str) -> ResultItem {
    ResultItem::new(
        ROW_ID,
        ItemKind::AskAi,
        format!("Compare Claude and ChatGPT: {question}"),
    )
    .with_subtitle("Ctrl Tab")
    .with_icon(glyph("compare"))
    .with_actions([Action::AskAi])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_row_asks_both_ais() {
        let item = row("which is faster?");
        assert_eq!(item.id, "ai-compare");
        assert_eq!(item.kind, ItemKind::AskAi);
        assert_eq!(item.title, "Compare Claude and ChatGPT: which is faster?");
        assert_eq!(item.icon.as_deref(), Some("glyph:compare"));
        assert_eq!(item.actions, [Action::AskAi]);
    }

    #[test]
    fn claude_answers_on_the_left() {
        assert_eq!(SIDES, [ProviderId::Claude, ProviderId::Chatgpt]);
    }
}
