//! File and folder search through Everything (voidtools), which keeps an
//! index of every file on NTFS drives and answers in milliseconds.
//!
//! Everything finds candidates (most recently modified first); we re-score
//! them by how well the name matches and how recent they are. Files rank a
//! little below apps with an equally good match.

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use super::{Action, ItemKind, ResultItem, SearchError, SearchProvider};
use crate::fuzzy::FuzzyQuery;
use crate::icons::{self, IconCache};
use crate::platform::{self, FileHit, FileSearch, IconSource, PlatformError, Shell};

const ID_PREFIX: &str = "file:";
pub const SETUP_START_ID: &str = "setup:everything-start";
pub const SETUP_INSTALL_ID: &str = "setup:everything-install";
pub const EVERYTHING_DOWNLOAD_URL: &str = "https://www.voidtools.com/downloads/";

/// Shorter queries match too much to be useful (and cost a round trip).
const MIN_QUERY_CHARS: usize = 2;
/// Candidates asked from Everything before re-scoring.
const MAX_HITS: u32 = 100;
/// Files rank a little below an equally good app or command match.
const FILE_WEIGHT: f32 = 0.8;
/// Score when the name can't be fuzzy-matched (Everything-only syntax such
/// as `ext:pdf`, or a match in the path).
const UNSCORED: f32 = 0.5;
/// Recency counts for this share of a file's score; it halves every 30 days.
const RECENCY_SHARE: f32 = 0.2;
const RECENCY_HALF_LIFE_DAYS: f32 = 30.0;
/// File types whose icon is specific to the file, not the extension.
const OWN_ICON_EXTENSIONS: [&str; 8] =
    ["exe", "lnk", "ico", "url", "appref-ms", "msc", "cpl", "scr"];

/// The path inside a result id, if it is a file or folder.
pub fn path_from_id(id: &str) -> Option<&str> {
    id.strip_prefix(ID_PREFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFilter {
    Any,
    Files,
    Folders,
}

/// Splits off the `f ` (files only) and `d ` (folders only) prefixes.
/// `None` when the rest is too short to search.
pub fn parse_query(raw: &str) -> Option<(KindFilter, &str)> {
    let query = raw.trim_start();
    let (kind, rest) = if let Some(rest) = query.strip_prefix("f ") {
        (KindFilter::Files, rest)
    } else if let Some(rest) = query.strip_prefix("d ") {
        (KindFilter::Folders, rest)
    } else {
        (KindFilter::Any, query)
    };
    let rest = rest.trim();
    (rest.chars().count() >= MIN_QUERY_CHARS).then_some((kind, rest))
}

/// The Everything query: the user's text (Everything syntax like `ext:pdf`
/// passes straight through), a kind filter, and one `!path:` per excluded
/// folder so nothing inside it shows up.
pub fn everything_search(kind: KindFilter, text: &str, exclusions: &[String]) -> String {
    let mut search = String::new();
    match kind {
        KindFilter::Files => search.push_str("file: "),
        KindFilter::Folders => search.push_str("folder: "),
        KindFilter::Any => {}
    }
    search.push_str(text);
    for folder in exclusions {
        let folder = folder.trim_matches(['\\', '/']).replace('"', "");
        if !folder.is_empty() {
            search.push_str(&format!(" !path:\"\\{folder}\\\""));
        }
    }
    search
}

pub struct FileProvider {
    shell: Arc<Shell>,
    icons: Arc<IconCache>,
    exclusions: Vec<String>,
    home: Option<String>,
}

impl FileProvider {
    pub fn new(shell: Arc<Shell>, icons: Arc<IconCache>, exclusions: Vec<String>) -> Self {
        Self {
            shell,
            icons,
            exclusions,
            home: dirs::home_dir().map(|h| h.display().to_string()),
        }
    }

    fn to_items(&self, hits: Vec<FileHit>, text: &str, limit: usize) -> Vec<ResultItem> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        // Everything syntax (`ext:pdf`, `size:>1mb`) isn't part of the name.
        let words: Vec<&str> = text
            .split_whitespace()
            .filter(|w| !w.contains(':'))
            .collect();
        let mut fuzzy = (!words.is_empty()).then(|| FuzzyQuery::new(&words.join(" ")));

        let mut items: Vec<ResultItem> = hits
            .into_iter()
            .map(|hit| {
                let name_score = fuzzy
                    .as_mut()
                    .and_then(|f| f.score(&hit.name))
                    .unwrap_or(UNSCORED);
                let recency = hit.modified.map_or(0.0, |m| recency(now - m));
                let score =
                    FILE_WEIGHT * name_score * (1.0 - RECENCY_SHARE + RECENCY_SHARE * recency);
                let (icon_key, icon_source) = icon_for(&hit);
                self.icons.register(&icon_key, icon_source);
                ResultItem {
                    id: format!("{ID_PREFIX}{}", hit.path),
                    kind: if hit.is_folder {
                        ItemKind::Folder
                    } else {
                        ItemKind::File
                    },
                    subtitle: display_dir(&hit.dir, self.home.as_deref()),
                    title: hit.name,
                    icon: Some(icon_key),
                    score,
                    actions: vec![Action::Open, Action::RevealInExplorer, Action::CopyPath],
                }
            })
            .collect();
        items.sort_by(|a, b| b.score.total_cmp(&a.score));
        items.truncate(limit);
        items
    }
}

#[async_trait]
impl SearchProvider for FileProvider {
    fn name(&self) -> &'static str {
        "files"
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let Some((kind, text)) = parse_query(query) else {
            return Ok(Vec::new());
        };
        let search = FileSearch {
            search: everything_search(kind, text, &self.exclusions),
            max: MAX_HITS,
        };
        let started = Instant::now();
        match self.shell.search_files(search).await {
            Ok(hits) => {
                tracing::debug!("everything: {} hits in {:?}", hits.len(), started.elapsed());
                Ok(self.to_items(hits, text, limit))
            }
            Err(PlatformError::EverythingNotRunning) => Ok(vec![setup_item()]),
            Err(PlatformError::Superseded | PlatformError::Unsupported) => Ok(Vec::new()),
            Err(e) => Err(SearchError::Provider {
                provider: "files",
                message: e.to_string(),
            }),
        }
    }
}

