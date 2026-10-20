//! Local database (SQLite) at `%LOCALAPPDATA%\desktop-assistant\data.db`.
//! For now it holds frecency: how often and how recently each result was opened.
//!
//! Frecency is kept in memory for ranking on every keystroke and written
//! through to SQLite on each open.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};

/// After two weeks unused, an item's boost halves.
const HALF_LIFE_SECS: f64 = 14.0 * 24.0 * 3600.0;

/// Schema migrations, applied in order. `PRAGMA user_version` records how
/// many have run. Append new steps; never edit old ones.
const MIGRATIONS: &[&str] = &["CREATE TABLE launches (
        id         TEXT PRIMARY KEY,
        score      REAL    NOT NULL,
        updated_at INTEGER NOT NULL,
        count      INTEGER NOT NULL DEFAULT 0
    );"];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Copy)]
struct Frecency {
    score: f64,
    updated_at: i64,
}

impl Frecency {
    fn decayed(self, now: i64) -> f64 {
        let age = (now - self.updated_at).max(0) as f64;
        self.score * 0.5_f64.powf(age / HALF_LIFE_SECS)
    }
}

pub struct Store {
    conn: Mutex<Connection>,
    frecency: RwLock<HashMap<String, Frecency>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::from_connection(Connection::open(path)?)
    }

    /// A throwaway store, used if the file can't be opened (and in tests).
    pub fn in_memory() -> Self {
        Connection::open_in_memory()
            .map_err(StoreError::from)
            .and_then(Self::from_connection)
            .expect("in-memory SQLite always opens")
    }

    fn from_connection(mut conn: Connection) -> Result<Self, StoreError> {
        migrate(&mut conn)?;
        let frecency = load_frecency(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            frecency: RwLock::new(frecency),
        })
    }

    /// Decayed open count for a result id; 0 if never opened.
    pub fn frecency(&self, id: &str) -> f64 {
        self.frecency_at(id, now())
    }

    fn frecency_at(&self, id: &str, now: i64) -> f64 {
        self.frecency
            .read()
            .ok()
            .and_then(|m| m.get(id).map(|f| f.decayed(now)))
            .unwrap_or(0.0)
    }

    /// Call when the user opens a result.
    pub fn record_open(&self, id: &str) -> Result<(), StoreError> {
        self.record_open_at(id, now())
    }

    fn record_open_at(&self, id: &str, now: i64) -> Result<(), StoreError> {
        let score = self.frecency_at(id, now) + 1.0;
        if let Ok(mut map) = self.frecency.write() {
            map.insert(
                id.to_owned(),
                Frecency {
                    score,
                    updated_at: now,
                },
            );
        }
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(poisoned) => poisoned.into_inner(),
        };
        conn.execute(
            "INSERT INTO launches (id, score, updated_at, count) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(id) DO UPDATE SET score = ?2, updated_at = ?3, count = count + 1",
            params![id, score, now],
        )?;
        Ok(())
    }
}

fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let applied: usize =
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    if applied >= MIGRATIONS.len() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    for sql in &MIGRATIONS[applied..] {
        tx.execute_batch(sql)?;
    }
    tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
    tx.commit()?;
    Ok(())
}

fn load_frecency(conn: &Connection) -> Result<HashMap<String, Frecency>, StoreError> {
    let mut stmt = conn.prepare("SELECT id, score, updated_at FROM launches")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Frecency {
                score: r.get(1)?,
                updated_at: r.get(2)?,
            },
        ))
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 3600;

    #[test]
    fn unknown_ids_score_zero() {
        assert_eq!(Store::in_memory().frecency("app:x"), 0.0);
    }

    #[test]
    fn opens_accumulate() {
        let s = Store::in_memory();
        s.record_open_at("app:x", 1000).unwrap();
        s.record_open_at("app:x", 1000).unwrap();
        assert!((s.frecency_at("app:x", 1000) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn decays_by_half_every_two_weeks() {
        let s = Store::in_memory();
        s.record_open_at("app:x", 0).unwrap();
        let later = s.frecency_at("app:x", 14 * DAY);
        assert!((later - 0.5).abs() < 1e-9, "{later}");
    }

    #[test]
    fn survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.db");
        Store::open(&path).unwrap().record_open("app:x").unwrap();
        let reopened = Store::open(&path).unwrap();
        assert!(reopened.frecency("app:x") > 0.9);
    }

    #[test]
    fn migrations_are_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
    }
}
