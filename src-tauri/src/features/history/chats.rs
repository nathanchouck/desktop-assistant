//! Conversation history as one service: the store, the recorder following
//! the live chat, and a writer thread that does the saving, so recording
//! never makes streaming wait for the disk.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::AppHandle;

use super::recorder::{resume_plan, Meta, Recorder};
use super::store::{ConversationStore, Record, StoreError};
use super::transcript::{ChatEntry, Transcript};
use crate::ai::chat::{Chat, Resumable};
use crate::ai::{AiEvent, ProviderId};
use crate::settings::{self, SharedSettings};

/// How often the age limit is applied while nothing is saved.
const PRUNE_EVERY: Duration = Duration::from_secs(60 * 60);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// A saved conversation, reopened: the chat view shows it and a follow-up
/// carries it on (it's the chat's current conversation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct OpenedChat {
    /// The chat's id for it, as `ai_ask` would return.
    pub conversation: u32,
    pub provider: ProviderId,
    /// The model that answered last ("claude-sonnet-5-5"); may be empty.
    pub model: String,
    pub entries: Vec<ChatEntry>,
}

/// What the settings menu shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct HistoryOverview {
    pub conversations: u32,
    pub pinned: u32,
}

enum Job {
    /// Read the list into memory and apply the limits (at startup).
    Load,
    Save(Record),
    #[cfg(test)]
    Flush(mpsc::Sender<()>),
}

pub struct ChatHistory {
    store: Arc<ConversationStore>,
    recorder: Mutex<Recorder>,
    writer: mpsc::Sender<Job>,
    settings: SharedSettings,
    /// Set once the app runs; reopening needs the chat in its state.
    pub(super) app: OnceLock<AppHandle>,
}

impl ChatHistory {
    /// Starts the writer thread. The list is read once `load` is called.
    pub fn new(store: ConversationStore, settings: SharedSettings) -> Arc<Self> {
        let store = Arc::new(store);
        let (writer, jobs) = mpsc::channel();
        let thread_store = Arc::clone(&store);
        let thread_settings = Arc::clone(&settings);
        let spawned = std::thread::Builder::new()
            .name("chat-history".into())
            .spawn(move || write_jobs(&jobs, &thread_store, &thread_settings));
        if let Err(e) = spawned {
            tracing::warn!("chat history won't be saved: {e}");
        }
        Arc::new(Self {
            store,
            recorder: Mutex::new(Recorder::default()),
            writer,
            settings,
            app: OnceLock::new(),
        })
    }

    pub fn store(&self) -> &Arc<ConversationStore> {
        &self.store
    }

    /// Reads the saved list in the background and tidies it.
    pub fn load(&self) {
        let _ = self.writer.send(Job::Load);
    }

    pub fn enabled(&self) -> bool {
        settings::current(&self.settings).history.enabled
    }

    // --- Recording ---------------------------------------------------------

    /// A new conversation's first question (`text` without the desktop
    /// context; `context` is its "Sees: …" caption).
    pub fn begin(&self, conversation: u32, saved: Resumable, text: &str, context: Option<&str>) {
        if !self.enabled() {
            return;
        }
        let now = now_ms();
        let meta = Meta::new(self.store.new_id(), saved, text, now);
        let saves = self
            .recorder()
            .begin(conversation, meta, text, context, now);
        self.save_all(saves);
    }

    /// A follow-up in conversation `conversation`.
    pub fn question(&self, conversation: u32, text: &str, context: Option<&str>) {
        let save = self
            .recorder()
            .question(conversation, text, context, now_ms());
        self.save_all(save);
    }

    /// One streamed event. Cheap: the transcript is in memory and saving
    /// happens on the writer thread.
    pub fn event(&self, conversation: u32, event: &AiEvent) {
        let save = self.recorder().event(conversation, event, now_ms());
        self.save_all(save);
    }

    /// The chat was closed.
    pub fn finish(&self) {
        let saves = self.recorder().finish(now_ms());
        self.save_all(saves);
    }

    // --- The list ----------------------------------------------------------

    /// Removes one conversation; false if it was already gone.
    pub fn delete(&self, id: i64) -> Result<bool, StoreError> {
        // First, so a live conversation isn't saved back.
        self.recorder().forget(&[id]);
        self.store.delete(id)
    }

    /// Removes every unpinned conversation; returns how many.
    pub fn clear(&self) -> Result<usize, StoreError> {
        let unpinned: Vec<i64> = self
            .store
            .with_summaries(|list| list.iter().filter(|s| !s.pinned).map(|s| s.id).collect());
        self.recorder().forget(&unpinned);
        Ok(self.store.clear()?.len())
    }

    /// Applies the size and age limits now.
    pub fn prune(&self) -> Result<(), StoreError> {
        prune(&self.store, &self.settings)
    }

