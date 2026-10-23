//! Drive files for what you type. Drive can't be searched ahead of time,
//! so each query (3+ characters) is asked for once you pause typing
//! (`DEBOUNCE`) and remembered for a while. A cached query answers at once;
//! a new one misses the 40 ms budget and its rows arrive as late results
//! (`Outcome::complete`, D-020).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use super::api::{Api, ApiError, Service, QUICK_TIMEOUT};
use super::drive::{self, DriveFile};

/// Typing faster than this doesn't send a request per letter.
pub const DEBOUNCE: Duration = Duration::from_millis(250);
/// How long a query's files are reused.
const KEEP_FOR: Duration = Duration::from_secs(2 * 60);
/// Queries remembered at most.
const KEEP_AT_MOST: usize = 64;
/// After a failure, Drive is left alone this long (offline, API off).
const BACK_OFF: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct DriveSearch {
    found: Mutex<HashMap<String, (Instant, Vec<DriveFile>)>>,
    /// Bumped by each search; a search that isn't the newest after the
    /// pause gives up without asking Drive.
    generation: AtomicU64,
    failed_at: Mutex<Option<Instant>>,
}

/// Queries that differ only in case or spacing share an answer.
fn key(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

impl DriveSearch {
    /// Files remembered for `query`, if still fresh.
    pub fn cached(&self, query: &str) -> Option<Vec<DriveFile>> {
        let found = self.found.lock().unwrap_or_else(|p| p.into_inner());
        found
            .get(&key(query))
            .filter(|(at, _)| at.elapsed() < KEEP_FOR)
            .map(|(_, files)| files.clone())
    }

    /// A remembered file, for opening it.
    pub fn file(&self, id: &str) -> Option<DriveFile> {
        let found = self.found.lock().unwrap_or_else(|p| p.into_inner());
        found
            .values()
            .flat_map(|(_, files)| files)
            .find(|f| f.id == id)
            .cloned()
    }

    fn remember(&self, query: &str, files: Vec<DriveFile>) {
        let mut found = self.found.lock().unwrap_or_else(|p| p.into_inner());
        found.retain(|_, (at, _)| at.elapsed() < KEEP_FOR);
        if found.len() >= KEEP_AT_MOST {
            if let Some(oldest) = found
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
            {
                found.remove(&oldest);
            }
        }
        found.insert(key(query), (Instant::now(), files));
    }

    fn backing_off(&self) -> bool {
        self.failed_at
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some_and(|at| at.elapsed() < BACK_OFF)
    }

    /// The files for `query`: remembered ones at once, else asked for
    /// after the pause, unless a newer search came in meanwhile.
    pub async fn search(&self, api: &Api, query: &str) -> Vec<DriveFile> {
        let mine = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(files) = self.cached(query) {
            return files;
        }
        if self.backing_off() {
            return Vec::new();
        }
        tokio::time::sleep(DEBOUNCE).await;
        if self.generation.load(Ordering::Relaxed) != mine {
            return Vec::new();
        }
        let path = drive::files_path(query);
        let result = api
            .get(Service::Drive, drive::HOST, &path, QUICK_TIMEOUT)
            .await
            .and_then(|body| drive::parse_files(&body).map_err(ApiError::Unreadable));
        match result {
            Ok(files) => {
                self.remember(query, files.clone());
                files
            }
            Err(e) => {
                // Queries are never logged.
                tracing::warn!("Drive search failed: {e}");
                *self.failed_at.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
                Vec::new()
            }
        }
    }

    pub fn clear(&self) {
        self.found.lock().unwrap_or_else(|p| p.into_inner()).clear();
        *self.failed_at.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    #[cfg(test)]
    pub fn put(&self, query: &str, files: Vec<DriveFile>) {
        self.remember(query, files);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: &str) -> DriveFile {
        DriveFile {
            id: id.into(),
            name: format!("File {id}"),
            mime_type: String::new(),
            modified_ms: None,
            link: format!("https://drive.google.com/open?id={id}"),
        }
    }

    #[test]
    fn remembers_by_query() {
        let search = DriveSearch::default();
        search.put("Budget  Plan", vec![file("a")]);
        assert_eq!(search.cached(" budget plan").map(|f| f.len()), Some(1));
        assert!(search.cached("budget").is_none());
        assert_eq!(search.file("a").map(|f| f.name), Some("File a".into()));
        assert!(search.file("b").is_none());
        for i in 0..(KEEP_AT_MOST + 5) {
            search.put(&format!("q{i}"), vec![file(&i.to_string())]);
        }
        assert!(search.found.lock().unwrap().len() <= KEEP_AT_MOST);
        search.clear();
        assert!(search.cached(&format!("q{}", KEEP_AT_MOST + 4)).is_none());
    }
}
