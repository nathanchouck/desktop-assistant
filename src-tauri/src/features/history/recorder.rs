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
use crate::ai::AiEvent;

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

/// Where a reopened conversation continues, and a note when it can't pick
/// up where it left off. Claude Code keeps sessions per folder, so with its
/// folder gone it starts over (in your home folder); a Codex thread
/// resumes anywhere.
pub fn resume_plan(
    saved: Resumable,
    folder_exists: bool,
    home: PathBuf,
) -> (Resumable, Option<String>) {
    if folder_exists {
        return (saved, None);
    }
    let gone = saved.cwd.display().to_string();
    match saved.provider {
        crate::ai::ProviderId::Claude => (
            Resumable::new(saved.provider, home, None),
            Some(format!(
                "Its folder ({gone}) is gone, so Claude starts this conversation over."
            )),
        ),
        crate::ai::ProviderId::Chatgpt => {
            let session = saved.session();
            (Resumable::new(saved.provider, home, session), None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ProviderId;
    use crate::features::history::transcript::ChatEntry;

    fn meta(record: i64) -> Meta {
        let resumable = Resumable::new(ProviderId::Claude, PathBuf::from("/work"), None);
        Meta::new(record, resumable, "  sort my\n downloads ", 100)
    }

    fn delta(text: &str) -> AiEvent {
        AiEvent::TextDelta { text: text.into() }
    }

    fn user(text: &str) -> ChatEntry {
        ChatEntry::User {
            text: text.into(),
            context: None,
        }
    }

    fn assistant(text: &str) -> ChatEntry {
        ChatEntry::Assistant { text: text.into() }
    }

    #[test]
    fn saves_on_the_question_and_at_each_turn_end() {
        let mut r = Recorder::default();
        let saved = r.begin(1, meta(7), "sort my downloads", Some("Downloads"), 200);
        assert_eq!(saved.len(), 1, "the question is saved at once");
        assert_eq!(saved[0].id, 7);
        assert_eq!(saved[0].title, "sort my downloads");
        assert_eq!(saved[0].created_ms, 100);
        assert_eq!(saved[0].cwd, "/work");

        assert!(r.event(1, &delta("Moving"), 300).is_empty(), "not mid-turn");
        let saved = r.event(1, &AiEvent::Done, 400).pop().unwrap();
        assert_eq!(saved.updated_ms, 400);
        assert_eq!(
            saved.entries,
            [
                ChatEntry::User {
                    text: "sort my downloads".into(),
                    context: Some("Downloads".into()),
                },
                assistant("Moving"),
            ]
        );

        let saved = r.question(1, "thanks", None, 500).unwrap();
        assert_eq!(saved.entries.last(), Some(&user("thanks")));
        assert_eq!(r.record_of(1), Some(7));
    }

    #[test]
    fn events_before_the_question_are_kept() {
        let mut r = Recorder::default();
        r.event(
            1,
            &AiEvent::Started {
                model: "claude-haiku-5-5".into(),
            },
            0,
        );
        r.event(1, &delta("Hi"), 0);
        let saved = r.begin(1, meta(1), "hello", None, 10);
        assert_eq!(saved[0].model, "claude-haiku-5-5");
        assert_eq!(saved[0].entries, [user("hello"), assistant("Hi")]);
    }

    #[test]
    fn a_new_conversation_finishes_the_last_one() {
        let mut r = Recorder::default();
        r.begin(1, meta(1), "first", None, 0);
        r.event(1, &delta("half an ans"), 0);
        // A new question while the old answer was still coming.
        let saved = r.begin(2, meta(2), "second", None, 10);
        let ids: Vec<i64> = saved.iter().map(|s| s.id).collect();
        assert_eq!(ids, [1, 2]);
        assert_eq!(saved[0].entries.last(), Some(&assistant("half an ans")));
        // Late events of the old one are dropped.
        assert!(r.event(1, &AiEvent::Done, 20).is_empty());
        assert_eq!(r.record_of(1), None);
        // An old conversation that was saved and is clean isn't saved again.
        r.event(2, &AiEvent::Done, 30);
        assert!(r.finish(40).is_empty());
        assert_eq!(r.record_of(2), None);
    }

    #[test]
    fn a_newer_conversation_ends_the_older_ones() {
        let mut r = Recorder::default();
        r.begin(1, meta(1), "first", None, 0);
        r.event(1, &delta("half an answer"), 0);
        // Conversation 2 isn't recorded (history was off when it was
        // asked): its first event ends 1, saved with what came.
        let saved = r.event(2, &delta("not kept"), 10);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].entries.last(), Some(&assistant("half an answer")));
        assert_eq!(r.record_of(1), None);
        // 3 ends 2 without saving anything: only one is held at a time.
        assert!(r.event(3, &delta("Hi"), 20).is_empty());
        assert_eq!(r.recordings.len(), 1);
    }

    #[test]
    fn closing_mid_turn_saves_what_came() {
        let mut r = Recorder::default();
        r.begin(3, meta(3), "long story", None, 0);
        r.event(3, &delta("Once upon"), 0);
        let saved = r.finish(50);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].entries.last(), Some(&assistant("Once upon")));
    }

    #[test]
    fn unrecorded_conversations_stay_unrecorded() {
        let mut r = Recorder::default();
        // History was off when this one started: its follow-ups aren't kept.
        assert!(r.question(4, "more", None, 0).is_none());
        // Events for a conversation never asked about are held, not saved.
        assert!(r.event(5, &AiEvent::Done, 0).is_empty());
        assert!(r.question(5, "more", None, 0).is_none());
        assert!(r.finish(0).is_empty());
    }

    #[test]
    fn a_reopened_conversation_carries_on_in_its_record() {
        let mut r = Recorder::default();
        let saved = Transcript::from_saved(vec![user("hi"), assistant("Hello")], "gpt-6".into());
        assert!(r.adopt(9, meta(42), saved, 0).is_empty());
        r.question(9, "and then?", None, 10);
        let record = r.event(9, &AiEvent::Done, 20).pop().unwrap();
        assert_eq!(record.id, 42);
        assert_eq!(record.model, "gpt-6");
        assert_eq!(record.entries.len(), 3);
    }

    #[test]
    fn forgotten_records_are_not_saved_again() {
        let mut r = Recorder::default();
        r.begin(1, meta(1), "secret", None, 0);
        r.forget(|record| record == 1);
        assert!(r.event(1, &AiEvent::Done, 10).is_empty());
        assert!(r.question(1, "more", None, 20).is_none());
        assert!(r.finish(30).is_empty());
    }

    #[test]
    fn a_lost_folder_means_claude_starts_over() {
        let home = PathBuf::from("/home/me");
        let claude = Resumable::new(ProviderId::Claude, "/gone".into(), Some("s".into()));
        let (plan, note) = resume_plan(claude.clone(), true, home.clone());
        assert_eq!(
            (plan.session(), plan.cwd, note),
            (Some("s".into()), "/gone".into(), None)
        );

        let (plan, note) = resume_plan(claude, false, home.clone());
        assert_eq!((plan.session(), plan.cwd), (None, home.clone()));
        assert!(note.unwrap().contains("/gone"));

        let codex = Resumable::new(ProviderId::Chatgpt, "/gone".into(), Some("t".into()));
        let (plan, note) = resume_plan(codex, false, home.clone());
        assert_eq!(
            (plan.session(), plan.cwd, note),
            (Some("t".into()), home, None)
        );
    }
}
