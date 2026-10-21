//! The clipboard history: its own SQLite file (`clipboard.db` in the data
//! folder), images as PNG files beside it, and an in-memory copy of the
//! list so searching never touches the disk.
//!
//! The list is newest first with pinned entries on top. Copying something
//! that's already in the list moves it up instead of adding it again.
//! Only unpinned entries count towards the cap.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use rusqlite::{params, Connection, OptionalExtension};

use super::content::{self, ClipKind};

/// Schema migrations, applied in order (`PRAGMA user_version` counts them).
/// Append new steps; never edit old ones.
const MIGRATIONS: &[&str] = &["CREATE TABLE clips (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        kind    TEXT    NOT NULL,
        content TEXT    NOT NULL,
        hash    TEXT    NOT NULL UNIQUE,
        width   INTEGER NOT NULL DEFAULT 0,
        height  INTEGER NOT NULL DEFAULT 0,
        source  TEXT    NOT NULL,
        created INTEGER NOT NULL,
        used    INTEGER NOT NULL,
        pinned  INTEGER NOT NULL DEFAULT 0
    );"];

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("clipboard database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("could not save a clipboard image: {0}")]
    Io(#[from] std::io::Error),
}

/// One row of the list, as search needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub kind: ClipKind,
    pub title: String,
    /// What the search filter matches: the start of the text, the paths.
    pub preview: String,
    pub hash: String,
    /// The program it was copied in ("chrome"); may be empty.
    pub source: String,
    /// Last copied or pasted (Unix ms); the list's order.
    pub used_ms: i64,
    pub pinned: bool,
}

/// Something to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewClip {
    pub kind: ClipKind,
    /// The text; for files, one path per line; empty for images.
    pub content: String,
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub source: String,
}

/// An entry's full content, for pasting it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    Text(String),
    Files(Vec<String>),
    /// The PNG file holding the image.
    Image(PathBuf),
}

pub struct History {
    conn: Mutex<Connection>,
    entries: RwLock<Vec<Entry>>,
    images_dir: PathBuf,
    thumbs_dir: PathBuf,
}

impl History {
    /// Opens (or creates) the history. Images go in `images_dir`, their
    /// thumbnails in `thumbs_dir` (served to rows as `thumb/clip-<hash>`).
    pub fn open(db: &Path, images_dir: PathBuf, thumbs_dir: PathBuf) -> Result<Self, HistoryError> {
        if let Some(dir) = db.parent() {
            fs::create_dir_all(dir)?;
        }
        Self::from_connection(Connection::open(db)?, images_dir, thumbs_dir)
    }

    /// A history that isn't saved (if the file can't be opened, and in tests).
    pub fn in_memory(images_dir: PathBuf, thumbs_dir: PathBuf) -> Result<Self, HistoryError> {
        Self::from_connection(Connection::open_in_memory()?, images_dir, thumbs_dir)
    }

