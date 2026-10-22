//! Chat history in search: `history`, `>history` or `chats` lists past
//! conversations, newest first with pinned ones on top; text after it
//! filters. Enter reopens one to carry on.

use std::sync::Arc;

use async_trait::async_trait;
use tauri::Manager;

use super::chats::{now_ms, ChatHistory};
use super::rows::{self, CLEAR_ID, EMPTY_ID, LIST_QUERY, OFF_ID, OPEN_ID, RESUME_ID};
use crate::fuzzy::FuzzyQuery;
use crate::search::{Action, Activation, ResultItem, SearchError, SearchProvider};
use crate::state::AppState;

const NAME: &str = "history";

pub struct HistoryProvider {
    history: Arc<ChatHistory>,
}

impl HistoryProvider {
    pub fn new(history: Arc<ChatHistory>) -> Self {
        Self { history }
    }

    /// Reopens saved conversation `id` in the launcher's chat.
    async fn reopen(&self, id: i64) -> Result<Activation, SearchError> {
        let app = self
            .history
            .app
            .get()
            .ok_or_else(|| failed("the launcher isn't ready yet"))?;
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| failed("the launcher isn't ready yet"))?;
        // The resumed AI saw an older screen: its next question sends
        // today's desktop context.
        if let Ok(mut sent) = state.sent_context.lock() {
            *sent = None;
        }
        let chat = self.history.reopen(&state.chat, id).await.map_err(failed)?;
        Ok(Activation::OpenChat { chat })
    }

}

fn failed(message: impl ToString) -> SearchError {
    SearchError::Provider {
        provider: NAME,
        message: message.to_string(),
    }
}

