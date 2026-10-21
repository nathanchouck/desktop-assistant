//! Runs the updater: checks GitHub, downloads in the background, tells the
//! UI how it's going, and installs when asked.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, EventId, Listener, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use tauri_specta::Event;

use super::errors::describe;
use super::saved::Saved;
use super::schedule::{unix_now, Attempt};
use super::tracker::{CheckResult, Tracker, UpdatePhase};
use super::version::{after_install, AfterInstall};
use crate::ipc::LauncherShown;
use crate::state::AppState;
use crate::{toast, window};

/// A check is one small JSON file; don't wait on it for long.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// The whole installer download (a few MB).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Where updating stands, for the settings menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct UpdateStatus {
    /// The running version ("0.1.0").
    pub current_version: String,
    pub phase: UpdatePhase,
    /// The phase in a few words ("Downloading v0.2.0… 40%").
    pub summary: String,
    /// When a check last got an answer on this PC (seconds since 1970).
    pub last_checked: Option<u32>,
    /// Goes up with every change, so the UI can tell an answer that
    /// arrives late (a command's reply after newer events) and ignore it.
    pub revision: u32,
}

/// Sent whenever `UpdateStatus` changes (a few times a second at most
/// while downloading).
#[derive(Debug, Clone, Serialize, specta::Type, tauri_specta::Event)]
pub struct UpdateStatusChanged {
    pub status: UpdateStatus,
}

/// A downloaded, verified installer waiting for "install and restart".
struct Downloaded {
    update: Update,
    bytes: Vec<u8>,
}

/// Where updating stands and the download it calls ready, under one
/// lock so the bytes installed are always the version shown.
struct State {
    tracker: Tracker,
    downloaded: Option<Arc<Downloaded>>,
}

pub struct Updates {
    app: OnceLock<AppHandle>,
    /// `updates.json` in the data folder.
    file: PathBuf,
    state: Mutex<State>,
    saved: Mutex<Saved>,
    last_attempt: Mutex<Option<Attempt>>,
    /// The toast waiting for the launcher to open, so a newer one replaces it.
    announcement: Mutex<Option<EventId>>,
    /// Counts status changes (`UpdateStatus::revision`).
    revision: AtomicU32,
}

