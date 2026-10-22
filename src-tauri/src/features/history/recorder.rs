//! Follows the live conversations and says when to save them. It's fed the
//! questions (from the `ai_*` commands) and the events (from the chat
//! event loop) and returns what to save; `mod.rs` hands that to the writer
//! thread, so streaming never waits for the disk.
//!
//! A conversation is saved when you ask (so the question is never lost),
//! when a turn ends, and when it's closed mid-turn. A crash loses at most
//! the answer in progress.

use std::collections::HashMap;
use std::path::PathBuf;

use super::store::Record;
use super::transcript::{self, Transcript};
use crate::ai::chat::Resumable;
use crate::ai::{AiEvent, ProviderId};

/// What's known about a recorded conversation besides its transcript.
pub struct Meta {
    /// Its row in the history database.
    pub record: i64,
    /// Provider, folder and session id (filled in once the AI reports it).
    pub resumable: Resumable,
    pub title: String,
    pub created_ms: i64,
}

impl Meta {
    /// A new conversation's, titled by its first question.
    pub fn new(record: i64, resumable: Resumable, question: &str, now_ms: i64) -> Self {
        Self {
            record,
            resumable,
            title: transcript::title(question),
            created_ms: now_ms,
        }
    }
}

struct Recording {
    /// `None` while its events came in before its question did.
    meta: Option<Meta>,
    transcript: Transcript,
    /// Changed since it was last saved.
    dirty: bool,
}

impl Recording {
    fn pending() -> Self {
        Self {
            meta: None,
            transcript: Transcript::default(),
            dirty: false,
        }
    }

    /// What to save now, if it's a conversation being recorded.
    fn snapshot(&mut self, now_ms: i64) -> Option<Record> {
        let meta = self.meta.as_ref()?;
        self.dirty = false;
        Some(Record {
            id: meta.record,
            provider: meta.resumable.provider,
            model: self.transcript.model.clone(),
            session: meta.resumable.session(),
            cwd: meta.resumable.cwd.display().to_string(),
            title: meta.title.clone(),
            created_ms: meta.created_ms,
            updated_ms: now_ms,
            pinned: false,
            entries: self.transcript.entries.clone(),
        })
    }
}

/// Live conversations by the chat's conversation id.
#[derive(Default)]
pub struct Recorder {
    recordings: HashMap<u32, Recording>,
    /// The newest conversation recorded. Events for an older one that isn't
    /// recorded are leftovers from a closed chat.
    newest: u32,
}

impl Recorder {
    /// A new conversation's first question. Other conversations are over
    /// (the chat runs one at a time): what they had unsaved is returned
    /// too.
    pub fn begin(
        &mut self,
        conversation: u32,
        meta: Meta,
        question: &str,
        context: Option<&str>,
        now_ms: i64,
    ) -> Vec<Record> {
        let mut saves = self.finish_others(conversation, now_ms);
        let recording = self
            .recordings
            .entry(conversation)
            .or_insert_with(Recording::pending);
        recording.meta = Some(meta);
        recording.transcript.user_message(question, context);
        saves.extend(recording.snapshot(now_ms));
        self.newest = self.newest.max(conversation);
        saves
    }

    /// A saved conversation reopened as `conversation`: recording carries
    /// on into the same record.
    pub fn adopt(
        &mut self,
        conversation: u32,
        meta: Meta,
        transcript: Transcript,
        now_ms: i64,
    ) -> Vec<Record> {
        let saves = self.finish_others(conversation, now_ms);
        self.recordings.insert(
            conversation,
            Recording {
                meta: Some(meta),
                transcript,
                dirty: false,
            },
        );
        self.newest = self.newest.max(conversation);
        saves
    }

    /// A follow-up. A conversation that started while history was off
    /// isn't recorded.
    pub fn question(
        &mut self,
        conversation: u32,
        text: &str,
        context: Option<&str>,
        now_ms: i64,
    ) -> Option<Record> {
        let recording = self.recordings.get_mut(&conversation)?;
        recording.meta.as_ref()?;
        recording.transcript.user_message(text, context);
        recording.snapshot(now_ms)
    }

    /// One streamed event; returns what to save: the conversation when the
    /// event ends a turn, and older ones a newer conversation has ended.
    pub fn event(&mut self, conversation: u32, event: &AiEvent, now_ms: i64) -> Vec<Record> {
        let mut saves = Vec::new();
        if !self.recordings.contains_key(&conversation) {
            if conversation <= self.newest {
                return saves;
            }
            // A new conversation whose question isn't recorded yet (the ask
            // is returning). The chat runs one at a time, so the others are
            // over; that also keeps unrecorded ones (history off) from
            // piling up.
            saves = self.finish_others(conversation, now_ms);
            self.newest = conversation;
            self.recordings.insert(conversation, Recording::pending());
        }
        let Some(recording) = self.recordings.get_mut(&conversation) else {
            return saves;
        };
        recording.dirty = true;
        if recording.transcript.apply(event) {
            saves.extend(recording.snapshot(now_ms));
        }
        saves
    }

    /// The chat was closed: every conversation is over. Returns what they
    /// had unsaved (a turn cut short).
    pub fn finish(&mut self, now_ms: i64) -> Vec<Record> {
        self.finish_others(0, now_ms)
    }

    /// Stops recording the conversations whose records `deleted` picks, so
    /// the next save doesn't bring them back.
    pub fn forget(&mut self, deleted: impl Fn(i64) -> bool) {
        self.recordings
            .retain(|_, r| r.meta.as_ref().is_none_or(|meta| !deleted(meta.record)));
    }

    /// The record conversation `conversation` is saved in, if it's recorded.
    #[cfg(test)]
    pub fn record_of(&self, conversation: u32) -> Option<i64> {
        self.recordings
            .get(&conversation)?
            .meta
            .as_ref()
            .map(|m| m.record)
    }

    fn finish_others(&mut self, keep: u32, now_ms: i64) -> Vec<Record> {
        let mut saves = Vec::new();
        self.recordings.retain(|&id, recording| {
            if id == keep {
                return true;
            }
            if recording.dirty {
                saves.extend(recording.snapshot(now_ms));
            }
            false
        });
        saves
    }
}

/// What reopening found on disk.
#[derive(Debug, Clone, Copy)]
pub struct Found {
    /// The conversation's folder still exists.
    pub folder: bool,
    /// Claude Code still has the session (see `claude_sessions`).
    pub session: bool,
}

/// Where a reopened conversation continues, and a note when it can't pick
/// up where it left off. Claude Code keeps sessions per folder and deletes
/// old ones, so without either it starts over (in your home folder if the
/// folder is gone). A Codex thread resumes anywhere, and one that's gone
/// is replaced when it resumes.
pub fn resume_plan(saved: Resumable, found: Found, home: PathBuf) -> (Resumable, Option<String>) {
    let note = match saved.provider {
        ProviderId::Chatgpt => None,
        ProviderId::Claude if !found.folder => Some(format!(
            "Its folder ({}) is gone, so Claude starts this conversation over.",
            saved.cwd.display()
        )),
        ProviderId::Claude if !found.session => Some(
            "Claude Code no longer has this conversation (it deletes old ones), \
             so your next message starts it over."
                .to_owned(),
        ),
        ProviderId::Claude => None,
    };
    if found.folder && note.is_none() {
        return (saved, None);
    }
    let cwd = if found.folder {
        saved.cwd.clone()
    } else {
        home
    };
    let session = if note.is_some() {
        None
    } else {
        saved.session()
    };
    (Resumable::new(saved.provider, cwd, session), note)
}

#[cfg(test)]
mod tests;