/// 1.0 for "just now", halving every 30 days.
fn recency(age_secs: i64) -> f32 {
    let days = age_secs.max(0) as f32 / 86_400.0;
    0.5_f32.powf(days / RECENCY_HALF_LIFE_DAYS)
}

/// Most files share their extension's icon (one cache entry per type);
/// programs, shortcuts and folders get their own.
fn icon_for(hit: &FileHit) -> (String, IconSource) {
    let ext = hit
        .name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_lowercase())
        .unwrap_or_default();
    let identity = if hit.is_folder || ext.is_empty() || OWN_ICON_EXTENSIONS.contains(&ext.as_str())
    {
        format!("path:{}", hit.path.to_lowercase())
    } else {
        format!("ext:{ext}")
    };
    (
        icons::key_for(&identity),
        IconSource::Path(hit.path.clone()),
    )
}

/// The folder a hit lives in, with the home folder shortened to `~`.
fn display_dir(dir: &str, home: Option<&str>) -> String {
    if let Some(home) = home {
        let lower = dir.to_lowercase();
        let home_lower = home.to_lowercase();
        if lower == home_lower {
            return "~".into();
        }
        if lower.starts_with(&home_lower) && dir[home.len()..].starts_with(['\\', '/']) {
            return format!("~{}", &dir[home.len()..]);
        }
    }
    dir.to_owned()
}

