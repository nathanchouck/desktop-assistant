//! Everything the feature shares: the account, the API client, the
//! calendar and mail cache, Drive's per-query cache, and the app handle
//! for status events. One per app (`super::shared`).

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use tauri::async_runtime::JoinHandle;
use tauri::AppHandle;
use tauri_specta::Event;

use super::account::{Account, GoogleStatus};
use super::api::Api;
use super::commands::GoogleStatusChanged;
use super::drive_search::DriveSearch;
use super::sync::Cache;
use super::GoogleSettings;
use crate::settings::SharedSettings;

pub struct Google {
    pub settings: SharedSettings,
    pub account: Arc<Account>,
    pub api: Api,
    pub cache: Cache,
    pub drive: DriveSearch,
    app: OnceLock<AppHandle>,
    /// A sign-in waiting in the browser; a new one replaces it.
    connecting: Mutex<Option<JoinHandle<()>>>,
}

impl Google {
    pub fn new(data_dir: &Path, settings: SharedSettings) -> Self {
        let account = Arc::new(Account::load(data_dir, Arc::clone(&settings)));
        Self {
            api: Api::new(Arc::clone(&account)),
            account,
            settings,
            cache: Cache::default(),
            drive: DriveSearch::default(),
            app: OnceLock::new(),
            connecting: Mutex::new(None),
        }
    }

    /// This feature's settings, without copying the rest.
    pub fn settings(&self) -> GoogleSettings {
        match self.settings.read() {
            Ok(s) => s.google.clone(),
            Err(poisoned) => poisoned.into_inner().google.clone(),
        }
    }

    /// Turned on and connected: the only time anything goes to Google.
    pub fn active(&self) -> bool {
        self.settings().enabled && self.account.connected()
    }

    pub fn status(&self) -> GoogleStatus {
        self.account.status()
    }

    pub fn set_app(&self, app: &AppHandle) {
        let _ = self.app.set(app.clone());
    }

    /// Tells Settings the status changed.
    pub fn announce(&self) {
        if let Some(app) = self.app.get() {
            let event = GoogleStatusChanged {
                status: self.status(),
            };
            if let Err(e) = event.emit(app) {
                tracing::debug!("could not send the Google status: {e}");
            }
        }
    }

    /// Keeps `task` as the sign-in in progress, ending any earlier one.
    pub fn replace_connecting(&self, task: Option<JoinHandle<()>>) {
        let mut slot = self.connecting.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(old) = slot.take() {
            old.abort();
        }
        *slot = task;
    }

    /// Forgets everything read from Google (after disconnecting).
    pub fn clear(&self) {
        self.cache.clear();
        self.drive.clear();
    }
}
