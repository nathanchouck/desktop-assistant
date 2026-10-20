//! Installed apps: an in-memory index of the Start menu's "All apps", and the
//! search provider that matches against it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::{Action, ItemKind, ResultItem, SearchError, SearchProvider};
use crate::fuzzy::FuzzyQuery;
use crate::icons::{self, IconCache};
use crate::platform::{PlatformError, RawApp, Shell};

const ID_PREFIX: &str = "app:";
/// Keyword matches (e.g. the exe name) count a little less than the title.
const KEYWORD_WEIGHT: f32 = 0.9;
/// Quiet time after a Start menu change before re-indexing.
const WATCH_DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
pub struct AppEntry {
    /// Result id: "app:<target>".
    pub id: String,
    pub name: String,
    /// Extra words that find this app, e.g. "code" for Code.exe.
    pub keywords: Vec<String>,
    pub icon: String,
    /// Desktop programs can run as administrator; Store apps can't.
    pub elevatable: bool,
}

/// The app target inside a result id, if it is an app.
pub fn target_from_id(id: &str) -> Option<&str> {
    id.strip_prefix(ID_PREFIX)
}

pub struct AppIndex {
    shell: Arc<Shell>,
    icons: Arc<IconCache>,
    apps: RwLock<Arc<Vec<AppEntry>>>,
    indexed_at: Mutex<Option<Instant>>,
    refreshing: AtomicBool,
}

impl AppIndex {
    pub fn new(shell: Arc<Shell>, icons: Arc<IconCache>) -> Self {
        Self {
            shell,
            icons,
            apps: RwLock::new(Arc::new(Vec::new())),
            indexed_at: Mutex::new(None),
            refreshing: AtomicBool::new(false),
        }
    }

    pub fn snapshot(&self) -> Arc<Vec<AppEntry>> {
        self.apps.read().map(|a| Arc::clone(&a)).unwrap_or_default()
    }

    /// Re-reads the app list from the OS. A refresh already in progress makes
    /// this a no-op.
    pub async fn refresh(&self) -> Result<usize, PlatformError> {
        if self.refreshing.swap(true, Ordering::AcqRel) {
            return Ok(self.snapshot().len());
        }
        let started = Instant::now();
        let listed = self.shell.list_apps().await;
        self.refreshing.store(false, Ordering::Release);

        let entries = build_entries(listed?);
        for entry in &entries {
            if let Some(target) = target_from_id(&entry.id) {
                self.icons.register(&entry.icon, target);
            }
        }
        let count = entries.len();
        if let Ok(mut apps) = self.apps.write() {
            *apps = Arc::new(entries);
        }
        if let Ok(mut t) = self.indexed_at.lock() {
            *t = Some(Instant::now());
        }
        tracing::info!("indexed {count} apps in {:?}", started.elapsed());
        Ok(count)
    }

    /// Refreshes in the background if the index is older than `max_age`.
    /// Catches Store installs, which don't touch the Start Menu folders.
    pub fn refresh_if_stale(self: &Arc<Self>, max_age: Duration) {
        let stale = self
            .indexed_at
            .lock()
            .ok()
            .and_then(|t| *t)
            .is_none_or(|t| t.elapsed() > max_age);
        if stale {
            self.refresh_in_background();
        }
    }

    pub fn refresh_in_background(self: &Arc<Self>) {
        let index = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            if let Err(e) = index.refresh().await {
                tracing::warn!("could not index apps: {e}");
            }
        });
    }
}

/// Filters out non-apps (uninstallers, readmes, web links), drops
/// duplicates, and sorts by name.
pub fn build_entries(raw: Vec<RawApp>) -> Vec<AppEntry> {
    let mut seen = HashSet::new();
    let mut entries: Vec<AppEntry> = raw
        .into_iter()
        .filter(is_app)
        .filter(|app| seen.insert(app.target.clone()))
        .map(|app| AppEntry {
            id: format!("{ID_PREFIX}{}", app.target),
            keywords: keywords_for(&app.target, &app.name),
            icon: icons::key_for(&app.target),
            // Store apps are identified by "<package>!<app>".
            elevatable: !app.target.contains('!'),
            name: app.name,
        })
        .collect();
    entries.sort_by_key(|e| e.name.to_lowercase());
    entries
}

