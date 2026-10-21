use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::mpsc;

use crate::ai::chat::Chat;
use crate::ai::claude::StatusCache;
use crate::ai::AiEvent;
use crate::desktop::{DesktopContext, DesktopTracker};
use crate::file_index::FileIndex;
use crate::icons::IconCache;
use crate::paths::{self, AppPaths};
use crate::platform::Shell;
use crate::search::apps::{AppIndex, AppProvider};
use crate::search::files::FileProvider;
use crate::search::{commands::CommandProvider, Aggregator};
use crate::settings::Settings;
use crate::store::Store;
use crate::window::LauncherState;

/// Shared app state, managed by Tauri and available to every command.
pub struct AppState {
    pub paths: AppPaths,
    pub settings: RwLock<Settings>,
    pub search: Aggregator,
    pub launcher: LauncherState,
    pub store: Arc<Store>,
    pub shell: Arc<Shell>,
    pub icons: Arc<IconCache>,
    pub apps: Arc<AppIndex>,
    /// Built-in file index, used when Everything isn't running.
    pub files: Arc<FileIndex>,
    /// Keeps the Start Menu watcher alive.
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub chat: Arc<Chat>,
    /// Whether Claude Code is installed and signed in, checked in the
    /// background so searching never waits.
    pub claude_status: Arc<StatusCache>,
    /// What was on screen when the launcher last opened.
    pub desktop: Arc<DesktopTracker>,
    /// The desktop context last sent in the current conversation, so a
    /// follow-up only repeats it when it changed.
    pub sent_context: Mutex<Option<DesktopContext>>,
    /// Chat events, taken once at startup and forwarded to the UI.
    pub chat_events: Mutex<Option<mpsc::UnboundedReceiver<(u32, AiEvent)>>>,
}

impl AppState {
    pub fn new(paths: AppPaths, settings: Settings) -> Self {
        let store = Store::open(&paths.database_file()).unwrap_or_else(|e| {
            tracing::error!("{e}; history won't be saved this session");
            Store::in_memory()
        });
        let shell = Arc::new(Shell::new());
        let icons = Arc::new(IconCache::new(paths.icon_cache_dir(), Arc::clone(&shell)));
        let apps = Arc::new(AppIndex::new(Arc::clone(&shell), Arc::clone(&icons)));
        let index_roots = paths::resolve_index_folders(
            &settings.search.index_folders,
            dirs::home_dir().as_deref(),
        );
        let files = Arc::new(FileIndex::new(
            &index_roots,
            &settings.search.exclusions,
            paths.file_index_file(),
        ));
        let search = Aggregator::new(vec![
            Arc::new(CommandProvider),
            Arc::new(AppProvider::new(Arc::clone(&apps))),
            Arc::new(FileProvider::new(
                Arc::clone(&shell),
                Arc::clone(&icons),
                Arc::clone(&files),
                settings.search.exclusions.clone(),
            )),
        ]);
        let (chat_tx, chat_rx) = mpsc::unbounded_channel();
        Self {
            chat: Arc::new(Chat::new(chat_tx)),
            claude_status: Arc::new(StatusCache::new()),
            chat_events: Mutex::new(Some(chat_rx)),
            desktop: Arc::new(DesktopTracker::new()),
            sent_context: Mutex::new(None),
            paths,
            settings: RwLock::new(settings),
            search,
            launcher: LauncherState::default(),
            store: Arc::new(store),
            shell,
            icons,
            apps,
            files,
            watcher: Mutex::new(None),
        }
    }

    /// Snapshot of the current settings. A poisoned lock (a panic while
    /// writing) falls back to the last value rather than failing the call.
    pub fn settings(&self) -> Settings {
        match self.settings.read() {
            Ok(s) => s.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}
