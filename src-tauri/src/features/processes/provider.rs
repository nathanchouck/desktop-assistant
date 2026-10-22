//! `kill <name>`: running programs whose name (or a window title) matches,
//! one row per program, plus one per window when a program has several
//! processes with windows. Enter (asked twice) ends them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::list::{self, Group, Proc, Report};
use crate::features::system::units::{count, format_size};
use crate::fuzzy::FuzzyQuery;
use crate::icons::{self, IconCache};
use crate::platform::IconSource;
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};

#[cfg(not(windows))]
use super::fallback as os;
#[cfg(windows)]
use super::win as os;

const NAME: &str = "processes";
const PREFIX: &str = "kill";
const ID_PREFIX: &str = "proc:";
const PID_PREFIX: &str = "proc:pid:";
/// A list this recent is reused while you type; ending something always
/// looks again.
const FRESH_FOR: Duration = Duration::from_secs(2);
/// Weaker name matches are noise.
const MIN_SCORE: f32 = 0.3;
/// A program found by one of its window titles rather than its name.
const TITLE_SCORE: f32 = 0.8;
/// A window row sits just below its program's row.
const WINDOW_SCORE: f32 = 0.9;
/// Window titles longer than this are cut in subtitles.
const TITLE_CHARS: usize = 60;

/// The text after `kill` ("" for `kill` alone), if the query starts with it.
pub fn needle(query: &str) -> Option<&str> {
    let head = query.get(..PREFIX.len())?;
    if !head.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    let rest = &query[PREFIX.len()..];
    if rest.is_empty() {
        Some("")
    } else if rest.starts_with(char::is_whitespace) {
        Some(rest.trim())
    } else {
        None
    }
}

pub struct ProcessProvider {
    icons: Arc<IconCache>,
    /// The last list and when it was taken.
    cache: Mutex<Option<(Instant, Arc<Vec<Proc>>)>>,
}

impl ProcessProvider {
    pub fn new(icons: Arc<IconCache>) -> Self {
        Self {
            icons,
            cache: Mutex::new(None),
        }
    }

    /// The processes the launcher may end, from a recent list unless
    /// `fresh`. Locked while listing, so a burst of keystrokes lists once.
    async fn killable(&self, fresh: bool) -> Result<Arc<Vec<Proc>>, String> {
        let mut cache = self.cache.lock().await;
        if let Some((at, procs)) = cache.as_ref() {
            if !fresh && at.elapsed() < FRESH_FOR {
                return Ok(Arc::clone(procs));
            }
        }
        let all = tokio::task::spawn_blocking(os::snapshot)
            .await
            .map_err(|e| e.to_string())??;
        let procs = Arc::new(list::killable(&all, std::process::id()));
        *cache = Some((Instant::now(), Arc::clone(&procs)));
        Ok(procs)
    }

    async fn forget(&self) {
        *self.cache.lock().await = None;
    }
}

/// Which processes an id means.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// Every process of a program (lower-case file name).
    Program(String),
    /// One process, still running that program (PIDs get reused).
    Process(u32, String),
}