fn is_app(app: &RawApp) -> bool {
    const JUNK: [&str; 8] = [
        "uninstall",
        "readme",
        "read me",
        "release notes",
        "license",
        "documentation",
        "help file",
        "website",
    ];
    let name = app.name.to_lowercase();
    let target = app.target.to_lowercase();
    let is_link = target.starts_with("http://") || target.starts_with("https://");
    let is_document = [".txt", ".pdf", ".chm", ".htm", ".html", ".url", ".rtf"]
        .iter()
        .any(|ext| target.ends_with(ext));
    !name.trim().is_empty() && !is_link && !is_document && !JUNK.iter().any(|j| name.contains(j))
}

/// The program's file name, when it differs from the display name, so
/// "code" finds Visual Studio Code (Code.exe).
fn keywords_for(target: &str, name: &str) -> Vec<String> {
    let file = target.rsplit(['\\', '/']).next().unwrap_or(target);
    let stem = file
        .strip_suffix(".exe")
        .or_else(|| file.strip_suffix(".EXE"));
    match stem {
        Some(stem) if !stem.is_empty() && !name.to_lowercase().contains(&stem.to_lowercase()) => {
            vec![stem.to_owned()]
        }
        _ => Vec::new(),
    }
}

pub struct AppProvider {
    index: Arc<AppIndex>,
}

impl AppProvider {
    pub fn new(index: Arc<AppIndex>) -> Self {
        Self { index }
    }
}

#[async_trait]
impl SearchProvider for AppProvider {
    fn name(&self) -> &'static str {
        "apps"
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let apps = self.index.snapshot();
        Ok(match_apps(&apps, query, limit))
    }
}

fn match_apps(apps: &[AppEntry], query: &str, limit: usize) -> Vec<ResultItem> {
    let mut fuzzy = FuzzyQuery::new(query);
    let mut items: Vec<ResultItem> = apps
        .iter()
        .filter_map(|app| {
            let title = fuzzy.score(&app.name);
            let keyword = app
                .keywords
                .iter()
                .filter_map(|k| fuzzy.score(k))
                .map(|s| s * KEYWORD_WEIGHT)
                .reduce(f32::max);
            let score = title.into_iter().chain(keyword).reduce(f32::max)?;
            let mut actions = vec![Action::Open];
            if app.elevatable {
                actions.push(Action::RunAsAdmin);
            }
            Some(ResultItem {
                id: app.id.clone(),
                kind: ItemKind::App,
                title: app.name.clone(),
                subtitle: "Application".into(),
                icon: Some(app.icon.clone()),
                score,
                actions,
            })
        })
        .collect();
    items.sort_by(|a, b| b.score.total_cmp(&a.score));
    items.truncate(limit);
    items
}

/// Re-indexes when shortcuts are added or removed in either Start Menu
/// folder. Keep the returned watcher alive; dropping it stops watching.
pub fn watch_start_menu(index: Arc<AppIndex>) -> Option<notify::RecommendedWatcher> {
    use notify::{RecursiveMode, Watcher};

    let dirs = start_menu_dirs();
    if dirs.is_empty() {
        return None;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = tx.send(());
        }
    })
    .map_err(|e| tracing::warn!("could not watch the Start Menu: {e}"))
    .ok()?;
    for dir in &dirs {
        if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
            tracing::warn!("could not watch {}: {e}", dir.display());
        }
    }

    tauri::async_runtime::spawn(async move {
        while rx.recv().await.is_some() {
            // Installers touch many files; wait for a quiet moment.
            loop {
                match tokio::time::timeout(WATCH_DEBOUNCE, rx.recv()).await {
                    Ok(Some(())) => continue,
                    Ok(None) => return,
                    Err(_) => break,
                }
            }
            tracing::debug!("start menu changed; re-indexing");
            if let Err(e) = index.refresh().await {
                tracing::warn!("could not re-index apps: {e}");
            }
        }
    });
    Some(watcher)
}

