//! Google Drive files in normal search: the request, its answer and the
//! rows (pure, tested with a recorded answer).

use chrono::DateTime;
use serde::Deserialize;

use crate::features::clipboard::content::age;
use crate::fuzzy::FuzzyQuery;
use crate::net;
use crate::search::files::file_score;
use crate::search::{glyph, Action, ItemKind, ResultItem};

pub const HOST: &str = "www.googleapis.com";
pub const ID_PREFIX: &str = "gdrive:";
/// Shorter queries match too much to be worth a request.
pub const MIN_QUERY_CHARS: usize = 3;
/// Files asked for per query.
const PAGE_SIZE: &str = "8";
/// Under a local file with the same name and age: the copy on this PC
/// opens faster and works offline.
const DRIVE_WEIGHT: f32 = 0.95;
/// Drive matched by a word start the fuzzy matcher doesn't see
/// ("q3 report" for "Report Q3").
const UNSCORED: f32 = 0.5;

/// Whether `query` should go to Drive.
pub fn wanted(query: &str) -> bool {
    query.trim().chars().count() >= MIN_QUERY_CHARS
}

/// Text for a Drive query string literal: `\` and `'` escaped.
pub fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}

/// `files.list` for names containing `text`, newest first.
pub fn files_path(text: &str) -> String {
    let q = format!(
        "name contains '{}' and trashed = false",
        escape(text.trim())
    );
    let query = net::form(&[
        ("q", &q),
        ("fields", "files(id,name,mimeType,modifiedTime,webViewLink)"),
        ("pageSize", PAGE_SIZE),
        ("orderBy", "modifiedTime desc"),
        ("spaces", "drive"),
    ]);
    format!("/drive/v3/files?{query}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    /// Unix milliseconds.
    pub modified_ms: Option<i64>,
    pub link: String,
}

#[derive(Deserialize)]
struct FileList {
    #[serde(default)]
    files: Vec<FileJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileJson {
    id: String,
    name: String,
    #[serde(default)]
    mime_type: String,
    modified_time: Option<String>,
    web_view_link: Option<String>,
}

pub fn parse_files(body: &[u8]) -> Result<Vec<DriveFile>, String> {
    let list: FileList = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    Ok(list
        .files
        .into_iter()
        .map(|f| DriveFile {
            link: f
                .web_view_link
                .filter(|l| l.starts_with("https://"))
                .unwrap_or_else(|| fallback_link(&f.id)),
            modified_ms: f
                .modified_time
                .as_deref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.timestamp_millis()),
            id: f.id,
            name: f.name,
            mime_type: f.mime_type,
        })
        .collect())
}

/// Opens any file by id, for a file no longer in the cache.
pub fn fallback_link(id: &str) -> String {
    format!("https://drive.google.com/open?id={}", net::encode(id))
}

/// Rows scored like local files (name match and age), a little lower.
pub fn rows(files: &[DriveFile], query: &str, now_ms: i64) -> Vec<ResultItem> {
    let mut fuzzy = FuzzyQuery::new(query.trim());
    files
        .iter()
        .map(|file| {
            let name_score = fuzzy.score(&file.name).unwrap_or(UNSCORED);
            let modified = file.modified_ms.map(|ms| ms / 1000);
            let score = file_score(name_score, modified, now_ms / 1000) * DRIVE_WEIGHT;
            let subtitle = match file.modified_ms {
                Some(ms) => format!("Google Drive · {}", age(now_ms, ms)),
                None => "Google Drive".to_owned(),
            };
            ResultItem::new(
                format!("{ID_PREFIX}{}", file.id),
                ItemKind::Link,
                &file.name,
            )
            .with_subtitle(subtitle)
            .with_icon(glyph("drive"))
            // Ctrl+Shift+C copies the link.
            .with_actions([Action::Open, Action::Copy])
            .with_score(score)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/google/files.json");
    /// 23 Oct 2026, 08:00 UTC.
    const NOW_MS: i64 = 1_792_742_400_000;

    #[test]
    fn escapes_query_text() {
        assert_eq!(escape("Ana's notes"), "Ana\\'s notes");
        assert_eq!(escape(r"a\b"), r"a\\b");
        assert_eq!(escape(r"it\'s"), r"it\\\'s");
        let path = files_path("  Ana's plan ");
        assert!(path.starts_with("/drive/v3/files?q=name%20contains%20%27Ana%5C%27s%20plan%27%20and%20trashed%20%3D%20false&"), "{path}");
        assert!(path.contains("pageSize=8"));
        assert!(path.contains("orderBy=modifiedTime%20desc"));
    }

    #[test]
    fn only_longer_queries() {
        assert!(!wanted("ab"));
        assert!(!wanted(" ab "));
        assert!(wanted("abc"));
        assert!(wanted("éèê"));
    }

    #[test]
    fn parses_the_recorded_answer() {
        let files = parse_files(FIXTURE.as_bytes()).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].name, "Budget 2026");
        assert_eq!(
            files[0].link,
            "https://docs.google.com/spreadsheets/d/1AbC/edit?usp=drivesdk"
        );
        assert_eq!(files[0].modified_ms, Some(1_792_738_800_000));
        // No link given: opened by id.
        assert_eq!(files[2].link, "https://drive.google.com/open?id=1GhI");
        assert_eq!(files[2].modified_ms, None);
        assert!(parse_files(b"{}").unwrap().is_empty());
        assert!(parse_files(b"oops").is_err());
    }

    #[test]
    fn rows_rank_under_an_equal_local_file() {
        let files = parse_files(FIXTURE.as_bytes()).unwrap();
        let rows = rows(&files, "budget", NOW_MS);
        assert_eq!(rows[0].id, "gdrive:1AbC");
        assert_eq!(rows[0].subtitle, "Google Drive · 1 h ago");
        assert_eq!(rows[0].icon.as_deref(), Some("glyph:drive"));
        assert_eq!(rows[0].kind, ItemKind::Link);
        assert_eq!(rows[0].actions, [Action::Open, Action::Copy]);
        let local = file_score(1.0, Some(NOW_MS / 1000 - 3600), NOW_MS / 1000);
        assert!(rows[0].score < local, "{} vs {local}", rows[0].score);
        assert!(rows[0].score > 0.7);
        // An older file scores lower.
        assert!(rows[1].score < rows[0].score);
        assert_eq!(rows[2].subtitle, "Google Drive");
    }
}
