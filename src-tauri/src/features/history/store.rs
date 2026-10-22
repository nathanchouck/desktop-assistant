//! Saved conversations: their own SQLite file (`history.db` in the data
//! folder), with a summary of each kept in memory so search never reads
//! the disk.
//!
//! The list is pinned first, then most recently used. Only unpinned
//! conversations count towards the cap or are forgotten with age.

use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock};

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::transcript::{self, ChatEntry};
use crate::ai::ProviderId;

/// Schema migrations, applied in order (`PRAGMA user_version` counts them).
/// Append new steps; never edit old ones.
const MIGRATIONS: &[&str] = &["CREATE TABLE conversations (
        id         INTEGER PRIMARY KEY,
        provider   TEXT    NOT NULL,
        model      TEXT    NOT NULL,
        session    TEXT,
        cwd        TEXT    NOT NULL,
        title      TEXT    NOT NULL,
        created    INTEGER NOT NULL,
        updated    INTEGER NOT NULL,
        messages   INTEGER NOT NULL,
        pinned     INTEGER NOT NULL DEFAULT 0,
        search     TEXT    NOT NULL,
        transcript TEXT    NOT NULL
    );"];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("chat history database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("could not make the chat history folder: {0}")]
    Io(#[from] std::io::Error),
}

/// A whole saved conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub id: i64,
    pub provider: ProviderId,
    /// The model that answered ("claude-sonnet-5-5"); may be empty.
    pub model: String,
    /// The Claude Code session or Codex thread, to resume it.
    pub session: Option<String>,
    /// The folder it ran in (Claude Code keeps sessions per folder).
    pub cwd: String,
    pub title: String,
    pub created_ms: i64,
    pub updated_ms: i64,
    /// Saving never changes this; see `ConversationStore::toggle_pin`.
    pub pinned: bool,
    pub entries: Vec<ChatEntry>,
}

/// What search needs about a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub id: i64,
    pub provider: ProviderId,
    pub title: String,
    pub updated_ms: i64,
    pub messages: u32,
    pub pinned: bool,
    /// Lowercase questions and answers (cut short), for the filter.
    pub search: String,
}

pub struct ConversationStore {
    conn: Mutex<Connection>,
    summaries: RwLock<Vec<Summary>>,
    /// The id the next new conversation gets.
    next_id: AtomicI64,
}

impl ConversationStore {
    /// Opens (or creates) the database. The summaries are read separately
    /// (`load_summaries`), off the startup path.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    /// A store that isn't saved (if the file can't be opened, and in tests).
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(mut conn: Connection) -> Result<Self, StoreError> {
        migrate(&mut conn)?;
        let last: i64 =
            conn.query_row("SELECT COALESCE(MAX(id), 0) FROM conversations", [], |r| {
                r.get(0)
            })?;
        Ok(Self {
            conn: Mutex::new(conn),
            summaries: RwLock::new(Vec::new()),
            next_id: AtomicI64::new(last + 1),
        })
    }

    /// An id for a new conversation. Handed out before its first save, so
    /// recording never waits for the database.
    pub fn new_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Reads every conversation's summary into memory.
    pub fn load_summaries(&self) -> Result<(), StoreError> {
        let loaded = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "SELECT id, provider, title, updated, messages, pinned, search
                 FROM conversations",
            )?;
            let rows = stmt.query_map([], summary_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut summaries = self.summaries_mut();
        *summaries = loaded;
        sort(&mut summaries);
        Ok(())
    }

    /// Runs `f` over the list (pinned first, then most recently used).
    pub fn with_summaries<R>(&self, f: impl FnOnce(&[Summary]) -> R) -> R {
        match self.summaries.read() {
            Ok(list) => f(&list),
            Err(poisoned) => f(&poisoned.into_inner()),
        }
    }

    /// The conversation used last, pinned or not.
    pub fn latest(&self) -> Option<i64> {
        self.with_summaries(|list| {
            list.iter()
                .max_by_key(|s| (s.updated_ms, s.id))
                .map(|s| s.id)
        })
    }

    /// (conversations, of which pinned).
    pub fn counts(&self) -> (usize, usize) {
        self.with_summaries(|list| (list.len(), list.iter().filter(|s| s.pinned).count()))
    }

    /// Saves a conversation, adding it or replacing its content. Its pin
    /// and creation time stay as they are.
    pub fn save(&self, record: &Record) -> Result<(), StoreError> {
        let messages = transcript::message_count(&record.entries);
        let search = transcript::search_text(&record.entries);
        let pinned = {
            let conn = self.conn();
            conn.execute(
                "INSERT INTO conversations
                   (id, provider, model, session, cwd, title, created, updated,
                    messages, search, transcript)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                   model = excluded.model, session = excluded.session,
                   updated = excluded.updated, messages = excluded.messages,
                   search = excluded.search, transcript = excluded.transcript",
                params![
                    record.id,
                    provider_key(record.provider),
                    record.model,
                    record.session,
                    record.cwd,
                    record.title,
                    record.created_ms,
                    record.updated_ms,
                    messages,
                    search,
                    transcript::to_json(&record.entries),
                ],
            )?;
            conn.query_row(
                "SELECT pinned FROM conversations WHERE id = ?1",
                [record.id],
                |r| r.get::<_, bool>(0),
            )?
        };
        let mut list = self.summaries_mut();
        list.retain(|s| s.id != record.id);
        list.push(Summary {
            id: record.id,
            provider: record.provider,
            title: record.title.clone(),
            updated_ms: record.updated_ms,
            messages,
            pinned,
            search,
        });
        sort(&mut list);
        Ok(())
    }

