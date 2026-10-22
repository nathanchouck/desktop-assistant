//! Conversation history as one service: the store, the recorder following
//! the live chat, and a writer thread that does the saving, so recording
//! never makes streaming wait for the disk.
//!
//! Everything that reads or changes the database goes through that thread,
//! in order: a delete can't be undone by an older save still waiting, and a
//! reopened chat is read after its last save.

use std::collections::HashSet;
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
const UNAVAILABLE: &str = "Chat history isn't available.";

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

type Work = Box<dyn FnOnce(&ConversationStore) + Send>;

enum Job {
    /// Read the list into memory and apply the limits (at startup).
    Load,
    Save(Record),
    /// Any other read or change, in order with the saves.
    Run(Work),
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
        let saves = self.recorder().event(conversation, event, now_ms());
        self.save_all(saves);
    }

    /// The chat was closed.
    pub fn finish(&self) {
        let saves = self.recorder().finish(now_ms());
        self.save_all(saves);
    }

    // --- The list ----------------------------------------------------------

    /// Removes one conversation; false if it was already gone.
    pub async fn delete(&self, id: i64) -> Result<bool, String> {
        // First, so a live conversation isn't saved back.
        self.recorder().forget(|record| record == id);
        self.in_order(move |store| store.delete(id)).await
    }

    /// Removes every unpinned conversation, the live one too; returns how
    /// many.
    pub async fn clear(&self) -> Result<usize, String> {
        let pinned: HashSet<i64> = self
            .store
            .with_summaries(|list| list.iter().filter(|s| s.pinned).map(|s| s.id).collect());
        // A new conversation whose first save is still queued isn't listed
        // yet, and isn't pinned either.
        self.recorder().forget(|record| !pinned.contains(&record));
        self.in_order(|store| store.clear().map(|ids| ids.len()))
            .await
    }

    /// Pins or unpins; returns the new state, or `None` if it's gone.
    pub async fn toggle_pin(&self, id: i64) -> Result<Option<bool>, String> {
        self.in_order(move |store| store.toggle_pin(id)).await
    }

    /// Applies the size and age limits now (a lowered limit shows at once).
    pub async fn tidy(&self) -> Result<(), String> {
        let settings = Arc::clone(&self.settings);
        self.in_order(move |store| prune(store, &settings)).await
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
        let record = self
            .in_order(move |store| store.load(id))
            .await?
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

    /// Runs `work` on the writer thread, after everything queued before it.
    async fn in_order<T: Send + 'static>(
        &self,
        work: impl FnOnce(&ConversationStore) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, String> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        let job = Job::Run(Box::new(move |store| {
            let _ = reply.send(work(store));
        }));
        self.writer.send(job).map_err(|_| UNAVAILABLE.to_owned())?;
        answer
            .await
            .map_err(|_| UNAVAILABLE.to_owned())?
            .map_err(|e| e.to_string())
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
        let job = Job::Run(Box::new(move |_| {
            let _ = done.send(());
        }));
        if self.writer.send(job).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(5));
        }
    }
}

/// The writer thread: runs jobs in order, and applies the limits after
/// each save and every hour.
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
            Job::Run(work) => {
                work(store);
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
mod tests;