/// The per-user and all-users Start Menu program folders that exist.
fn start_menu_dirs() -> Vec<PathBuf> {
    let programs = |root: PathBuf| root.join(r"Microsoft\Windows\Start Menu\Programs");
    [
        dirs::data_dir().map(programs),
        std::env::var_os("ProgramData").map(|p| programs(PathBuf::from(p))),
    ]
    .into_iter()
    .flatten()
    .filter(|d| d.is_dir())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str, target: &str) -> RawApp {
        RawApp {
            name: name.into(),
            target: target.into(),
        }
    }

    #[test]
    fn filters_junk_and_duplicates() {
        let entries = build_entries(vec![
            raw(
                "Notepad++",
                r"{6D809377-6AF0-444B-8957-A3773F02200E}\Notepad++\notepad++.exe",
            ),
            raw("Uninstall Notepad++", r"{6D809377}\Notepad++\uninstall.exe"),
            raw("Notepad++ Readme", r"{6D809377}\Notepad++\readme.txt"),
            raw("Project website", "https://example.com"),
            raw(
                "Calculator",
                "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            ),
            raw(
                "Calculator",
                "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            ),
            raw("  ", "blank"),
        ]);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Calculator", "Notepad++"]);
    }

    #[test]
    fn store_apps_cannot_run_as_admin() {
        let entries = build_entries(vec![
            raw(
                "Calculator",
                "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            ),
            raw("Code", r"{6D809377}\Microsoft VS Code\Code.exe"),
        ]);
        assert!(!entries[0].elevatable);
        assert!(entries[1].elevatable);
    }

    #[test]
    fn exe_name_becomes_a_keyword() {
        assert_eq!(
            keywords_for(r"{X}\Microsoft VS Code\Code.exe", "Visual Studio Code"),
            Vec::<String>::new(),
            "already in the name"
        );
        assert_eq!(
            keywords_for(r"{X}\Mozilla Firefox\firefox.exe", "Firefox"),
            Vec::<String>::new()
        );
        assert_eq!(
            keywords_for(r"{X}\Tools\procexp64.exe", "Process Explorer"),
            ["procexp64"]
        );
        assert!(keywords_for(
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            "Calculator"
        )
        .is_empty());
    }

    #[test]
    fn ids_round_trip() {
        let entries = build_entries(vec![raw("Calculator", "Calc!App")]);
        assert_eq!(target_from_id(&entries[0].id), Some("Calc!App"));
        assert_eq!(target_from_id("cmd:quit"), None);
    }

    #[test]
    fn matches_by_name_and_keyword() {
        let apps = build_entries(vec![
            raw("Visual Studio Code", r"{X}\Microsoft VS Code\Code.exe"),
            raw("Process Explorer", r"{X}\Tools\procexp64.exe"),
            raw("Calculator", "Calc!App"),
        ]);
        let titles = |q: &str| -> Vec<String> {
            match_apps(&apps, q, 8)
                .into_iter()
                .map(|i| i.title)
                .collect()
        };
        assert_eq!(titles("vsc"), ["Visual Studio Code"]);
        assert_eq!(titles("procexp"), ["Process Explorer"]);
        assert!(titles("zzzz").is_empty());
    }

    #[test]
    fn results_carry_icon_and_actions() {
        let apps = build_entries(vec![raw("Calculator", "Calc!App")]);
        let item = &match_apps(&apps, "calc", 8)[0];
        assert_eq!(
            item.icon.as_deref(),
            Some(icons::key_for("Calc!App").as_str())
        );
        assert_eq!(item.actions, [Action::Open]);
    }

    /// Real Windows: the runner's Start menu should list something.
    #[cfg(windows)]
    #[tokio::test]
    async fn lists_real_apps_on_windows() {
        let shell = Shell::new();
        let raw = shell.list_apps().await.expect("AppsFolder enumeration");
        let entries = build_entries(raw);
        eprintln!(
            "found {} apps, e.g. {:?}",
            entries.len(),
            entries.iter().take(5).map(|e| &e.name).collect::<Vec<_>>()
        );
        assert!(!entries.is_empty());
    }

    /// Real Windows: icon extraction returns a full RGBA image.
    #[cfg(windows)]
    #[tokio::test]
    async fn extracts_a_real_icon_on_windows() {
        let shell = Shell::new();
        let entries = build_entries(shell.list_apps().await.unwrap());
        let mut extracted = 0;
        for entry in entries.iter().take(10) {
            let target = target_from_id(&entry.id).unwrap().to_owned();
            if let Ok(image) = shell.app_icon(target, crate::icons::ICON_SIZE).await {
                assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
                crate::icons::encode_png(&image).unwrap();
                extracted += 1;
            }
        }
        eprintln!("extracted {extracted} of the first 10 icons");
        assert!(extracted > 0);
    }
}