/// A poisoned lock (a panic elsewhere) still holds usable state.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Updates {
    pub fn new(file: PathBuf) -> Self {
        let saved = Saved::load(&file);
        Self {
            app: OnceLock::new(),
            file,
            state: Mutex::new(State {
                tracker: Tracker::new(),
                downloaded: None,
            }),
            saved: Mutex::new(saved),
            last_attempt: Mutex::new(None),
            announcement: Mutex::new(None),
            revision: AtomicU32::new(0),
        }
    }

    /// Connects to the running app; `set_up` says whether this build can
    /// update. Also reports how an install started last time went.
    pub fn attach(&self, app: &AppHandle, set_up: bool) {
        let _ = self.app.set(app.clone());
        if set_up {
            lock(&self.state).tracker.enable();
        }
        let installing = lock(&self.saved).installing.take();
        if installing.is_some() {
            self.save();
        }
        match after_install(installing.as_deref(), &self.current_version()) {
            Some(AfterInstall::Updated(version)) => {
                tracing::info!("updated to {version}");
                self.announce(format!("Updated to v{version}"), false);
            }
            Some(AfterInstall::NotInstalled(version)) => self.announce(
                format!(
                    "The update to v{version} didn't finish installing; it will download again."
                ),
                true,
            ),
            None => {}
        }
    }

    /// The version the updater compares releases with (tauri.conf.json's).
    pub fn current_version(&self) -> String {
        self.app.get().map_or_else(
            || env!("CARGO_PKG_VERSION").to_owned(),
            |app| app.package_info().version.to_string(),
        )
    }

    pub fn status(&self) -> UpdateStatus {
        // Read before the state: a change counts up after it's made, so
        // this state is at least as new as its revision says.
        let revision = self.revision.load(Ordering::SeqCst);
        let phase = lock(&self.state).tracker.phase().clone();
        UpdateStatus {
            current_version: self.current_version(),
            summary: phase.summary(),
            phase,
            last_checked: lock(&self.saved)
                .last_checked
                .and_then(|t| u32::try_from(t).ok()),
            revision,
        }
    }

    /// The downloaded version waiting to be installed, if any.
    pub fn ready(&self) -> Option<String> {
        lock(&self.state).tracker.ready().map(str::to_owned)
    }

    pub fn last_attempt(&self) -> Option<Attempt> {
        *lock(&self.last_attempt)
    }

    /// Looks for a newer release. When there is one, its download starts
    /// in the background; this returns once the check itself is done.
    pub async fn check(self: &Arc<Self>) -> CheckResult {
        let Some(app) = self.app.get() else {
            return CheckResult::NotSetUp;
        };
        {
            let tracker = &mut lock(&self.state).tracker;
            if *tracker.phase() == UpdatePhase::NotSetUp {
                return CheckResult::NotSetUp;
            }
            if !tracker.start_check() {
                return CheckResult::Busy(tracker.phase().clone());
            }
        }
        self.emit();
        let result = match app.updater_builder().timeout(CHECK_TIMEOUT).build() {
            Ok(updater) => updater.check().await,
            Err(e) => Err(e),
        };
        let now = unix_now();
        *lock(&self.last_attempt) = Some(Attempt {
            at: now,
            failed: result.is_err(),
        });
        match result {
            Ok(None) => {
                tracing::info!("no update available");
                {
                    let mut state = lock(&self.state);
                    state.tracker.up_to_date();
                    if *state.tracker.phase() == UpdatePhase::UpToDate {
                        state.downloaded = None;
                    }
                }
                self.remember_check(now);
            }
            Ok(Some(update)) => {
                tracing::info!("update {} available", update.version);
                let download = lock(&self.state).tracker.found(&update.version);
                self.remember_check(now);
                if download {
                    tauri::async_runtime::spawn(Arc::clone(self).download(update));
                }
            }
            Err(e) => {
                tracing::warn!("update check failed: {e}");
                lock(&self.state).tracker.check_failed(&describe(&e));
            }
        }
        self.emit();
        CheckResult::Done(lock(&self.state).tracker.phase().clone())
    }

    async fn download(self: Arc<Self>, mut update: Update) {
        update.timeout = Some(DOWNLOAD_TIMEOUT);
        let version = update.version.clone();
        let progress = Arc::clone(&self);
        let mut received: u64 = 0;
        let result = update
            .download(
                move |chunk, total| {
                    received += chunk as u64;
                    let changed = lock(&progress.state).tracker.progress(received, total);
                    if changed {
                        progress.emit();
                    }
                },
                || {},
            )
            .await;
        match result {
            Ok(bytes) => {
                tracing::info!("update {version} downloaded and verified");
                let kept = {
                    let mut state = lock(&self.state);
                    let kept = state.tracker.downloaded();
                    if kept {
                        state.downloaded = Some(Arc::new(Downloaded { update, bytes }));
                    }
                    kept
                };
                // Overtaken (an install of an older download started).
                if !kept {
                    return;
                }
                self.emit();
                self.announce(
                    format!("Update ready: v{version} — restart to install"),
                    false,
                );
            }
            Err(e) => {
                tracing::warn!("could not download update {version}: {e}");
                // Try again after the shorter wait, like a failed check.
                *lock(&self.last_attempt) = Some(Attempt {
                    at: unix_now(),
                    failed: true,
                });
                lock(&self.state).tracker.download_failed(&describe(&e));
                self.emit();
            }
        }
    }

    /// Installs the downloaded update: closes Claude Code or Codex, starts
    /// the installer and quits; the installer opens the new version. Only
    /// returns if that didn't happen.
    pub async fn install(self: &Arc<Self>) -> Result<(), String> {
        let Some(app) = self.app.get() else {
            return Err("the app is still starting".into());
        };
        let (version, downloaded) = {
            let mut state = lock(&self.state);
            let Some(downloaded) = state.downloaded.clone() else {
                return Err("no update has been downloaded".into());
            };
            let Some(version) = state.tracker.start_install() else {
                return Err("an install is already starting".into());
            };
            (version, downloaded)
        };
        self.emit();
        lock(&self.saved).installing = Some(version.clone());
        self.save();
        if let Some(state) = app.try_state::<AppState>() {
            state.chat.shut_down().await;
        }
        tracing::info!("installing update {version}");
        let installed = tauri::async_runtime::spawn_blocking(move || {
            downloaded.update.install(&downloaded.bytes)
        })
        .await;
        let error = match installed {
            // Windows never gets here: the app has quit for the installer.
            // Elsewhere the new version is in place and needs a restart.
            Ok(Ok(())) => app.restart(),
            Ok(Err(e)) => describe(&e),
            Err(e) => e.to_string(),
        };
        tracing::warn!("could not install update {version}: {error}");
        lock(&self.saved).installing = None;
        self.save();
        lock(&self.state).tracker.install_failed();
        self.emit();
        Err(error)
    }

    /// `install` without waiting; a failure shows in the footer.
    pub fn install_in_background(self: &Arc<Self>) {
        let updates = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            if let Err(e) = updates.install().await {
                if let Some(app) = updates.app.get() {
                    toast::error(app, format!("Couldn't install the update: {e}"));
                }
            }
        });
    }

    fn remember_check(&self, at: u64) {
        lock(&self.saved).last_checked = Some(at);
        self.save();
    }

    fn save(&self) {
        let saved = lock(&self.saved).clone();
        if let Err(e) = saved.save(&self.file) {
            tracing::warn!("could not save {}: {e}", self.file.display());
        }
    }

    fn emit(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        let Some(app) = self.app.get() else { return };
        if let Err(e) = (UpdateStatusChanged {
            status: self.status(),
        })
        .emit(app)
        {
            tracing::debug!("could not send the update status: {e}");
        }
    }

    /// Shows `message` in the launcher's footer: now if it's open, else
    /// when it next opens, so it isn't missed while the launcher is hidden.
    fn announce(&self, message: String, error: bool) {
        let Some(app) = self.app.get() else { return };
        let mut pending = lock(&self.announcement);
        if let Some(id) = pending.take() {
            app.unlisten(id);
        }
        let show = move |app: &AppHandle| {
            if error {
                toast::error(app, message);
            } else {
                toast::info(app, message);
            }
        };
        let visible = window::launcher(app).is_some_and(|w| w.is_visible().unwrap_or(false));
        if visible {
            show(app);
        } else {
            let handle = app.clone();
            *pending = Some(LauncherShown::once_any(app, move |_| show(&handle)));
        }
    }
}