    pub fn load(&self, id: i64) -> Result<Option<Record>, StoreError> {
        let record = self
            .conn()
            .query_row(
                "SELECT id, provider, model, session, cwd, title, created, updated,
                        pinned, transcript
                 FROM conversations WHERE id = ?1",
                [id],
                |r| {
                    Ok(Record {
                        id: r.get(0)?,
                        provider: provider_from_key(&r.get::<_, String>(1)?),
                        model: r.get(2)?,
                        session: r.get(3)?,
                        cwd: r.get(4)?,
                        title: r.get(5)?,
                        created_ms: r.get(6)?,
                        updated_ms: r.get(7)?,
                        pinned: r.get(8)?,
                        entries: transcript::from_json(&r.get::<_, String>(9)?),
                    })
                },
            )
            .optional()?;
        Ok(record)
    }

    /// Removes one conversation; false if it wasn't there.
    pub fn delete(&self, id: i64) -> Result<bool, StoreError> {
        let removed = self
            .conn()
            .execute("DELETE FROM conversations WHERE id = ?1", [id])?;
        self.summaries_mut().retain(|s| s.id != id);
        Ok(removed > 0)
    }

    /// Removes every unpinned conversation; returns their ids.
    pub fn clear(&self) -> Result<Vec<i64>, StoreError> {
        let ids = self.ids_where("pinned = 0", [])?;
        self.remove(&ids)?;
        Ok(ids)
    }

    /// Pins or unpins; returns the new state, or `None` if it's gone.
    pub fn toggle_pin(&self, id: i64) -> Result<Option<bool>, StoreError> {
        let pinned = {
            let conn = self.conn();
            conn.execute(
                "UPDATE conversations SET pinned = 1 - pinned WHERE id = ?1",
                [id],
            )?;
            conn.query_row(
                "SELECT pinned FROM conversations WHERE id = ?1",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
        };
        if let Some(pinned) = pinned {
            let mut list = self.summaries_mut();
            if let Some(s) = list.iter_mut().find(|s| s.id == id) {
                s.pinned = pinned;
            }
            sort(&mut list);
        }
        Ok(pinned)
    }

    /// Drops unpinned conversations last used before `older_than_ms` (if
    /// set), then the oldest unpinned ones past `max`. Returns their ids.
    pub fn prune(&self, max: u32, older_than_ms: Option<i64>) -> Result<Vec<i64>, StoreError> {
        let mut doomed = match older_than_ms {
            Some(cutoff) => self.ids_where("pinned = 0 AND updated < ?1", [cutoff])?,
            None => Vec::new(),
        };
        doomed.extend(self.ids_where(
            "pinned = 0 AND id IN (SELECT id FROM conversations WHERE pinned = 0
             ORDER BY updated DESC, id DESC LIMIT -1 OFFSET ?1)",
            [i64::from(max)],
        )?);
        doomed.sort_unstable();
        doomed.dedup();
        self.remove(&doomed)?;
        Ok(doomed)
    }

    fn ids_where<P: rusqlite::Params>(
        &self,
        filter: &str,
        params: P,
    ) -> Result<Vec<i64>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT id FROM conversations WHERE {filter}"))?;
        let rows = stmt.query_map(params, |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<i64>, _>>()?)
    }

    fn remove(&self, ids: &[i64]) -> Result<(), StoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            for id in ids {
                tx.execute("DELETE FROM conversations WHERE id = ?1", [id])?;
            }
            tx.commit()?;
        }
        self.summaries_mut().retain(|s| !ids.contains(&s.id));
        Ok(())
    }

    /// The connection; a panic while it was held doesn't stop history.
    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn summaries_mut(&self) -> std::sync::RwLockWriteGuard<'_, Vec<Summary>> {
        self.summaries.write().unwrap_or_else(|p| p.into_inner())
    }
}

fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let done: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, step) in (0_i64..)
        .zip(MIGRATIONS)
        .skip(usize::try_from(done).unwrap_or(0))
    {
        let tx = conn.transaction()?;
        tx.execute_batch(step)?;
        tx.pragma_update(None, "user_version", i + 1)?;
        tx.commit()?;
    }
    Ok(())
}

fn summary_row(r: &Row<'_>) -> rusqlite::Result<Summary> {
    Ok(Summary {
        id: r.get(0)?,
        provider: provider_from_key(&r.get::<_, String>(1)?),
        title: r.get(2)?,
        updated_ms: r.get(3)?,
        messages: r.get(4)?,
        pinned: r.get(5)?,
        search: r.get(6)?,
    })
}

fn sort(list: &mut [Summary]) {
    list.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.updated_ms.cmp(&a.updated_ms))
            .then(b.id.cmp(&a.id))
    });
}

fn provider_key(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "claude",
        ProviderId::Chatgpt => "chatgpt",
    }
}

fn provider_from_key(key: &str) -> ProviderId {
    match key {
        "chatgpt" => ProviderId::Chatgpt,
        _ => ProviderId::Claude,
    }
}

#[cfg(test)]
mod tests;
