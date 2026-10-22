//! Settings while the app runs: saves from the menu, and changes made to
//! the files elsewhere (another PC through the sync, a hand edit), which
//! are applied the same way a change in the menu is.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use tokio::sync::mpsc;

use super::disk::Files;
use super::files::{Change, SettingsFiles};
use super::{Settings, SettingsError};
use crate::hotkey;
use crate::state::AppState;
use crate::toast;

/// Sync clients and editors write in steps (temp file, rename): wait for
/// quiet before reading.
const QUIET: Duration = Duration::from_millis(500);
/// The files are looked at this often anyway: for a chosen folder that
/// comes back, a cloud file that becomes readable, or a change the watcher
/// missed. Two small reads.
const EVERY: Duration = Duration::from_secs(30);

/// The settings changed outside the menu and the new ones are in use.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct SettingsChanged {
    pub settings: Settings,
}

/// Saves settings from the menu and puts them in use. The caller changes
/// the shortcut first, since that can fail.
pub fn save(app: &AppHandle, old: &Settings, new: Settings) -> Result<(), SettingsError> {
    app.state::<SettingsFiles>().save(&new)?;
    use_settings(app, old, new);
    Ok(())
}

/// Makes `new` the settings in use and applies what takes more than
/// reading the new value: the Windows key hook, starting at login, and a
/// fresh check of an AI whose location or billing changed. Everything else
/// is read fresh where it's used.
pub fn use_settings(app: &AppHandle, old: &Settings, new: Settings) {
    let win_key = new.hotkeys.win_key_override;
    let login = new.general.launch_at_login;
    let claude_changed = new.ai.claude.path != old.ai.claude.path
        || new.ai.claude.billing != old.ai.claude.billing;
    let codex_changed = new.ai.chatgpt.path != old.ai.chatgpt.path
        || new.ai.chatgpt.billing != old.ai.chatgpt.billing
        || new.ai.default_provider != old.ai.default_provider;
    {
        let state = app.state::<AppState>();
        let mut current = state
            .settings
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current = new;
    }
    if win_key != old.hotkeys.win_key_override {
        hotkey::set_win_key(app, win_key);
    }
    if login != old.general.launch_at_login {
        crate::sync_launch_at_login(app, login);
    }
    if claude_changed {
        crate::check_claude_status(app, true);
    }
    if codex_changed {
        crate::check_codex_status(app, true);
    }
}

/// Settings read from the files after a change made elsewhere.
fn reloaded(app: &AppHandle, mut new: Settings) {
    let old = app.state::<AppState>().settings();
    if new == old {
        return;
    }
    // Like the menu: a shortcut that can't be registered on this PC
    // (another app has it) leaves the current one.
    if new.hotkeys.toggle != old.hotkeys.toggle {
        if let Err(e) = hotkey::change_toggle(app, &old.hotkeys.toggle, &new.hotkeys.toggle) {
            toast::error(
                app,
                format!(
                    "Settings changed, but {} can't be used here, so {} stays: {e}",
                    new.hotkeys.toggle, old.hotkeys.toggle
                ),
            );
            new.hotkeys.toggle = old.hotkeys.toggle.clone();
        }
    }
    tracing::info!("settings changed on disk; using them");
    use_settings(app, &old, new.clone());
    if let Err(e) = (SettingsChanged { settings: new }).emit(app) {
        tracing::debug!("could not tell the UI about new settings: {e}");
    }
}

/// Settings adopted from another folder in the menu: applied like a change
/// made elsewhere.
pub fn adopted(app: &AppHandle, settings: Settings) {
    reloaded(app, settings);
}

/// Follows the settings files for the life of the app.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { follow(app).await });
}

async fn follow(app: AppHandle) {
    let (tx, mut changes) = mpsc::unbounded_channel::<()>();
    // What the watchers were made for, and the watchers.
    let mut watching: Option<(Files, Vec<RecommendedWatcher>)> = None;
    loop {
        // The folder changes when you move settings, or when a chosen
        // folder goes away or comes back.
        let files = app.state::<SettingsFiles>().files();
        if watching.as_ref().map(|(f, _)| f) != Some(&files) {
            let watchers = watch(&files, &tx);
            watching = Some((files, watchers));
        }
        tokio::select! {
            Some(()) = changes.recv() => {
                tokio::time::sleep(QUIET).await;
                while changes.try_recv().is_ok() {}
            }
            _ = tokio::time::sleep(EVERY) => {}
        }
        check(&app).await;
    }
}

/// Reads the files (off the async threads: a cloud file may be fetched)
/// and uses what changed.
async fn check(app: &AppHandle) {
    let handle = app.clone();
    let change = tauri::async_runtime::spawn_blocking(move || {
        let current = handle.state::<AppState>().settings();
        handle.state::<SettingsFiles>().check(&current)
    })
    .await;
    match change {
        Ok(Change::None) => {}
        Ok(Change::Reloaded(settings)) => reloaded(app, settings),
        Ok(Change::Broken(message)) => {
            toast::error(app, format!("Settings weren't reloaded: {message}"));
        }
        Err(e) => tracing::warn!("could not check the settings files: {e}"),
    }
}

/// Watches the folders of both files (not their subfolders). A folder that
/// can't be watched is still looked at every `EVERY`.
fn watch(files: &Files, tx: &mpsc::UnboundedSender<()>) -> Vec<RecommendedWatcher> {
    let names: Vec<OsString> = [&files.synced, &files.local]
        .iter()
        .filter_map(|p| p.file_name().map(OsString::from))
        .collect();
    let mut folders: Vec<&Path> = [files.synced.parent(), files.local.parent()]
        .into_iter()
        .flatten()
        .collect();
    folders.dedup();
    folders
        .into_iter()
        .filter_map(|folder| {
            let tx = tx.clone();
            let names = names.clone();
            let mut watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if event.is_ok_and(|e| touches(&e.paths, &names)) {
                        let _ = tx.send(());
                    }
                })
                .map_err(|e| tracing::warn!("can't watch the settings files: {e}"))
                .ok()?;
            match watcher.watch(folder, RecursiveMode::NonRecursive) {
                Ok(()) => Some(watcher),
                Err(e) => {
                    tracing::debug!("not watching {}: {e}", folder.display());
                    None
                }
            }
        })
        .collect()
}

/// Whether an event is about a settings file. The local data folder also
/// holds the database, the file index and logs, which change all the time.
fn touches(paths: &[PathBuf], names: &[OsString]) -> bool {
    paths
        .iter()
        .filter_map(|p| p.file_name())
        .any(|name| names.iter().any(|wanted| wanted == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_settings_files_count() {
        let names = [OsString::from("settings.toml"), OsString::from("settings.local.toml")];
        let data = Path::new("/data/desktop-assistant");
        assert!(touches(&[data.join("settings.local.toml")], &names));
        // A save renames its temp file over the real one.
        assert!(touches(
            &[data.join("settings.local.toml.tmp"), data.join("settings.local.toml")],
            &names
        ));
        assert!(!touches(&[data.join("data.db-wal")], &names));
        assert!(!touches(&[data.join("settings.toml.tmp")], &names));
        assert!(!touches(&[], &names));
    }
}