#[async_trait]
impl SearchProvider for HistoryProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let store = self.history.store();
        let Some(filter) = rows::history_filter(query) else {
            // Elsewhere, only the commands show up.
            let latest = store.latest();
            return Ok(store.with_summaries(|list| {
                let latest = list.iter().find(|s| Some(s.id) == latest);
                rows::command_rows(query, latest)
            }));
        };
        let enabled = self.history.enabled();
        let now = now_ms();
        let mut found =
            store.with_summaries(|list| rows::conversation_rows(list, filter, now, limit));
        if filter.is_empty() {
            if found.is_empty() {
                found.push(rows::empty_row(enabled));
            } else if !enabled {
                found.push(rows::off_row());
            }
        } else if let Some(score) = FuzzyQuery::new(filter).score("Clear Chat History") {
            found.push(rows::clear_row().with_score(score));
        }
        Ok(found)
    }

    fn claims(&self, query: &str) -> bool {
        rows::history_filter(query).is_some()
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(rows::PREFIX)
    }

    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError> {
        match id {
            OPEN_ID => {
                return Ok(Activation::SetQuery {
                    query: LIST_QUERY.into(),
                })
            }
            RESUME_ID => {
                let Some(latest) = self.history.store().latest() else {
                    return Ok(Activation::Notice {
                        message: "No saved chats yet.".into(),
                    });
                };
                return self.reopen(latest).await;
            }
            CLEAR_ID => {
                let removed = self.history.clear().await.map_err(failed)?;
                tracing::info!("cleared {removed} saved chats");
                return Ok(Activation::SetQuery {
                    query: LIST_QUERY.into(),
                });
            }
            OFF_ID => return Ok(Activation::OpenSettings),
            EMPTY_ID => {
                return Ok(Activation::Notice {
                    message: "Ask Claude or ChatGPT something and it shows up here.".into(),
                })
            }
            _ => {}
        }
        let entry =
            rows::parse_entry_id(id).ok_or_else(|| failed(format!("{id} can't be opened")))?;
        match action {
            Action::Pin => {
                self.history.toggle_pin(entry).await.map_err(failed)?;
                Ok(Activation::Refresh)
            }
            Action::Delete => {
                self.history.delete(entry).await.map_err(failed)?;
                Ok(Activation::Refresh)
            }
            _ => self.reopen(entry).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat::Resumable;
    use crate::ai::{AiEvent, ProviderId};
    use crate::features::history::store::ConversationStore;
    use crate::search::ItemKind;
    use crate::settings::{Settings, SharedSettings};
    use std::sync::RwLock;

    fn provider() -> (HistoryProvider, SharedSettings) {
        let settings: SharedSettings = Arc::new(RwLock::new(Settings::default()));
        let store = ConversationStore::in_memory().unwrap();
        let history = ChatHistory::new(store, Arc::clone(&settings));
        (HistoryProvider::new(history), settings)
    }

    fn chat(p: &HistoryProvider, conversation: u32, question: &str, answer: &str) {
        let here = Resumable::new(ProviderId::Chatgpt, std::env::temp_dir(), None);
        p.history.begin(conversation, here, question, None);
        p.history.event(
            conversation,
            &AiEvent::TextDelta {
                text: answer.into(),
            },
        );
        p.history.event(conversation, &AiEvent::Done);
        p.history.flush();
    }

    fn ids(rows: &[ResultItem]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[tokio::test]
    async fn lists_and_filters_saved_chats() {
        let (p, _) = provider();
        assert!(p.claims("history") && p.claims(">history x") && p.claims("chats"));
        assert!(!p.claims("chat") && !p.claims("histogram"));
        assert_eq!(ids(&p.query("history", 32).await.unwrap()), [EMPTY_ID]);

        chat(&p, 1, "Plan a trip to Kyoto", "Go in April.");
        chat(&p, 2, "Sort my downloads", "Moved 3 files.");
        let rows = p.query("history", 32).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "Sort my downloads", "newest first");
        assert_eq!(rows[0].kind, ItemKind::Conversation);
        assert_eq!(rows[0].subtitle, "ChatGPT · just now");
        assert_eq!(rows[0].accessory.as_deref(), Some("2 messages"));
        assert!(rows.iter().all(|r| p.owns(&r.id)));

        let rows = p.query(">history april", 32).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Plan a trip to Kyoto");
        let rows = p.query("chats clear", 32).await.unwrap();
        assert!(rows.iter().any(|r| r.id == CLEAR_ID && r.confirm.is_some()));
        assert!(p.query("chats zzz", 32).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn offers_its_commands_in_normal_search() {
        let (p, _) = provider();
        chat(&p, 1, "Sort my downloads", "Done.");
        let rows = p.query("resume last", 32).await.unwrap();
        assert_eq!(ids(&rows), [RESUME_ID]);
        assert!(rows[0].subtitle.starts_with("Sort my downloads"));
        let rows = p.query("chat history", 32).await.unwrap();
        assert!(ids(&rows).contains(&OPEN_ID));
        assert_eq!(
            p.activate(OPEN_ID, Action::Run).await.unwrap(),
            Activation::SetQuery {
                query: LIST_QUERY.into()
            }
        );
    }

    #[tokio::test]
    async fn pins_deletes_and_clears() {
        let (p, _) = provider();
        chat(&p, 1, "older", "a");
        chat(&p, 2, "newer", "b");
        let rows = p.query("history", 32).await.unwrap();
        let older = rows[1].id.clone();

        assert_eq!(
            p.activate(&older, Action::Pin).await.unwrap(),
            Activation::Refresh
        );
        let rows = p.query("history", 32).await.unwrap();
        assert_eq!(rows[0].id, older, "pinned on top");
        assert!(rows[0].subtitle.ends_with("Pinned"));

        assert_eq!(
            p.activate(CLEAR_ID, Action::Run).await.unwrap(),
            Activation::SetQuery {
                query: LIST_QUERY.into()
            }
        );
        assert_eq!(
            ids(&p.query("history", 32).await.unwrap()),
            [older.as_str()]
        );
        assert_eq!(
            p.activate(&older, Action::Delete).await.unwrap(),
            Activation::Refresh
        );
        assert_eq!(ids(&p.query("history", 32).await.unwrap()), [EMPTY_ID]);
        assert!(matches!(
            p.activate(RESUME_ID, Action::Run).await.unwrap(),
            Activation::Notice { .. }
        ));
        assert!(p.activate("history:nope", Action::Open).await.is_err());
    }

    #[tokio::test]
    async fn says_when_history_is_off() {
        let (p, settings) = provider();
        settings.write().unwrap().history.enabled = false;
        assert_eq!(ids(&p.query("history", 32).await.unwrap()), [OFF_ID]);
        assert_eq!(
            p.activate(OFF_ID, Action::Run).await.unwrap(),
            Activation::OpenSettings
        );
        // What was saved before stays listed, with a reminder at the end.
        settings.write().unwrap().history.enabled = true;
        chat(&p, 1, "kept", "yes");
        settings.write().unwrap().history.enabled = false;
        let rows = p.query("history", 32).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].id, OFF_ID);
    }
}
