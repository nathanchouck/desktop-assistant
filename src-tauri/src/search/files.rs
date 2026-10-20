//! File and folder search. Uses Everything (voidtools) when it's running,
//! which covers every NTFS drive; otherwise the launcher's own index of the
//! folders in `search.index_folders` (see `file_index`).
//!
//! Either way we score candidates by how well the name matches and how
//! recently the file changed. Files rank a little below apps with an equally
//! good match.

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use super::{Action, ItemKind, ResultItem, SearchError, SearchProvider};
use crate::file_index::{FileIndex, IndexEntry};
use crate::fuzzy::FuzzyQuery;
use crate::icons::{self, IconCache};
use crate::platform::{FileHit, FileSearch, IconSource, PlatformError, Shell};

const ID_PREFIX: &str = "file:";

/// Shorter queries match too much to be useful.
const MIN_QUERY_CHARS: usize = 2;
/// Candidates gathered before the final ranking.
const MAX_HITS: usize = 100;
/// Files rank a little below an equally good app or command match.
const FILE_WEIGHT: f32 = 0.8;
/// Name score when there's nothing to fuzzy-match (only `ext:pdf`, or an
/// Everything match in the path).
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

/// What the built-in index understands of a query: name words, a kind
/// filter, and `ext:` (e.g. `ext:pdf` or `ext:jpg;png`). Other
/// Everything-only syntax is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexQuery {
    words: String,
    kind: KindFilter,
    extensions: Vec<String>,
}

impl IndexQuery {
    fn parse(kind: KindFilter, text: &str) -> Self {
        let mut words = Vec::new();
        let mut extensions = Vec::new();
        for token in text.split_whitespace() {
            if let Some(exts) = token.strip_prefix("ext:") {
                extensions.extend(
                    exts.split([';', ','])
                        .map(|e| e.trim_start_matches('.').to_lowercase())
                        .filter(|e| !e.is_empty()),
                );
            } else if !token.contains(':') {
                words.push(token);
            }
        }
        Self {
            words: words.join(" "),
            kind,
            extensions,
        }
    }

    /// A scorer for index entries: `None` when the entry doesn't match.
    fn scorer(&self, now: i64) -> impl FnMut(&IndexEntry) -> Option<f32> + use<'_> {
        let mut fuzzy = (!self.words.is_empty()).then(|| FuzzyQuery::new(&self.words));
        move |entry: &IndexEntry| {
            match self.kind {
                KindFilter::Files if entry.is_folder => return None,
                KindFilter::Folders if !entry.is_folder => return None,
                _ => {}
            }
            if !self.extensions.is_empty() {
                let ext = extension(&entry.name);
                if entry.is_folder || !self.extensions.contains(&ext) {
                    return None;
                }
            }
            let name_score = match fuzzy.as_mut() {
                Some(f) => f.score(&entry.name)?,
                None => UNSCORED,
            };
            Some(file_score(name_score, entry.modified, now))
        }
    }
}

pub struct FileProvider {
    shell: Arc<Shell>,
    icons: Arc<IconCache>,
    index: Arc<FileIndex>,
    exclusions: Vec<String>,
    home: Option<String>,
}

impl FileProvider {
    pub fn new(
        shell: Arc<Shell>,
        icons: Arc<IconCache>,
        index: Arc<FileIndex>,
        exclusions: Vec<String>,
    ) -> Self {
        Self {
            shell,
            icons,
            index,
            exclusions,
            home: dirs::home_dir().map(|h| h.display().to_string()),
        }
    }

    /// Searches the built-in index (starting it if needed) off the async
    /// threads: scoring a few hundred thousand names takes ~15 ms.
    async fn search_index(&self, kind: KindFilter, text: &str) -> Vec<FileHit> {
        self.index.ensure_started();
        let index = Arc::clone(&self.index);
        let query = IndexQuery::parse(kind, text);
        let now = unix_now();
        tokio::task::spawn_blocking(move || index.search(query.scorer(now), MAX_HITS))
            .await
            .unwrap_or_default()
    }

