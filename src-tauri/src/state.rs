use std::sync::{Arc, RwLock};

use crate::paths::AppPaths;
use crate::search::{commands::CommandProvider, Aggregator};
use crate::settings::Settings;

/// Shared app state, managed by Tauri and available to every command.
pub struct AppState {
    pub paths: AppPaths,
    pub settings: RwLock<Settings>,
    pub search: Aggregator,
}

impl AppState {
    pub fn new(paths: AppPaths, settings: Settings) -> Self {
        Self {
            paths,
            settings: RwLock::new(settings),
            search: Aggregator::new(vec![Arc::new(CommandProvider)]),
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
