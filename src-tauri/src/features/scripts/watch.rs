//! Keeps the index current: reads the scripts folder at startup, again
//! when anything in it changes, when the folder setting changes, and when
//! a chosen folder that didn't exist is made.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::library::Library;

/// A chosen folder that doesn't exist yet is looked for this often.
const MISSING_EVERY: Duration = Duration::from_secs(1);
/// Editors save in several steps (temp file, rename): wait for quiet.
const QUIET: Duration = Duration::from_millis(300);

/// Runs for the life of the app.
pub async fn follow(library: Arc<Library>) {
    let (tx, mut changes) = mpsc::unbounded_channel::<()>();
    // Subscribed before the first look, so no change is missed.
    let mut settings_changed = crate::settings::changes();
    // What the watcher and the index were made for: (on, folder).
    let mut current: Option<(bool, PathBuf)> = None;
    let mut watcher: Option<RecommendedWatcher> = None;
    // The folder wasn't there to watch (a new folder setting); checked
    // each second until it is ("Open folder" makes it).
    let mut missing = false;
    loop {
        let wanted = (library.settings().enabled, library.folder());
        if current.as_ref() != Some(&wanted) || (missing && wanted.1.is_dir()) {
            // Watch before scanning, so a file saved in between isn't missed.
            watcher = if wanted.0 {
                watch(&wanted.1, tx.clone())
            } else {
                None
            };
            library.rescan().await;
            missing = wanted.0 && !wanted.1.is_dir();
            current = Some(wanted);
        }
        tokio::select! {
            Some(()) = changes.recv() => {
                tokio::time::sleep(QUIET).await;
                while changes.try_recv().is_ok() {}
                if watcher.is_some() {
                    library.rescan().await;
                }
            }
            _ = settings_changed.changed() => {}
            _ = tokio::time::sleep(MISSING_EVERY), if missing => {}
        }
    }
}

/// Watches `folder` (not its subfolders); `None` if it can't be watched.
fn watch(folder: &Path, tx: mpsc::UnboundedSender<()>) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = tx.send(());
        }
    })
    .map_err(|e| tracing::warn!("can't watch the scripts folder: {e}"))
    .ok()?;
    match watcher.watch(folder, RecursiveMode::NonRecursive) {
        Ok(()) => Some(watcher),
        Err(e) => {
            tracing::debug!("not watching {}: {e}", folder.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use std::sync::RwLock;

    #[tokio::test]
    async fn picks_up_new_scripts_and_folder_changes() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("one");
        let second = dir.path().join("two");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("b.py"), "# @title Bee\n").unwrap();
        let mut settings = crate::settings::Settings::default();
        settings.scripts.folder = first.display().to_string();
        let shared = Arc::new(RwLock::new(settings));
        let paths = AppPaths::with_roots(&dir.path().join("c"), &dir.path().join("d"));
        let library = Arc::new(Library::new(paths, Arc::clone(&shared)));
        let task = tokio::spawn(follow(Arc::clone(&library)));

        let wait_for = |what: &'static str, check: Box<dyn Fn(&Library) -> bool>| {
            let library = Arc::clone(&library);
            async move {
                for _ in 0..100 {
                    if check(&library) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                panic!("{what} never happened");
            }
        };
        wait_for("first scan", Box::new(move |l| l.index().folder == first)).await;
        std::fs::write(dir.path().join("one").join("a.py"), "# @title Ay\n").unwrap();
        wait_for(
            "new file noticed",
            Box::new(|l| {
                let index = l.index();
                index.scripts.len() + index.problems.len() == 1
            }),
        )
        .await;
        shared.write().unwrap().scripts.folder = second.display().to_string();
        crate::settings::announce_change();
        wait_for(
            "folder change",
            Box::new(move |l| l.index().folder == second),
        )
        .await;

        // A folder that doesn't exist yet is followed once it's made.
        let third = dir.path().join("three");
        shared.write().unwrap().scripts.folder = third.display().to_string();
        crate::settings::announce_change();
        wait_for(
            "missing folder",
            Box::new(|l| l.index().folder_error.is_some()),
        )
        .await;
        std::fs::create_dir_all(&third).unwrap();
        wait_for(
            "folder made",
            Box::new(|l| l.index().folder_error.is_none()),
        )
        .await;
        std::fs::write(third.join("c.py"), "# @title Sea\n").unwrap();
        wait_for(
            "script in the new folder",
            Box::new(|l| {
                let index = l.index();
                index.scripts.len() + index.problems.len() == 1
            }),
        )
        .await;
        task.abort();
    }
}