    pub fn overview(&self) -> HistoryOverview {
        let (conversations, pinned) = self.store.counts();
        HistoryOverview {
            conversations: u32::try_from(conversations).unwrap_or(u32::MAX),
            pinned: u32::try_from(pinned).unwrap_or(u32::MAX),
        }
    }

    // --- Reopening ---------------------------------------------------------

    /// Makes saved conversation `id` the chat's current one, idle: the
    /// next follow-up resumes it (Claude Code `--resume` in its folder,
    /// Codex `thread/resume`). Recording carries on into the same record.
    pub async fn reopen(&self, chat: &Chat, id: i64) -> Result<OpenedChat, String> {
        let store = Arc::clone(&self.store);
        let record = tokio::task::spawn_blocking(move || store.load(id))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?
            .ok_or("That chat isn't saved anymore.")?;
        let saved = Resumable::new(
            record.provider,
            PathBuf::from(&record.cwd),
            record.session.clone(),
        );
        let folder_exists = !record.cwd.is_empty() && Path::new(&record.cwd).is_dir();
        let home = dirs::home_dir().unwrap_or_default();
        let (resumable, note) = resume_plan(saved, folder_exists, home);
        // The same session handle goes to the chat and the recorder, so a
        // session id the resumed AI reports is saved too.
        let conversation = chat.adopt(resumable.clone()).await;
        let transcript = Transcript::from_saved(record.entries, record.model.clone());
        let mut entries = transcript.entries.clone();
        if self.enabled() {
            let meta = Meta {
                record: record.id,
                resumable,
                title: record.title,
                created_ms: record.created_ms,
            };
            let saves = self
                .recorder()
                .adopt(conversation, meta, transcript, now_ms());
            self.save_all(saves);
        }
        tracing::info!("reopened saved chat {id} as conversation {conversation}");
        if let Some(text) = note {
            entries.push(ChatEntry::Note { text });
        }
        Ok(OpenedChat {
            conversation,
            provider: record.provider,
            model: record.model,
            entries,
        })
    }

    fn save_all(&self, saves: impl IntoIterator<Item = Record>) {
        for record in saves {
            let _ = self.writer.send(Job::Save(record));
        }
    }

    fn recorder(&self) -> MutexGuard<'_, Recorder> {
        self.recorder.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Waits until everything sent to the writer is done.
    #[cfg(test)]
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();
        if self.writer.send(Job::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(5));
        }
    }
}

