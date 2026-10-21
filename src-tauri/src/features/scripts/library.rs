//! The feature's shared state: the scripts folder as last read, which
//! scripts are running, and the app (for toasts). The search provider,
//! the folder watcher and the settings menu share one `Library`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::Serialize;
use tauri::AppHandle;

use super::header::ScriptMode;
use super::index::{self, Index};
use super::language::Interpreters;
use super::{starter, ScriptSettings};
use crate::paths::AppPaths;
use crate::settings::SharedSettings;

/// The scripts folder for the settings menu.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct ScriptsOverview {
    /// The folder scripts are read from.
    pub folder: String,
    /// The folder used when the setting is empty.
    pub default_folder: String,
    /// Why the folder can't be read, if it can't.
    pub folder_error: Option<String>,
    pub scripts: Vec<ScriptInfo>,
    /// Script files that aren't commands, and why.
    pub problems: Vec<ScriptProblem>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct ScriptInfo {
    pub file: String,
    pub title: String,
    pub description: String,
    /// Like a result's `icon` (`glyph:…` or `text:…`).
    pub icon: String,
    pub mode: ScriptMode,
    /// What its text argument is called, if it takes one.
    pub argument: Option<String>,
    /// "PowerShell", "Python", ...
    pub language: String,
    /// Runs in File Explorer's folder rather than its own.
    pub in_explorer_folder: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct ScriptProblem {
    pub file: String,
    pub message: String,
}

pub struct Library {
    paths: AppPaths,
    settings: SharedSettings,
    /// Swapped whole on each scan, so a search only holds the lock long
    /// enough to clone an `Arc`.
    index: RwLock<Arc<Index>>,
    /// Scripts running now, by file name: Enter twice doesn't run one twice.
    running: Mutex<HashSet<String>>,
    app: OnceLock<AppHandle>,
}

impl Library {
    pub fn new(paths: AppPaths, settings: SharedSettings) -> Self {
        Self {
            paths,
            settings,
            index: RwLock::new(Arc::new(Index::default())),
            running: Mutex::new(HashSet::new()),
            app: OnceLock::new(),
        }
    }

    /// Read on every search and every second by the watcher, so only this
    /// section is copied, not all the settings.
    pub fn settings(&self) -> ScriptSettings {
        match self.settings.read() {
            Ok(s) => s.scripts.clone(),
            Err(poisoned) => poisoned.into_inner().scripts.clone(),
        }
    }

    /// `scripts` in the settings folder (`%APPDATA%\desktop-assistant`).
    pub fn default_folder(&self) -> PathBuf {
        self.paths.config_dir.join("scripts")
    }

    pub fn folder(&self) -> PathBuf {
        resolve_folder(
            &self.settings().folder,
            &self.default_folder(),
            dirs::home_dir().as_deref(),
            &|name| std::env::var(name).ok(),
        )
    }

    pub fn index(&self) -> Arc<Index> {
        match self.index.read() {
            Ok(index) => Arc::clone(&index),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    /// Reads the folder again. Blocks on the disk: call it off the async
    /// threads (see `rescan`).
    pub fn rescan_blocking(&self) -> Arc<Index> {
        let settings = self.settings();
        let folder = self.folder();
        let index = if settings.enabled {
            if folder == self.default_folder() && !folder.exists() {
                match starter::create(&folder) {
                    Ok(()) => tracing::info!("made the scripts folder {}", folder.display()),
                    Err(e) => tracing::warn!("could not make {}: {e}", folder.display()),
                }
            }
            index::scan(&folder, &Interpreters::find())
        } else {
            Index {
                folder,
                ..Index::default()
            }
        };
        tracing::debug!(
            "scripts: {} commands, {} other files",
            index.scripts.len(),
            index.problems.len()
        );
        let index = Arc::new(index);
        match self.index.write() {
            Ok(mut current) => *current = Arc::clone(&index),
            Err(poisoned) => *poisoned.into_inner() = Arc::clone(&index),
        }
        index
    }

    pub async fn rescan(self: &Arc<Self>) -> Arc<Index> {
        let library = Arc::clone(self);
        tokio::task::spawn_blocking(move || library.rescan_blocking())
            .await
            .unwrap_or_else(|_| self.index())
    }

    /// The folder, made first (with the README and example) if it doesn't
    /// exist yet, ready to open in File Explorer.
    pub fn ensure_folder(&self) -> Result<PathBuf, String> {
        let folder = self.folder();
        if !folder.exists() {
            starter::create(&folder)
                .map_err(|e| format!("Couldn't make {}: {e}", folder.display()))?;
            tracing::info!("made the scripts folder {}", folder.display());
        }
        Ok(folder)
    }

    pub fn set_app(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }

    /// Marks `file` as running until the returned guard is dropped;
    /// `None` if it's running already.
    pub fn claim(self: &Arc<Self>, file: &str) -> Option<Running> {
        let mut running = self.running.lock().ok()?;
        running.insert(file.to_owned()).then(|| Running {
            library: Arc::clone(self),
            file: file.to_owned(),
        })
    }

    pub fn toast_info(&self, message: String) {
        match self.app.get() {
            Some(app) => crate::toast::info(app, message),
            None => tracing::info!("{message}"),
        }
    }

    pub fn toast_error(&self, message: String) {
        match self.app.get() {
            Some(app) => crate::toast::error(app, message),
            None => tracing::warn!("{message}"),
        }
    }

    /// Where a script's long output is written to be opened.
    pub fn output_file(&self, script_file: &str) -> PathBuf {
        self.paths
            .data_dir
            .join("script-output")
            .join(format!("{script_file}.txt"))
    }

    pub fn overview(&self) -> ScriptsOverview {
        let index = self.index();
        ScriptsOverview {
            folder: self.folder().display().to_string(),
            default_folder: self.default_folder().display().to_string(),
            folder_error: index.folder_error.clone(),
            scripts: index
                .scripts
                .iter()
                .map(|s| ScriptInfo {
                    file: s.file.clone(),
                    title: s.header.title.clone(),
                    description: s.header.description.clone(),
                    icon: super::rows::icon(&s.header),
                    mode: s.header.mode,
                    argument: s.header.argument.clone(),
                    language: s.language.label().to_owned(),
                    in_explorer_folder: s.header.cwd == super::header::WorkDir::Explorer,
                })
                .collect(),
            problems: index
                .problems
                .iter()
                .map(|p| ScriptProblem {
                    file: p.file.clone(),
                    message: p.message.clone(),
                })
                .collect(),
        }
    }
}

/// A script marked as running.
pub struct Running {
    library: Arc<Library>,
    file: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut running) = self.library.running.lock() {
            running.remove(&self.file);
        }
    }
}

/// The `scripts.folder` setting as a folder: empty is `default`, `~` is
/// your user folder, `%NAME%` an environment variable, and quotes from
/// Explorer's "Copy as path" are dropped. A relative path is taken from
/// your user folder.
pub fn resolve_folder(
    setting: &str,
    default: &Path,
    home: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
) -> PathBuf {
    let text = setting.trim().trim_matches('"').trim();
    if text.is_empty() {
        return default.to_path_buf();
    }
    let expanded = expand_env(text, env);
    let path = match (expanded.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches(['\\', '/'])),
        _ => PathBuf::from(&expanded),
    };
    match home {
        Some(home) if path.is_relative() => home.join(path),
        _ => path,
    }
}