impl Target {
    fn id(&self) -> String {
        match self {
            Self::Program(exe) => format!("{ID_PREFIX}{exe}"),
            Self::Process(pid, exe) => format!("{PID_PREFIX}{pid}:{exe}"),
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        if let Some(rest) = id.strip_prefix(PID_PREFIX) {
            let (pid, exe) = rest.split_once(':')?;
            return Some(Self::Process(pid.parse().ok()?, exe.to_owned()));
        }
        let exe = id.strip_prefix(ID_PREFIX)?;
        (!exe.is_empty()).then(|| Self::Program(exe.to_owned()))
    }

    fn matches(&self, p: &Proc) -> bool {
        match self {
            Self::Program(exe) => p.exe.to_lowercase() == *exe,
            Self::Process(pid, exe) => p.pid == *pid && p.exe.to_lowercase() == *exe,
        }
    }
}

/// The icon key for a program's file (shared with file results).
fn icon_key(path: &str) -> String {
    icons::key_for(&format!("path:{}", path.to_lowercase()))
}

/// Rows for the text after `kill`, best first, at most `limit`.
fn rows(groups: &[Group], needle: &str, limit: usize) -> Vec<ResultItem> {
    let mut fuzzy = FuzzyQuery::new(needle);
    let lower = needle.to_lowercase();
    let mut found: Vec<(f32, &Group)> = groups
        .iter()
        .enumerate()
        .filter_map(|(i, g)| {
            if needle.is_empty() {
                // Keep the list's order: apps with windows, then memory.
                return Some((1.0 - i as f32 * 0.001, g));
            }
            let by_name = fuzzy.score(g.stem()).filter(|&s| s >= MIN_SCORE);
            let by_title = g
                .windows()
                .any(|t| t.to_lowercase().contains(&lower))
                .then_some(TITLE_SCORE);
            by_name
                .into_iter()
                .chain(by_title)
                .reduce(f32::max)
                .map(|s| (s, g))
        })
        .collect();
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut rows = Vec::new();
    for (score, group) in found {
        rows.push(program_row(group).with_score(score));
        // Several processes with windows (two Notepads): each can be
        // ended on its own.
        let windowed: Vec<&Proc> = group
            .procs
            .iter()
            .filter(|p| !p.windows.is_empty())
            .collect();
        if windowed.len() > 1 {
            rows.extend(
                windowed
                    .into_iter()
                    .map(|p| process_row(group, p).with_score(score * WINDOW_SCORE)),
            );
        }
        if rows.len() >= limit {
            break;
        }
    }
    rows.truncate(limit);
    rows
}

fn icon(group: &Group) -> String {
    group.path().map_or_else(|| glyph("process"), icon_key)
}

fn program_row(group: &Group) -> ResultItem {
    let n = group.procs.len();
    let mut subtitle = count(n as u64, "process", "processes");
    if let Some(title) = group.windows().next() {
        subtitle = format!("{subtitle} · {}", cut(title));
    }
    let row = ResultItem::new(
        Target::Program(group.key()).id(),
        ItemKind::Process,
        group.exe.clone(),
    )
    .with_subtitle(subtitle)
    .with_icon(icon(group))
    .with_actions([Action::End])
    .with_confirm(list::confirm(&group.exe, n));
    with_memory(row, group.memory())
}

fn process_row(group: &Group, p: &Proc) -> ResultItem {
    let title = p.windows.first().map_or("", String::as_str);
    let row = ResultItem::new(
        Target::Process(p.pid, group.key()).id(),
        ItemKind::Window,
        cut(title),
    )
    .with_subtitle(format!("{} · PID {}", group.exe, p.pid))
    .with_icon(icon(group))
    .with_actions([Action::End])
    .with_confirm(format!("End this {} process (PID {})?", group.exe, p.pid));
    with_memory(row, p.memory)
}

fn with_memory(row: ResultItem, bytes: u64) -> ResultItem {
    if bytes == 0 {
        row
    } else {
        row.with_accessory(format_size(bytes))
    }
}

fn cut(text: &str) -> String {
    if text.chars().count() <= TITLE_CHARS {
        return text.to_owned();
    }
    let kept: String = text.chars().take(TITLE_CHARS - 1).collect();
    format!("{kept}…")
}

fn notice(message: impl Into<String>) -> Activation {
    Activation::Notice {
        message: message.into(),
    }
}

#[async_trait]
impl SearchProvider for ProcessProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    fn claims(&self, query: &str) -> bool {
        needle(query).is_some()
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let Some(needle) = needle(query) else {
            return Ok(Vec::new());
        };
        let procs = self
            .killable(false)
            .await
            .map_err(|message| SearchError::Provider {
                provider: NAME,
                message,
            })?;
        let groups = list::groups(&procs);
        for g in &groups {
            if let Some(path) = g.path() {
                self.icons
                    .register(&icon_key(path), IconSource::Path(path.to_owned()));
            }
        }
        Ok(rows(&groups, needle, limit))
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(ID_PREFIX)
    }