/// Shown instead of file results when Everything isn't running.
fn setup_item() -> ResultItem {
    let (id, title, subtitle) = if platform::everything_exe().is_some() {
        (
            SETUP_START_ID,
            "Start Everything to search files",
            "Everything is installed but not running",
        )
    } else {
        (
            SETUP_INSTALL_ID,
            "Install Everything to search files",
            "Free from voidtools.com",
        )
    };
    ResultItem {
        id: id.into(),
        kind: ItemKind::Command,
        title: title.into(),
        subtitle: subtitle.into(),
        icon: None,
        score: 0.05,
        actions: vec![Action::Run],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str, is_folder: bool, modified: Option<i64>) -> FileHit {
        let (dir, name) = path.rsplit_once('\\').unwrap();
        FileHit {
            path: path.into(),
            name: name.into(),
            dir: dir.into(),
            is_folder,
            modified,
        }
    }

    #[test]
    fn parses_prefixes_and_minimum_length() {
        assert_eq!(parse_query("report"), Some((KindFilter::Any, "report")));
        assert_eq!(parse_query("f report"), Some((KindFilter::Files, "report")));
        assert_eq!(parse_query("d proj"), Some((KindFilter::Folders, "proj")));
        assert_eq!(parse_query("r"), None);
        assert_eq!(parse_query("f r"), None);
        assert_eq!(parse_query("  "), None);
    }

    #[test]
    fn builds_everything_queries() {
        let exclusions = vec!["node_modules".to_owned(), r"AppData\Local\Temp\".to_owned()];
        assert_eq!(
            everything_search(KindFilter::Files, "report ext:pdf", &exclusions),
            r#"file: report ext:pdf !path:"\node_modules\" !path:"\AppData\Local\Temp\""#
        );
        assert_eq!(everything_search(KindFilter::Any, "x", &[]), "x");
        assert_eq!(
            everything_search(KindFilter::Folders, "x", &[]),
            "folder: x"
        );
    }

    #[test]
    fn exclusions_cannot_break_out_of_quotes() {
        let search = everything_search(KindFilter::Any, "x", &[r#"bad" name"#.to_owned()]);
        assert_eq!(search, r#"x !path:"\bad name\""#);
    }

    #[test]
    fn shortens_home() {
        let home = Some(r"C:\Users\nathan");
        assert_eq!(
            display_dir(r"C:\Users\nathan\Documents", home),
            r"~\Documents"
        );
        assert_eq!(display_dir(r"c:\users\NATHAN\Desktop", home), r"~\Desktop");
        assert_eq!(display_dir(r"C:\Users\nathan", home), "~");
        assert_eq!(
            display_dir(r"C:\Users\nathanb\Docs", home),
            r"C:\Users\nathanb\Docs"
        );
        assert_eq!(display_dir(r"D:\Projects", home), r"D:\Projects");
    }

    #[test]
    fn icons_are_shared_per_extension_but_not_for_programs() {
        let a = icon_for(&hit(r"C:\a\one.pdf", false, None)).0;
        let b = icon_for(&hit(r"C:\b\two.PDF", false, None)).0;
        let exe1 = icon_for(&hit(r"C:\a\app.exe", false, None)).0;
        let exe2 = icon_for(&hit(r"C:\b\app.exe", false, None)).0;
        assert_eq!(a, b);
        assert_ne!(exe1, exe2);
    }

    #[test]
    fn better_names_and_newer_files_rank_higher() {
        let dir = tempfile::tempdir().unwrap();
        let icons = Arc::new(IconCache::new(dir.path().into(), Arc::new(Shell::new())));
        let provider = FileProvider::new(Arc::new(Shell::new()), icons, Vec::new());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let items = provider.to_items(
            vec![
                hit(r"C:\x\old report.docx", false, Some(now - 400 * 86_400)),
                hit(r"C:\x\report.docx", false, Some(now)),
                hit(r"C:\x\r_e_p_o_r_t notes.txt", false, Some(now)),
            ],
            "report",
            8,
        );
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles[0], "report.docx");
        assert_eq!(items[0].id, r"file:C:\x\report.docx");
        assert_eq!(items[0].actions[0], Action::Open);
        assert!(items.iter().all(|i| i.score <= FILE_WEIGHT));
    }

    #[test]
    fn everything_syntax_still_gets_a_score() {
        let dir = tempfile::tempdir().unwrap();
        let icons = Arc::new(IconCache::new(dir.path().into(), Arc::new(Shell::new())));
        let provider = FileProvider::new(Arc::new(Shell::new()), icons, Vec::new());
        let items = provider.to_items(vec![hit(r"C:\x\scan.pdf", false, None)], "ext:pdf", 8);
        assert_eq!(items.len(), 1);
        assert!(items[0].score > 0.0);
    }

    #[test]
    fn ids_round_trip() {
        assert_eq!(path_from_id(r"file:C:\a b\c.txt"), Some(r"C:\a b\c.txt"));
        assert_eq!(path_from_id("app:x"), None);
    }
}