    fn from_connection(
        mut conn: Connection,
        images_dir: PathBuf,
        thumbs_dir: PathBuf,
    ) -> Result<Self, HistoryError> {
        migrate(&mut conn)?;
        let entries = load(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            entries: RwLock::new(entries),
            images_dir,
            thumbs_dir,
        })
    }

    /// Runs `f` over the list (pinned first, then newest first).
    pub fn with_entries<R>(&self, f: impl FnOnce(&[Entry]) -> R) -> R {
        match self.entries.read() {
            Ok(entries) => f(&entries),
            Err(poisoned) => f(&poisoned.into_inner()),
        }
    }

    pub fn len(&self) -> usize {
        self.with_entries(<[Entry]>::len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Where an image entry's PNG lives.
    pub fn image_path(&self, hash: &str) -> PathBuf {
        self.images_dir.join(format!("{hash}.png"))
    }

    /// Where an image entry's thumbnail lives.
    pub fn thumb_path(&self, hash: &str) -> PathBuf {
        self.thumbs_dir.join(format!("{}.png", thumb_name(hash)))
    }

    /// If an entry with this content exists, moves it to the top and
    /// returns true.
    pub fn bump_hash(&self, hash: &str, now_ms: i64) -> Result<bool, HistoryError> {
        let changed = self.conn()?.execute(
            "UPDATE clips SET used = ?1 WHERE hash = ?2",
            params![now_ms, hash],
        )?;
        if changed > 0 {
            self.reload()?;
        }
        Ok(changed > 0)
    }

    /// Moves an entry to the top (it was just pasted).
    pub fn bump(&self, id: i64, now_ms: i64) -> Result<(), HistoryError> {
        self.conn()?.execute(
            "UPDATE clips SET used = ?1 WHERE id = ?2",
            params![now_ms, id],
        )?;
        self.reload()
    }

    /// Adds a copy at the top, or moves the same content up if it's already
    /// listed. Then drops the oldest unpinned entries past `max_items`.
    pub fn add(&self, clip: &NewClip, now_ms: i64, max_items: u32) -> Result<(), HistoryError> {
        self.conn()?.execute(
            "INSERT INTO clips (kind, content, hash, width, height, source, created, used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(hash) DO UPDATE SET used = excluded.used",
            params![
                clip.kind.as_str(),
                clip.content,
                clip.hash,
                clip.width,
                clip.height,
                clip.source,
                now_ms
            ],
        )?;
        self.drop_oldest(max_items)?;
        self.reload()
    }

    /// Drops the oldest unpinned entries past `max_items` (and their images).
    pub fn trim(&self, max_items: u32) -> Result<(), HistoryError> {
        self.drop_oldest(max_items)?;
        self.reload()
    }

    fn drop_oldest(&self, max_items: u32) -> Result<(), HistoryError> {
        let doomed = {
            let conn = self.conn()?;
            let mut stmt = conn.prepare(
                "SELECT id, kind, hash FROM clips WHERE pinned = 0
                 ORDER BY used DESC, id DESC LIMIT -1 OFFSET ?1",
            )?;
            let rows = stmt.query_map([max_items], doomed_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        self.remove(&doomed)
    }

    /// The full content of an entry.
    pub fn stored(&self, id: i64) -> Result<Option<Stored>, HistoryError> {
        let row = self
            .conn()?
            .query_row(
                "SELECT kind, content, hash FROM clips WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(
            row.and_then(|(kind, content, hash)| match ClipKind::parse(&kind)? {
                ClipKind::Text => Some(Stored::Text(content)),
                ClipKind::Files => {
                    Some(Stored::Files(content.lines().map(str::to_owned).collect()))
                }
                ClipKind::Image => Some(Stored::Image(self.image_path(&hash))),
            }),
        )
    }

    /// Pins or unpins an entry; returns whether it's pinned now.
    pub fn toggle_pin(&self, id: i64) -> Result<bool, HistoryError> {
        let pinned = self.conn()?.query_row(
            "UPDATE clips SET pinned = 1 - pinned WHERE id = ?1 RETURNING pinned",
            [id],
            |r| r.get::<_, bool>(0),
        )?;
        self.reload()?;
        Ok(pinned)
    }

    pub fn delete(&self, id: i64) -> Result<(), HistoryError> {
        let doomed = self
            .conn()?
            .query_row(
                "SELECT id, kind, hash FROM clips WHERE id = ?1",
                [id],
                doomed_row,
            )
            .optional()?;
        self.remove(&doomed.into_iter().collect::<Vec<_>>())?;
        self.reload()
    }

    /// Removes every unpinned entry; returns how many went.
    pub fn clear(&self) -> Result<usize, HistoryError> {
        let doomed = {
            let conn = self.conn()?;
            let mut stmt = conn.prepare("SELECT id, kind, hash FROM clips WHERE pinned = 0")?;
            let rows = stmt.query_map([], doomed_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        self.remove(&doomed)?;
        self.reload()?;
        Ok(doomed.len())
    }

    /// Deletes image files no entry refers to (left behind if the app
    /// stopped between saving a picture and listing it). Call before
    /// recording starts. Returns how many went.
    pub fn sweep(&self) -> usize {
        let known: HashSet<String> = self.with_entries(|entries| {
            entries
                .iter()
                .filter(|e| e.kind == ClipKind::Image)
                .map(|e| e.hash.clone())
                .collect()
        });
        // The thumbnails folder is shared: only `clip-` files are ours.
        let images = png_stems(&self.images_dir);
        let thumbs = png_stems(&self.thumbs_dir)
            .into_iter()
            .filter_map(|stem| Some(stem.strip_prefix("clip-")?.to_owned()));
        let mut removed = 0;
        for hash in images.into_iter().filter(|h| !known.contains(h)) {
            removed += usize::from(fs::remove_file(self.image_path(&hash)).is_ok());
        }
        for hash in thumbs.filter(|h| !known.contains(h)) {
            removed += usize::from(fs::remove_file(self.thumb_path(&hash)).is_ok());
        }
        removed
    }

    /// Deletes rows, then their image files. The caller reloads the list.
    fn remove(&self, doomed: &[(i64, ClipKind, String)]) -> Result<(), HistoryError> {
        if doomed.is_empty() {
            return Ok(());
        }
        {
            let mut conn = self.conn()?;
            let tx = conn.transaction()?;
            for (id, _, _) in doomed {
                tx.execute("DELETE FROM clips WHERE id = ?1", [id])?;
            }
            tx.commit()?;
        }
        for (_, kind, hash) in doomed {
            if *kind == ClipKind::Image {
                // Already gone is fine.
                let _ = fs::remove_file(self.image_path(hash));
                let _ = fs::remove_file(self.thumb_path(hash));
            }
        }
        Ok(())
    }

    /// Refreshes the in-memory list from the database.
    fn reload(&self) -> Result<(), HistoryError> {
        let fresh = load(&*self.conn()?)?;
        match self.entries.write() {
            Ok(mut entries) => *entries = fresh,
            Err(poisoned) => *poisoned.into_inner() = fresh,
        }
        Ok(())
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, HistoryError> {
        // A panic while holding the lock leaves the connection usable.
        Ok(self.conn.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

/// Names (without `.png`) of the PNG files in `dir`.
fn png_stems(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "png" {
                return None;
            }
            Some(path.file_stem()?.to_str()?.to_owned())
        })
        .collect()
}

/// The thumbnail's name for `thumb_icon`: `clip-<hash>`.
pub fn thumb_name(hash: &str) -> String {
    format!("clip-{hash}")
}

fn doomed_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, ClipKind, String)> {
    let kind: String = r.get(1)?;
    Ok((
        r.get(0)?,
        ClipKind::parse(&kind).unwrap_or(ClipKind::Text),
        r.get(2)?,
    ))
}

fn load(conn: &Connection) -> Result<Vec<Entry>, HistoryError> {
    // Only the start of long texts: enough for the title and the filter.
    let mut stmt = conn.prepare(
        "SELECT id, kind, substr(content, 1, 1000), hash, width, height, source, used, pinned
         FROM clips ORDER BY pinned DESC, used DESC, id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        let kind: String = r.get(1)?;
        let Some(kind) = ClipKind::parse(&kind) else {
            return Ok(None);
        };
        let text: String = r.get(2)?;
        let (title, preview) = match kind {
            ClipKind::Text => (content::text_title(&text), content::preview(&text)),
            ClipKind::Image => (content::image_title(r.get(4)?, r.get(5)?), String::new()),
            ClipKind::Files => {
                let paths: Vec<String> = text.lines().map(str::to_owned).collect();
                (content::files_title(&paths), paths.join(" "))
            }
        };
        Ok(Some(Entry {
            id: r.get(0)?,
            kind,
            title,
            preview,
            hash: r.get(3)?,
            source: r.get(6)?,
            used_ms: r.get(7)?,
            pinned: r.get(8)?,
        }))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        entries.extend(row?);
    }
    Ok(entries)
}

fn migrate(conn: &mut Connection) -> Result<(), HistoryError> {
    let applied = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> (tempfile::TempDir, History) {
        let dir = tempfile::tempdir().unwrap();
        let history = History::open(
            &dir.path().join("clipboard.db"),
            dir.path().join("images"),
            dir.path().join("thumbs"),
        )
        .unwrap();
        (dir, history)
    }

    fn text(s: &str) -> NewClip {
        NewClip {
            kind: ClipKind::Text,
            content: s.into(),
            hash: content::text_hash(s),
            width: 0,
            height: 0,
            source: "notepad".into(),
        }
    }

    fn titles(h: &History) -> Vec<String> {
        h.with_entries(|e| e.iter().map(|e| e.title.clone()).collect())
    }

    #[test]
    fn newest_first_and_repeats_move_up() {
        let (_dir, h) = history();
        h.add(&text("one"), 1, 10).unwrap();
        h.add(&text("two"), 2, 10).unwrap();
        h.add(&text("three"), 3, 10).unwrap();
        assert_eq!(titles(&h), ["three", "two", "one"]);
        h.add(&text("one"), 4, 10).unwrap();
        assert_eq!(titles(&h), ["one", "three", "two"]);
        assert!(h.bump_hash(&content::text_hash("two"), 5).unwrap());
        assert!(!h.bump_hash("nope", 6).unwrap());
        assert_eq!(titles(&h), ["two", "one", "three"]);
        let first = h.with_entries(|e| e[0].clone());
        assert_eq!(first.source, "notepad");
        assert_eq!(first.used_ms, 5);
    }

    #[test]
    fn caps_unpinned_entries_only() {
        let (_dir, h) = history();
        h.add(&text("keep me"), 1, 2).unwrap();
        let pinned = h.with_entries(|e| e[0].id);
        assert!(h.toggle_pin(pinned).unwrap());
        for (i, t) in ["a", "b", "c"].iter().enumerate() {
            h.add(&text(t), 10 + i as i64, 2).unwrap();
        }
        assert_eq!(titles(&h), ["keep me", "c", "b"]);
        assert!(h.with_entries(|e| e[0].pinned));
        assert!(!h.toggle_pin(pinned).unwrap());
        assert_eq!(titles(&h), ["c", "b", "keep me"]);
    }

    #[test]
    fn delete_and_clear_keep_pinned() {
        let (_dir, h) = history();
        for (i, t) in ["a", "b", "c"].iter().enumerate() {
            h.add(&text(t), i as i64, 10).unwrap();
        }
        let ids: Vec<i64> = h.with_entries(|e| e.iter().map(|e| e.id).collect());
        h.delete(ids[0]).unwrap();
        assert_eq!(titles(&h), ["b", "a"]);
        h.toggle_pin(ids[2]).unwrap();
        assert_eq!(h.clear().unwrap(), 1);
        assert_eq!(titles(&h), ["a"]);
    }

    #[test]
    fn stores_full_content_and_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("clipboard.db");
        let long = format!("start\n{}", "x".repeat(5000));
        {
            let h = History::open(&db, dir.path().into(), dir.path().into()).unwrap();
            h.add(&text(&long), 1, 10).unwrap();
            h.add(
                &NewClip {
                    kind: ClipKind::Files,
                    content: "C:\\a.txt\nC:\\b".into(),
                    hash: content::files_hash(&["C:\\a.txt".into(), "C:\\b".into()]),
                    width: 0,
                    height: 0,
                    source: "explorer".into(),
                },
                2,
                10,
            )
            .unwrap();
        }
        let h = History::open(&db, dir.path().into(), dir.path().into()).unwrap();
        let entries = h.with_entries(<[Entry]>::to_vec);
        assert_eq!(entries[0].title, "a.txt and 1 more");
        assert_eq!(entries[1].title, "start");
        assert_eq!(entries[1].preview.chars().count(), 1000);
        assert_eq!(h.stored(entries[1].id).unwrap(), Some(Stored::Text(long)));
        assert_eq!(
            h.stored(entries[0].id).unwrap(),
            Some(Stored::Files(vec!["C:\\a.txt".into(), "C:\\b".into()]))
        );
        assert_eq!(h.stored(999).unwrap(), None);
    }

    #[test]
    fn image_files_go_with_their_rows() {
        let (dir, h) = history();
        let hash = "00112233aabbccdd";
        fs::create_dir_all(dir.path().join("images")).unwrap();
        fs::create_dir_all(dir.path().join("thumbs")).unwrap();
        fs::write(h.image_path(hash), b"png").unwrap();
        fs::write(h.thumb_path(hash), b"png").unwrap();
        assert_eq!(
            h.thumb_path(hash),
            dir.path().join("thumbs/clip-00112233aabbccdd.png")
        );
        let image = NewClip {
            kind: ClipKind::Image,
            content: String::new(),
            hash: hash.into(),
            width: 640,
            height: 480,
            source: String::new(),
        };
        h.add(&image, 1, 10).unwrap();
        let entry = h.with_entries(|e| e[0].clone());
        assert_eq!(entry.title, "Image 640 × 480");
        assert_eq!(
            h.stored(entry.id).unwrap(),
            Some(Stored::Image(h.image_path(hash)))
        );
        // Pushed out by newer copies: the files go too.
        h.add(&text("newer"), 2, 1).unwrap();
        assert!(!h.image_path(hash).exists());
        assert!(!h.thumb_path(hash).exists());
    }

    #[test]
    fn sweep_removes_only_unlisted_clipboard_files() {
        let (dir, h) = history();
        let images = dir.path().join("images");
        let thumbs = dir.path().join("thumbs");
        fs::create_dir_all(&images).unwrap();
        fs::create_dir_all(&thumbs).unwrap();
        let listed = "1111111111111111";
        h.add(
            &NewClip {
                kind: ClipKind::Image,
                content: String::new(),
                hash: listed.into(),
                width: 1,
                height: 1,
                source: String::new(),
            },
            1,
            10,
        )
        .unwrap();
        for name in [listed, "2222222222222222"] {
            fs::write(images.join(format!("{name}.png")), b"png").unwrap();
            fs::write(thumbs.join(format!("clip-{name}.png")), b"png").unwrap();
        }
        // Another feature's thumbnail stays.
        fs::write(thumbs.join("window-1.png"), b"png").unwrap();
        assert_eq!(h.sweep(), 2);
        assert!(h.image_path(listed).exists() && h.thumb_path(listed).exists());
        assert!(!images.join("2222222222222222.png").exists());
        assert!(thumbs.join("window-1.png").exists());
    }
}