/// `%NAME%` → the variable's value. Unknown names stay as typed.
fn expand_env(text: &str, env: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let value = after
            .find('%')
            .filter(|end| *end > 0)
            .and_then(|end| Some((env(&after[..end])?, end)));
        match value {
            Some((value, end)) => {
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        (name == "USERPROFILE").then(|| "/home/me".to_owned())
    }

    #[test]
    fn expands_environment_variables() {
        assert_eq!(expand_env("%USERPROFILE%/x", &env), "/home/me/x");
        assert_eq!(expand_env("%NOPE%/x", &env), "%NOPE%/x");
        assert_eq!(expand_env("100%", &env), "100%");
        assert_eq!(expand_env("%%USERPROFILE%", &env), "%/home/me");
        assert_eq!(expand_env("a%b", &env), "a%b");
    }

    #[cfg(unix)]
    #[test]
    fn resolves_the_folder_setting() {
        let default = Path::new("/cfg/scripts");
        let home = Some(Path::new("/home/me"));
        let resolve = |s: &str| resolve_folder(s, default, home, &env);
        assert_eq!(resolve(""), default);
        assert_eq!(resolve("  "), default);
        assert_eq!(resolve("~/bin"), Path::new("/home/me/bin"));
        assert_eq!(resolve("~"), Path::new("/home/me"));
        assert_eq!(resolve("\"/opt/scripts\""), Path::new("/opt/scripts"));
        assert_eq!(resolve("%USERPROFILE%/s"), Path::new("/home/me/s"));
        assert_eq!(resolve("relative"), Path::new("/home/me/relative"));
    }

    #[cfg(windows)]
    #[test]
    fn resolves_windows_folders() {
        let default = Path::new(r"C:\cfg\scripts");
        let home = Some(Path::new(r"C:\Users\me"));
        let resolve = |s: &str| resolve_folder(s, default, home, &|_| None);
        assert_eq!(resolve(r"D:\Scripts"), Path::new(r"D:\Scripts"));
        assert_eq!(resolve(r"~\Scripts"), Path::new(r"C:\Users\me\Scripts"));
    }

    fn library(dir: &Path) -> Arc<Library> {
        let paths = AppPaths::with_roots(&dir.join("config"), &dir.join("data"));
        Arc::new(Library::new(
            paths,
            Arc::new(RwLock::new(crate::settings::Settings::default())),
        ))
    }

    #[test]
    fn the_default_folder_is_made_with_an_example() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        assert_eq!(
            lib.folder(),
            dir.path().join("config/desktop-assistant/scripts")
        );
        let index = lib.rescan_blocking();
        assert!(lib.folder().join("README.txt").is_file());
        // The example needs PowerShell, which a Linux machine may not have.
        let example = index
            .scripts
            .iter()
            .map(|s| &s.file)
            .chain(index.problems.iter().map(|p| &p.file));
        assert_eq!(example.collect::<Vec<_>>(), ["hello.ps1"]);
        // Deleted, it stays deleted while the folder is there.
        std::fs::remove_file(lib.folder().join("hello.ps1")).unwrap();
        let index = lib.rescan_blocking();
        assert!(index.scripts.is_empty() && index.problems.is_empty());
        assert_eq!(lib.ensure_folder().unwrap(), lib.folder());
        assert!(!lib.folder().join("hello.ps1").exists());
        let overview = lib.overview();
        assert_eq!(overview.folder, overview.default_folder);
    }

    #[test]
    fn a_chosen_folder_is_made_only_when_opened() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let chosen = dir.path().join("mine");
        lib.settings.write().unwrap().scripts.folder = chosen.display().to_string();
        let index = lib.rescan_blocking();
        assert_eq!(
            index.folder_error.as_deref(),
            Some("The folder doesn't exist.")
        );
        assert!(!chosen.exists());
        assert_eq!(lib.ensure_folder().unwrap(), chosen);
        assert!(chosen.join("README.txt").is_file() && chosen.join("hello.ps1").is_file());
    }

    #[test]
    fn nothing_is_read_while_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        lib.settings.write().unwrap().scripts.enabled = false;
        let index = lib.rescan_blocking();
        assert!(index.scripts.is_empty());
        assert!(!lib.folder().exists(), "no folder made while off");
    }

    #[test]
    fn a_script_runs_once_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let first = lib.claim("a.ps1").expect("free");
        assert!(lib.claim("a.ps1").is_none());
        assert!(lib.claim("b.ps1").is_some());
        drop(first);
        assert!(lib.claim("a.ps1").is_some());
    }
}
