//! Copying what's selected in another app: press Ctrl+C in the window in
//! front, wait for the clipboard to change, read it, and put back what was
//! there before. Used by the "ask about the selection" shortcut and by
//! saved commands with `{selection}`.

use std::path::Path;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use super::attachments::{self, Attachment};
use super::images;
use crate::features::clipboard::content::Captured;
use crate::features::clipboard::system;
use crate::platform::{vk, Shell};
use crate::state::AppState;
use crate::window;

/// How long the app gets to put its copy on the clipboard.
const COPY_WAIT: Duration = Duration::from_millis(400);
const POLL: Duration = Duration::from_millis(10);
/// Some apps change the clipboard in steps (empty it, then fill it): a
/// moment more before reading.
const SETTLE: Duration = Duration::from_millis(30);
/// Between bringing your window back and pressing Ctrl+C in it.
const FOCUS_DELAY: Duration = Duration::from_millis(80);

/// Waits until `sequence()` differs from `before`, for up to `timeout`.
/// Returns whether it changed.
pub fn wait_for_change(
    before: u32,
    sequence: impl Fn() -> u32,
    timeout: Duration,
    poll: Duration,
) -> bool {
    let started = Instant::now();
    loop {
        if sequence() != before {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(poll);
    }
}

/// Presses Ctrl+C in the window in front and returns what it copied
/// (`None`: nothing was selected, so the clipboard didn't change). With
/// `restore`, what was on the clipboard before goes back right after.
pub async fn grab(shell: &Shell, restore: bool) -> Result<Option<Captured>, String> {
    let (before, sequence) = blocking(move || {
        let before = if restore { system::read() } else { None };
        (before, system::sequence())
    })
    .await?;
    shell
        .press_keys(vec![vk::CONTROL, vk::C])
        .await
        .map_err(|e| format!("couldn't press Ctrl+C: {e}"))?;
    blocking(move || {
        if !wait_for_change(sequence, system::sequence, COPY_WAIT, POLL) {
            return None;
        }
        std::thread::sleep(SETTLE);
        let copied = system::read();
        if let Some(content) = before.and_then(system::to_content) {
            // Private, like snippets: it's already in the history once.
            if let Err(e) = system::write(&content, true) {
                tracing::debug!("could not put the clipboard back: {e}");
            }
        }
        copied
    })
    .await
}

/// The selection in the window the launcher was opened from: the launcher
/// hides and that window comes back to the front first. The caller shows
/// the launcher again.
pub async fn grab_behind_launcher(
    app: &AppHandle,
    restore: bool,
) -> Result<Option<Captured>, String> {
    let state = app.try_state::<AppState>().ok_or("the app isn't ready")?;
    let target = state
        .desktop
        .window()
        .ok_or("there's no window to copy from")?;
    window::hide(app);
    state
        .shell
        .focus_window(target.handle)
        .await
        .map_err(|e| format!("couldn't bring {} back: {e}", target.app))?;
    tokio::time::sleep(FOCUS_DELAY).await;
    grab(&state.shell, restore).await
}

/// A copy as attachments: text, the copied files, or the copied picture
/// (saved for the AI).
pub async fn to_attachments(captured: Captured, data_dir: &Path, thumbs: &Path) -> Vec<Attachment> {
    match captured {
        Captured::Text(text) if !text.trim().is_empty() => vec![Attachment::Text { text }],
        Captured::Text(_) => Vec::new(),
        Captured::Files(list) => attachments::paths(&list, |p| Path::new(p).is_dir()),
        picture => {
            let dir = data_dir.join(images::DIR_NAME);
            let thumbs = thumbs.to_owned();
            let saved = blocking(move || {
                let image = images::from_clipboard(&picture).ok_or("not a picture we can read")?;
                images::sweep(&dir, &thumbs, images::KEEP_FOR);
                images::save(image, &dir, &thumbs)
            })
            .await
            .and_then(|r: Result<Attachment, String>| r);
            match saved {
                Ok(attachment) => vec![attachment],
                Err(e) => {
                    tracing::warn!("could not attach the copied picture: {e}");
                    Vec::new()
                }
            }
        }
    }
}

/// A copy as text for `{selection}`: the text, or the copied files' paths
/// one per line.
pub fn as_text(captured: Captured) -> Option<String> {
    match captured {
        Captured::Text(text) => Some(text),
        Captured::Files(list) => Some(list.join("\n")),
        Captured::Png(_) | Captured::Dib(_) => None,
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("the clipboard task stopped: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn waits_for_the_clipboard_to_change() {
        let calls = Cell::new(0);
        let changed = wait_for_change(
            7,
            || {
                calls.set(calls.get() + 1);
                if calls.get() > 3 {
                    8
                } else {
                    7
                }
            },
            Duration::from_secs(5),
            Duration::ZERO,
        );
        assert!(changed);
        assert_eq!(calls.get(), 4);
        let started = Instant::now();
        assert!(!wait_for_change(
            7,
            || 7,
            Duration::from_millis(30),
            Duration::from_millis(5)
        ));
        assert!(started.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn copies_become_text_for_placeholders() {
        assert_eq!(as_text(Captured::Text("hi".into())), Some("hi".into()));
        assert_eq!(
            as_text(Captured::Files(vec!["C:\\a".into(), "C:\\b".into()])),
            Some("C:\\a\nC:\\b".into())
        );
        assert_eq!(as_text(Captured::Png(vec![1])), None);
    }

    #[tokio::test]
    async fn copies_become_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let thumbs = dir.path().join("thumbnails");
        let text = to_attachments(Captured::Text("teh cat".into()), dir.path(), &thumbs).await;
        assert_eq!(
            text,
            [Attachment::Text {
                text: "teh cat".into()
            }]
        );
        assert!(
            to_attachments(Captured::Text(" \n".into()), dir.path(), &thumbs)
                .await
                .is_empty()
        );
        let folder = dir.path().display().to_string();
        let files =
            to_attachments(Captured::Files(vec![folder.clone()]), dir.path(), &thumbs).await;
        assert_eq!(
            files,
            [Attachment::Path {
                path: folder,
                folder: true
            }]
        );
        let png = crate::icons::encode_png(&crate::platform::Image {
            width: 2,
            height: 2,
            rgba: vec![9; 16],
        })
        .unwrap();
        let picture = to_attachments(Captured::Png(png), dir.path(), &thumbs).await;
        assert!(
            matches!(
                &picture[..],
                [Attachment::Image {
                    width: 2,
                    height: 2,
                    ..
                }]
            ),
            "{picture:?}"
        );
        assert!(
            to_attachments(Captured::Dib(vec![0; 3]), dir.path(), &thumbs)
                .await
                .is_empty()
        );
    }
}