    fn to_items(&self, hits: Vec<FileHit>, text: &str, limit: usize) -> Vec<ResultItem> {
        let now = unix_now();
        let words = IndexQuery::parse(KindFilter::Any, text).words;
        let mut fuzzy = (!words.is_empty()).then(|| FuzzyQuery::new(&words));

        let mut items: Vec<ResultItem> = hits
            .into_iter()
            .map(|hit| {
                let name_score = fuzzy
                    .as_mut()
                    .and_then(|f| f.score(&hit.name))
                    .unwrap_or(UNSCORED);
                let score = file_score(name_score, hit.modified, now);
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
        let started = Instant::now();
        let search = FileSearch {
            search: everything_search(kind, text, &self.exclusions),
            max: MAX_HITS as u32,
        };
        let hits = match self.shell.search_files(search).await {
            Ok(hits) => {
                tracing::debug!("everything: {} hits in {:?}", hits.len(), started.elapsed());
                hits
            }
            Err(PlatformError::EverythingNotRunning | PlatformError::Unsupported) => {
                let hits = self.search_index(kind, text).await;
                tracing::debug!("file index: {} hits in {:?}", hits.len(), started.elapsed());
                hits
            }
            Err(PlatformError::Superseded) => return Ok(Vec::new()),
            Err(e) => {
                return Err(SearchError::Provider {
                    provider: "files",
                    message: e.to_string(),
                })
            }
        };
        Ok(self.to_items(hits, text, limit))
    }
}

fn file_score(name_score: f32, modified: Option<i64>, now: i64) -> f32 {
    let recency = modified.map_or(0.0, |m| recency(now - m));
    FILE_WEIGHT * name_score * (1.0 - RECENCY_SHARE + RECENCY_SHARE * recency)
}

/// 1.0 for "just now", halving every 30 days.
fn recency(age_secs: i64) -> f32 {
    let days = age_secs.max(0) as f32 / 86_400.0;
    0.5_f32.powf(days / RECENCY_HALF_LIFE_DAYS)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_lowercase())
        .unwrap_or_default()
}

/// Most files share their extension's icon (one cache entry per type);
/// programs, shortcuts and folders get their own.
fn icon_for(hit: &FileHit) -> (String, IconSource) {
    let ext = extension(&hit.name);
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

    fn entry(name: &str, is_folder: bool) -> IndexEntry {
        IndexEntry {
            name: name.into(),
            is_folder,
            modified: None,
        }
    }

    fn provider() -> (tempfile::TempDir, FileProvider) {
        let dir = tempfile::tempdir().unwrap();
        let shell = Arc::new(Shell::new());
        let icons = Arc::new(IconCache::new(dir.path().join("icons"), Arc::clone(&shell)));
        let index = Arc::new(FileIndex::new(&[], &[], dir.path().join("index.txt")));
        (dir, FileProvider::new(shell, icons, index, Vec::new()))
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
    fn index_queries_understand_ext_and_kind() {
        let q = IndexQuery::parse(KindFilter::Any, "scan ext:pdf;.PNG size:>1mb");
        assert_eq!(q.words, "scan");
        assert_eq!(q.extensions, ["pdf", "png"]);

        let mut score = q.scorer(0);
        assert!(score(&entry("scan.pdf", false)).is_some());
        assert!(score(&entry("Scan.PNG", false)).is_some());
        assert!(score(&entry("scan.docx", false)).is_none());
        assert!(
            score(&entry("scan.pdf", true)).is_none(),
            "folders have no extension"
        );

        let folders = IndexQuery::parse(KindFilter::Folders, "proj");
        let mut score = folders.scorer(0);
        assert!(score(&entry("projects", true)).is_some());
        assert!(score(&entry("projects.txt", false)).is_none());
    }

    #[test]
    fn only_ext_still_matches() {
        let q = IndexQuery::parse(KindFilter::Any, "ext:pdf");
        assert!(q.words.is_empty());
        assert!(q.scorer(0)(&entry("anything.pdf", false)).is_some());
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
        let (_dir, provider) = provider();
        let now = unix_now();
        let items = provider.to_items(
            vec![
                hit(r"C:\x\old report.docx", false, Some(now - 400 * 86_400)),
                hit(r"C:\x\report.docx", false, Some(now)),
                hit(r"C:\x\r_e_p_o_r_t notes.txt", false, Some(now)),
            ],
            "report",
            8,
        );
        assert_eq!(items[0].title, "report.docx");
        assert_eq!(items[0].id, r"file:C:\x\report.docx");
        assert_eq!(items[0].actions[0], Action::Open);
        assert!(items.iter().all(|i| i.score <= FILE_WEIGHT));
    }

    #[test]
    fn everything_syntax_still_gets_a_score() {
        let (_dir, provider) = provider();
        let items = provider.to_items(vec![hit(r"C:\x\scan.pdf", false, None)], "ext:pdf", 8);
        assert_eq!(items.len(), 1);
        assert!(items[0].score > 0.0);
    }

    /// Without Everything (always the case off Windows), results come from
    /// the built-in index.
    #[tokio::test]
    async fn falls_back_to_the_built_in_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("home");
        std::fs::create_dir_all(root.join("School")).unwrap();
        std::fs::write(root.join("School/orbit notes.md"), b"x").unwrap();
        let shell = Arc::new(Shell::new());
        let icons = Arc::new(IconCache::new(dir.path().join("icons"), Arc::clone(&shell)));
        let index = Arc::new(FileIndex::new(&[root], &[], dir.path().join("index.txt")));
        let provider = FileProvider::new(shell, icons, Arc::clone(&index), Vec::new());

        index.ensure_started();
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while index.is_empty() && Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let items = provider.query("orbit", 8).await.unwrap();
        assert_eq!(items[0].title, "orbit notes.md");
        assert!(items[0].subtitle.ends_with("School"));
    }

    #[test]
    fn ids_round_trip() {
        assert_eq!(path_from_id(r"file:C:\a b\c.txt"), Some(r"C:\a b\c.txt"));
        assert_eq!(path_from_id("app:x"), None);
    }
}
