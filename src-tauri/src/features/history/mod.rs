//! Conversation history: every chat with Claude or ChatGPT is saved in its
//! own SQLite file (`history.db` in the data folder), listed by `history`
//! (or `>history`, `chats`), and reopened to carry on where it left off.
//!
//! - `transcript.rs`: a chat as the chat pane shows it, built from events.
//! - `recorder.rs`: follows the live chat, says what to save when.
//! - `store.rs`: the database, with a summary of each chat in memory.
//! - `chats.rs`: the three together, a writer thread, and reopening.
//! - `claude_sessions.rs`: whether Claude Code still has a session.
//! - `rows.rs` / `provider.rs`: search rows and what Enter, pin and delete do.
//! - `commands.rs`: the settings menu's count and Clear, and the chat's
//!   copy / paste of the last answer.
//!
//! The `ai_*` commands report each question (`asked`), the chat event loop
//! each event (`observe`), and closing the chat (`closed`).

mod chats;
mod claude_sessions;
pub mod commands;
mod provider;
mod recorder;
mod rows;
mod store;
mod transcript;

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::ProviderContext;
use crate::ai::chat::Chat;
use crate::ai::AiEvent;
use crate::paths::AppPaths;
use crate::search::SearchProvider;
use crate::settings::SharedSettings;
use crate::state::AppState;
pub use chats::{ChatHistory, HistoryOverview, OpenedChat};
pub use transcript::{ApprovalState, ChatEntry, ToolState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct HistorySettings {
    /// Save chats. Off: nothing new is saved; what's saved stays until
    /// cleared.
    pub enabled: bool,
    /// Chats kept, not counting pinned ones; the oldest go first.
    pub max_conversations: u32,
    /// Chats not used for this many days are forgotten (0: never).
    pub forget_after_days: u32,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_conversations: 500,
            forget_after_days: 0,
        }
    }
}

/// The history, shared by the search provider (made while the app state is
/// built), the `ai_*` commands and the chat event loop. `None` if not even
/// an in-memory database would open.
static HISTORY: OnceLock<Option<Arc<ChatHistory>>> = OnceLock::new();

fn open(paths: &AppPaths, settings: &SharedSettings) -> Option<Arc<ChatHistory>> {
    HISTORY
        .get_or_init(|| {
            let store = store::ConversationStore::open(&paths.data_dir.join("history.db"))
                .or_else(|e| {
                    crate::toast::report(format!("Chats won't be saved this session: {e}"));
                    store::ConversationStore::in_memory()
                })
                .map_err(|e| crate::toast::report(format!("Chat history is off: {e}")))
                .ok()?;
            Some(ChatHistory::new(store, Arc::clone(settings)))
        })
        .clone()
}

/// The history, once the app state has opened it.
pub fn get() -> Option<Arc<ChatHistory>> {
    HISTORY.get().cloned().flatten()
}

/// Saved chats in search (`history`).
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    let history = open(&ctx.paths, &ctx.settings)?;
    Some(Arc::new(provider::HistoryProvider::new(history)))
}

/// Reads the saved list (in the background) and lets reopening reach the chat.
pub fn start(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some(history) = open(&state.paths, &state.settings) else {
        return;
    };
    let _ = history.app.set(app.clone());
    history.load();
}

/// A question was sent in `conversation`: its first (`new`), or a
/// follow-up. `text` is what you typed (without the desktop context);
/// `context` the "Sees: …" caption.
pub async fn asked(chat: &Chat, conversation: u32, text: &str, context: Option<&str>, new: bool) {
    let Some(history) = get() else {
        return;
    };
    if !new {
        history.question(conversation, text, context);
    } else if let Some(saved) = chat.resumable(conversation).await {
        history.begin(conversation, saved, text, context);
    }
}

/// An event on its way to the UI.
pub fn observe(conversation: u32, event: &AiEvent) {
    if let Some(history) = get() {
        history.event(conversation, event);
    }
}

/// The chat was closed: saves what the conversation had unsaved.
pub fn closed() {
    if let Some(history) = get() {
        history.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_500_chats_forever() {
        let s = HistorySettings::default();
        assert!(s.enabled);
        assert_eq!(s.max_conversations, 500);
        assert_eq!(s.forget_after_days, 0);
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let s = HistorySettings {
            enabled: false,
            max_conversations: 50,
            forget_after_days: 30,
        };
        let text = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<HistorySettings>(&text).unwrap(), s);
    }
}