    async fn activate(&self, id: &str, _action: Action) -> Result<Activation, SearchError> {
        let Some(target) = Target::from_id(id) else {
            return Ok(notice("That process isn't known."));
        };
        if !os::SUPPORTED {
            return Ok(notice("Ending processes works only on Windows."));
        }
        // Always a fresh look: the list may be seconds old, and a PID may
        // belong to another program by now.
        let procs = match self.killable(true).await {
            Ok(procs) => procs,
            Err(e) => return Ok(notice(format!("Couldn't list processes: {e}"))),
        };
        let chosen: Vec<Proc> = procs
            .iter()
            .filter(|p| target.matches(p))
            .cloned()
            .collect();
        let exe = chosen.first().map_or_else(
            || match &target {
                Target::Program(exe) | Target::Process(_, exe) => exe.clone(),
            },
            |p| p.exe.clone(),
        );
        let order = list::kill_order(&chosen);
        tracing::info!("ending {} {exe} process(es)", order.len());
        let outcomes = tokio::task::spawn_blocking(move || os::end(&order))
            .await
            .unwrap_or_default();
        self.forget().await;
        Ok(match list::report(&exe, &outcomes) {
            Report::Done(message) => {
                super::announce(&message);
                Activation::Refresh
            }
            Report::Problem(message) => notice(message),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, exe: &str, memory: u64, windows: &[&str]) -> Proc {
        Proc {
            pid,
            parent: 1,
            exe: exe.into(),
            path: None,
            memory,
            windows: windows.iter().map(|w| w.to_string()).collect(),
        }
    }

    fn sample() -> Vec<Group> {
        list::groups(&[
            proc(10, "chrome.exe", 300 * 1024 * 1024, &["Inbox - Gmail"]),
            proc(11, "chrome.exe", 200 * 1024 * 1024, &[]),
            proc(
                20,
                "notepad.exe",
                10 * 1024 * 1024,
                &["notes.txt - Notepad"],
            ),
            proc(21, "notepad.exe", 12 * 1024 * 1024, &["todo.txt - Notepad"]),
            proc(
                30,
                "Spotify.exe",
                150 * 1024 * 1024,
                &["Queen - Spotify Premium"],
            ),
            proc(40, "Code.exe", 0, &[]),
        ])
    }

    fn ids(rows: &[ResultItem]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn the_kill_prefix() {
        assert_eq!(needle("kill chrome"), Some("chrome"));
        assert_eq!(needle("Kill  Chrome "), Some("Chrome"));
        assert_eq!(needle("kill"), Some(""));
        assert_eq!(needle("killer"), None);
        assert_eq!(needle("kil"), None);
        assert_eq!(needle("skill x"), None);
        assert_eq!(needle("ki"), None);
        assert_eq!(needle("é"), None);
    }

    #[test]
    fn ids_round_trip() {
        for target in [
            Target::Program("chrome.exe".into()),
            Target::Process(1234, "notepad.exe".into()),
        ] {
            assert_eq!(Target::from_id(&target.id()), Some(target));
        }
        assert_eq!(Target::from_id("proc:"), None);
        assert_eq!(Target::from_id("proc:pid:x:a.exe"), None);
        assert_eq!(Target::from_id("sys:lock"), None);
    }

    #[test]
    fn a_program_row_asks_and_shows_memory() {
        let rows = rows(&sample(), "chrome", 8);
        assert_eq!(ids(&rows), ["proc:chrome.exe"]);
        let chrome = &rows[0];
        assert_eq!(chrome.title, "chrome.exe");
        assert_eq!(chrome.subtitle, "2 processes · Inbox - Gmail");
        assert_eq!(chrome.accessory.as_deref(), Some("500 MB"));
        assert_eq!(
            chrome.confirm.as_deref(),
            Some("End all 2 chrome.exe processes?")
        );
        assert_eq!(chrome.actions, [Action::End]);
        assert_eq!(chrome.icon.as_deref(), Some("glyph:process"));
    }

    #[test]
    fn several_windows_get_a_row_each() {
        let rows = rows(&sample(), "notepad", 8);
        assert_eq!(
            ids(&rows),
            [
                "proc:notepad.exe",
                "proc:pid:20:notepad.exe",
                "proc:pid:21:notepad.exe"
            ]
        );
        assert_eq!(rows[1].title, "notes.txt - Notepad");
        assert_eq!(rows[1].subtitle, "notepad.exe · PID 20");
        assert_eq!(rows[1].kind, ItemKind::Window);
        assert!(rows[1].score < rows[0].score);
    }

    #[test]
    fn found_by_window_title_too() {
        let rows = rows(&sample(), "queen", 8);
        assert_eq!(ids(&rows), ["proc:spotify.exe"]);
        assert!((rows[0].score - TITLE_SCORE).abs() < f32::EPSILON);
    }

    #[test]
    fn kill_alone_lists_everything_in_order() {
        let rows = rows(&sample(), "", 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "proc:chrome.exe");
        let all = super::rows(&sample(), "", 50);
        assert!(ids(&all).contains(&"proc:code.exe"));
        let code = all.iter().find(|r| r.id == "proc:code.exe").unwrap();
        assert_eq!(code.accessory, None, "unknown memory isn't shown");
        assert!(super::rows(&sample(), "zzzz", 8).is_empty());
    }

    #[test]
    fn its_icon_is_drawn() {
        assert!(crate::features::system::matching::glyph_exists("process"));
    }

    #[test]
    fn long_titles_are_cut() {
        assert_eq!(cut("short"), "short");
        let long = "x".repeat(100);
        assert_eq!(cut(&long).chars().count(), TITLE_CHARS);
    }

    #[tokio::test]
    async fn claims_kill_and_owns_its_ids() {
        let dir = tempfile::tempdir().unwrap();
        let shell = Arc::new(crate::platform::Shell::new());
        let p = ProcessProvider::new(Arc::new(IconCache::new(dir.path().into(), shell)));
        assert!(p.claims("kill chrome") && p.claims("kill"));
        assert!(!p.claims("killer queen"));
        assert!(p.owns("proc:chrome.exe") && p.owns("proc:pid:1:a.exe"));
        assert!(!p.owns("sys:lock"));
        assert!(p.query("chrome", 8).await.unwrap().is_empty());
    }
}
