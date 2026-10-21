//! Keeps the index current: reads the scripts folder at startup, again
//! when anything in it changes, and when the folder setting changes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::library::Library;

/// Settings have no change notification, so they're compared this often
/// (a read lock and a small copy).
const SETTINGS_EVERY: Duration = Duration::from_secs(1);
/// Editors save in several steps (temp file, rename): wait for quiet.
const QUIET: Duration = Duration::from_millis(300);

/// Runs for the life of the app.
pub async fn follow(library: Arc<Library>) {
    let (tx, mut changes) = mpsc::unbounded_channel::<()>();
    // What the watcher and the index were made for: (on, folder).
    let mut current: Option<(bool, PathBuf)> = None;
    let mut watcher: Option<RecommendedWatcher> = None;
    loop {
        let wanted = (library.settings().enabled, library.folder());
        if current.as_ref() != Some(&wanted) {
            watcher = None;
            library.rescan().await;
            if wanted.0 {
                watcher = watch(&wanted.1, tx.clone());
            }
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
            _ = tokio::time::sleep(SETTINGS_EVERY) => {}
        }
    }
}

/// Watches `folder` (not its subfolders); `None` if it can't be watched
/// (it doesn't exist): Reload Scripts or a settings change tries again.
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
        wait_for(
            "first scan",
            Box::new(move |l| l.index().folder == first),
        )
        .await;
        std::fs::write(
            dir.path().join("one").join("a.py"),
            "# @title Ay\n",
        )
        .unwrap();
        wait_for(
            "new file noticed",
            Box::new(|l| {
                let index = l.index();
                index.scripts.len() + index.problems.len() == 1
            }),
        )
        .await;
        shared.write().unwrap().scripts.folder = second.display().to_string();
        wait_for(
            "folder change",
            Box::new(move |l| l.index().folder == second),
        )
        .await;
        task.abort();
    }
}