/// The writer thread: saves in order, and applies the limits after each
/// save and every hour.
fn write_jobs(jobs: &mpsc::Receiver<Job>, store: &ConversationStore, settings: &SharedSettings) {
    loop {
        let job = match jobs.recv_timeout(PRUNE_EVERY) {
            Ok(job) => job,
            Err(RecvTimeoutError::Timeout) => {
                if let Err(e) = prune(store, settings) {
                    tracing::warn!("could not tidy chat history: {e}");
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let result = match job {
            Job::Load => store.load_summaries().and_then(|()| prune(store, settings)),
            Job::Save(record) => {
                // Turned off since the conversation started: stop saving.
                if !settings::current(settings).history.enabled {
                    continue;
                }
                store.save(&record).and_then(|()| prune(store, settings))
            }
            #[cfg(test)]
            Job::Flush(done) => {
                let _ = done.send(());
                Ok(())
            }
        };
        if let Err(e) = result {
            tracing::warn!("chat history: {e}");
        }
    }
}

fn prune(store: &ConversationStore, settings: &SharedSettings) -> Result<(), StoreError> {
    let limits = settings::current(settings).history;
    let cutoff = (limits.forget_after_days > 0)
        .then(|| now_ms() - i64::from(limits.forget_after_days) * DAY_MS);
    let dropped = store.prune(limits.max_conversations.max(1), cutoff)?;
    if !dropped.is_empty() {
        tracing::info!("forgot {} old chats", dropped.len());
    }
    Ok(())
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::RwLock;
    use tokio::sync::mpsc as tokio_mpsc;

    fn history() -> (Arc<ChatHistory>, SharedSettings) {
        let settings: SharedSettings = Arc::new(RwLock::new(Settings::default()));
        let store = ConversationStore::in_memory().unwrap();
        (ChatHistory::new(store, Arc::clone(&settings)), settings)
    }

    fn here() -> Resumable {
        Resumable::new(ProviderId::Claude, std::env::temp_dir(), Some("s-1".into()))
    }

    fn titles(history: &ChatHistory) -> Vec<String> {
        history
            .store()
            .with_summaries(|l| l.iter().map(|s| s.title.clone()).collect())
    }

    #[test]
    fn records_a_conversation_as_it_goes() {
        let (h, _) = history();
        h.begin(1, here(), "what's 2+2?", Some("Desktop"));
        h.flush();
        assert_eq!(titles(&h), ["what's 2+2?"], "saved as soon as you ask");

        h.event(1, &AiEvent::TextDelta { text: "4".into() });
        h.event(1, &AiEvent::Done);
        h.flush();
        let id = h.store().latest().unwrap();
        let saved = h.store().load(id).unwrap().unwrap();
        assert_eq!(saved.session.as_deref(), Some("s-1"));
        assert_eq!(saved.entries.len(), 2);
        assert_eq!(
            h.overview(),
            HistoryOverview {
                conversations: 1,
                pinned: 0
            }
        );
    }

    #[test]
    fn nothing_is_recorded_while_off() {
        let (h, settings) = history();
        settings.write().unwrap().history.enabled = false;
        h.begin(1, here(), "secret", None);
        h.event(1, &AiEvent::Done);
        h.finish();
        h.flush();
        assert!(titles(&h).is_empty());

        // Turned off mid-chat: later saves are dropped too.
        settings.write().unwrap().history.enabled = true;
        h.begin(2, here(), "first", None);
        h.flush();
        settings.write().unwrap().history.enabled = false;
        h.question(2, "second", None);
        h.flush();
        let id = h.store().latest().unwrap();
        assert_eq!(h.store().load(id).unwrap().unwrap().entries.len(), 1);
    }

    #[test]
    fn keeps_to_the_limits() {
        let (h, settings) = history();
        settings.write().unwrap().history.max_conversations = 2;
        for (i, title) in ["one", "two", "three"].iter().enumerate() {
            h.begin(i as u32 + 1, here(), title, None);
            h.flush();
        }
        assert_eq!(titles(&h), ["three", "two"]);
    }

    #[test]
    fn deleting_the_live_conversation_stops_recording_it() {
        let (h, _) = history();
        h.begin(1, here(), "oops", None);
        h.flush();
        let id = h.store().latest().unwrap();
        assert!(h.delete(id).unwrap());
        h.event(1, &AiEvent::Done);
        h.finish();
        h.flush();
        assert!(titles(&h).is_empty());

        h.begin(2, here(), "a", None);
        h.begin(3, here(), "b", None);
        h.flush();
        let pinned = h.store().latest().unwrap();
        h.store().toggle_pin(pinned).unwrap();
        assert_eq!(h.clear().unwrap(), 1);
        h.question(3, "still pinned, still recorded", None);
        h.flush();
        assert_eq!(titles(&h), ["b"]);
        assert_eq!(h.store().load(pinned).unwrap().unwrap().entries.len(), 2);
    }

    #[tokio::test]
    async fn reopens_a_saved_conversation_where_it_left_off() {
        let (h, _) = history();
        h.begin(1, here(), "hi", None);
        h.event(
            1,
            &AiEvent::Started {
                model: "claude-haiku-5-5".into(),
            },
        );
        h.event(
            1,
            &AiEvent::TextDelta {
                text: "Hello".into(),
            },
        );
        h.event(1, &AiEvent::Done);
        h.finish();
        h.flush();
        let id = h.store().latest().unwrap();

        let (tx, _rx) = tokio_mpsc::unbounded_channel();
        let chat = Chat::new(tx);
        let opened = h.reopen(&chat, id).await.unwrap();
        assert_eq!(opened.provider, ProviderId::Claude);
        assert_eq!(opened.model, "claude-haiku-5-5");
        assert_eq!(opened.entries.len(), 2);
        assert_eq!(chat.provider().await, Some(ProviderId::Claude));
        let resumable = chat.resumable(opened.conversation).await.unwrap();
        assert_eq!(resumable.session().as_deref(), Some("s-1"));

        // A follow-up is recorded into the same saved chat.
        h.question(opened.conversation, "and you?", None);
        h.flush();
        assert_eq!(h.store().load(id).unwrap().unwrap().entries.len(), 3);
        assert_eq!(h.overview().conversations, 1);

        assert!(h.reopen(&chat, 999).await.is_err());
    }

    #[tokio::test]
    async fn a_chat_whose_folder_is_gone_says_so() {
        let (h, _) = history();
        let gone = Resumable::new(
            ProviderId::Claude,
            "/no/such/folder".into(),
            Some("s".into()),
        );
        h.begin(1, gone, "hi", None);
        h.finish();
        h.flush();
        let id = h.store().latest().unwrap();
        let (tx, _rx) = tokio_mpsc::unbounded_channel();
        let chat = Chat::new(tx);
        let opened = h.reopen(&chat, id).await.unwrap();
        assert!(matches!(
            opened.entries.last(),
            Some(ChatEntry::Note { text }) if text.contains("/no/such/folder")
        ));
        let resumable = chat.resumable(opened.conversation).await.unwrap();
        assert_eq!(resumable.session(), None, "Claude starts over");
    }
}
