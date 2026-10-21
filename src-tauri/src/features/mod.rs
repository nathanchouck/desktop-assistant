//! Features that plug into search. Each is a folder here with:
//! - `provider(ctx)`: its search results and what happens when you pick
//!   one (`SearchProvider`, incl. `claims`/`owns`/`activate`), if any;
//! - `start(app)`: background work once the app is running;
//! - its settings struct, which `settings::Settings` includes;
//! - Windows-only code in its own `#[cfg(windows)] mod win;`.
//!
//! So a feature lives in its own folder and only touches shared files to
//! register commands or events (see docs/CONVENTIONS.md).

pub mod ai_commands;
pub mod calc;
pub mod clipboard;
pub mod color;
pub mod emoji;
pub mod google;
pub mod history;
pub mod media;
pub mod processes;
pub mod quicklinks;
pub mod scripts;
pub mod snippets;
pub mod system;
pub mod updater;
pub mod window_layout;

use std::sync::Arc;

use tauri::AppHandle;

use crate::desktop::DesktopTracker;
use crate::icons::IconCache;
use crate::paths::AppPaths;
use crate::platform::Shell;
use crate::search::SearchProvider;
use crate::settings::SharedSettings;
use crate::store::Store;

/// What a feature's search provider can use. Settings are shared, so a
/// provider always reads the current ones.
#[derive(Clone)]
pub struct ProviderContext {
    pub shell: Arc<Shell>,
    pub icons: Arc<IconCache>,
    pub store: Arc<Store>,
    pub settings: SharedSettings,
    /// What was on screen when the launcher opened (the window you were in).
    pub desktop: Arc<DesktopTracker>,
    pub paths: AppPaths,
}

/// Every feature's search provider.
pub fn providers(ctx: &ProviderContext) -> Vec<Arc<dyn SearchProvider>> {
    [
        calc::provider(ctx),
        quicklinks::provider(ctx),
        emoji::provider(ctx),
        clipboard::provider(ctx),
        snippets::provider(ctx),
        scripts::provider(ctx),
        updater::provider(ctx),
        system::provider(ctx),
        window_layout::provider(ctx),
        processes::provider(ctx),
        media::provider(ctx),
        color::provider(ctx),
        history::provider(ctx),
        ai_commands::provider(ctx),
        google::provider(ctx),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Starts every feature's background work.
pub fn start(app: &AppHandle) {
    calc::start(app);
    quicklinks::start(app);
    emoji::start(app);
    clipboard::start(app);
    snippets::start(app);
    scripts::start(app);
    updater::start(app);
    system::start(app);
    window_layout::start(app);
    processes::start(app);
    media::start(app);
    color::start(app);
    history::start(app);
    ai_commands::start(app);
    google::start(app);
}
