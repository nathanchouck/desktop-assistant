//! Desktop context: what was on screen when the launcher opened (the window
//! you were in; for File Explorer or the desktop, its folder and selection),
//! so Claude knows what "this file" and "here" mean.
//!
//! The window is read instantly as the launcher opens. Explorer's folder and
//! selection come from a COM round trip, so they're looked up in the
//! background; asking Claude waits briefly for them.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::platform::{ExplorerView, ForegroundWindow, Shell};

/// Selected paths written out for Claude; the rest are only counted.
const PROMPT_SELECTION: usize = 20;
/// Window titles longer than this are cut in the UI summary.
const SUMMARY_TITLE: usize = 48;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopContext {
    pub window: Option<ForegroundWindow>,
    pub view: Option<ExplorerView>,
    /// The Explorer lookup is still running.
    pub pending: bool,
}

impl DesktopContext {
    fn is_desktop(&self) -> bool {
        self.window
            .as_ref()
            .is_some_and(|w| matches!(w.class.as_str(), "Progman" | "WorkerW"))
    }

    /// The folder File Explorer (or the desktop) showed, if it's on disk.
    pub fn folder(&self) -> Option<&str> {
        self.view.as_ref()?.folder.as_deref()
    }

    /// The block put in front of a message to Claude; `None` if there's
    /// nothing to say.
    pub fn prompt_block(&self) -> Option<String> {
        let window = self.window.as_ref()?;
        let mut lines = vec!["<desktop_context>".to_owned()];
        if self.is_desktop() {
            lines.push("Opened from: the Windows desktop".into());
        } else {
            let app = if window.app.is_empty() {
                String::new()
            } else {
                format!(" ({})", window.app)
            };
            lines.push(format!("Opened from: \"{}\"{app}", window.title));
        }
        if let Some(view) = &self.view {
            if let Some(folder) = &view.folder {
                lines.push(format!("Folder shown: {folder}"));
            }
            if view.selected > 0 {
                lines.push(format!("Selected ({}):", view.selected));
                lines.extend(view.selection.iter().take(PROMPT_SELECTION).cloned());
                let listed = view.selection.len().min(PROMPT_SELECTION);
                if view.selected > listed {
                    lines.push(format!("…and {} more", view.selected - listed));
                }
            }
        }
        lines.push("</desktop_context>".into());
        Some(lines.join("\n"))
    }

    /// One short line for the UI: what Claude was told about the screen.
    pub fn summary(&self) -> Option<String> {
        let window = self.window.as_ref()?;
        let place = match self.folder() {
            _ if self.is_desktop() => "Desktop".to_owned(),
            Some(folder) => folder_name(folder),
            None if !window.title.is_empty() => cut(&window.title, SUMMARY_TITLE),
            None => window.app.clone(),
        };
        if place.is_empty() {
            return None;
        }
        Some(match self.view.as_ref().map_or(0, |v| v.selected) {
            0 => place,
            1 => format!("{place} · 1 selected"),
            n => format!("{place} · {n} selected"),
        })
    }
}

/// `text` with `context` in front, when there is any.
pub fn with_context(text: &str, context: Option<&DesktopContext>) -> String {
    match context.and_then(DesktopContext::prompt_block) {
        Some(block) => format!("{block}\n\n{text}"),
        None => text.to_owned(),
    }
}

/// `C:\Users\me\Downloads` → `Downloads` (either slash, on any OS).
fn folder_name(folder: &str) -> String {
    let trimmed = folder.trim_end_matches(['\\', '/']);
    match trimmed.rsplit(['\\', '/']).next() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => folder.to_owned(),
    }
}

fn cut(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max_chars - 1).collect();
    format!("{kept}…")
}

/// Holds the latest capture.
pub struct DesktopTracker {
    latest: watch::Sender<DesktopContext>,
    /// Bumped per capture, so a slow lookup can't overwrite a newer one.
    generation: AtomicU64,
}

impl Default for DesktopTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopTracker {
    pub fn new() -> Self {
        Self {
            latest: watch::Sender::new(DesktopContext::default()),
            generation: AtomicU64::new(0),
        }
    }

    /// Call as the launcher opens, before it takes focus. Returns at once;
    /// Explorer's folder and selection are filled in shortly after.
    pub fn capture(self: &Arc<Self>, shell: &Arc<Shell>) {
        let Some(window) = shell.foreground_window() else {
            // The launcher itself (or nothing) is in front: keep what we had.
            return;
        };
        let generation = self.begin(window.clone());
        let tracker = Arc::clone(self);
        let shell = Arc::clone(shell);
        tauri::async_runtime::spawn(async move {
            let view = shell.explorer_view(window).await.unwrap_or_else(|e| {
                tracing::debug!("could not read File Explorer's folder: {e}");
                None
            });
            tracker.finish(generation, view);
        });
    }

    /// Records the window; returns the capture's generation.
    fn begin(&self, window: ForegroundWindow) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let pending = crate::platform::is_shell_view(&window.class);
        self.latest.send_replace(DesktopContext {
            window: Some(window),
            view: None,
            pending,
        });
        generation
    }

    /// Adds Explorer's view, unless a newer capture has started.
    fn finish(&self, generation: u64, view: Option<ExplorerView>) {
        if self.generation.load(Ordering::Acquire) != generation {
            return;
        }
        self.latest.send_modify(|context| {
            context.view = view;
            context.pending = false;
        });
    }

    /// The latest capture, waiting up to `wait` for Explorer's details.
    pub async fn current(&self, wait: Duration) -> DesktopContext {
        let mut rx = self.latest.subscribe();
        let ready = tokio::time::timeout(wait, async {
            rx.wait_for(|c| !c.pending).await.map(|c| c.clone())
        })
        .await;
        match ready {
            Ok(Ok(context)) => context,
            _ => self.latest.borrow().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn explorer(title: &str) -> ForegroundWindow {
        ForegroundWindow {
            handle: 1,
            title: title.into(),
            class: "CabinetWClass".into(),
            app: "explorer.exe".into(),
        }
    }

    fn view(folder: &str, selection: &[&str]) -> ExplorerView {
        ExplorerView {
            folder: Some(folder.into()),
            selection: selection.iter().map(|s| s.to_string()).collect(),
            selected: selection.len(),
        }
    }

    #[test]
    fn describes_an_explorer_window() {
        let context = DesktopContext {
            window: Some(explorer("Downloads")),
            view: Some(view(
                r"C:\Users\me\Downloads",
                &[r"C:\Users\me\Downloads\a.pdf"],
            )),
            pending: false,
        };
        assert_eq!(
            context.prompt_block().unwrap(),
            "<desktop_context>\nOpened from: \"Downloads\" (explorer.exe)\n\
             Folder shown: C:\\Users\\me\\Downloads\nSelected (1):\n\
             C:\\Users\\me\\Downloads\\a.pdf\n</desktop_context>"
        );
        assert_eq!(context.summary().unwrap(), "Downloads · 1 selected");
        assert_eq!(context.folder(), Some(r"C:\Users\me\Downloads"));
    }

    #[test]
    fn describes_other_windows_and_the_desktop() {
        let code = DesktopContext {
            window: Some(ForegroundWindow {
                handle: 2,
                title: "main.rs - site - Visual Studio Code".into(),
                class: "Chrome_WidgetWin_1".into(),
                app: "Code.exe".into(),
            }),
            view: None,
            pending: false,
        };
        assert!(code
            .prompt_block()
            .unwrap()
            .contains("Opened from: \"main.rs - site - Visual Studio Code\" (Code.exe)"));
        assert_eq!(
            code.summary().unwrap(),
            "main.rs - site - Visual Studio Code"
        );

        let desktop = DesktopContext {
            window: Some(ForegroundWindow {
                class: "WorkerW".into(),
                ..explorer("")
            }),
            view: Some(view(r"C:\Users\me\Desktop", &["x", "y"])),
            pending: false,
        };
        assert!(desktop
            .prompt_block()
            .unwrap()
            .contains("the Windows desktop"));
        assert_eq!(desktop.summary().unwrap(), "Desktop · 2 selected");
    }

    #[test]
    fn caps_long_selections() {
        let mut big = view("C:\\x", &[]);
        big.selection = (0..30).map(|i| format!("f{i}")).collect();
        big.selected = 75;
        let context = DesktopContext {
            window: Some(explorer("x")),
            view: Some(big),
            pending: false,
        };
        let block = context.prompt_block().unwrap();
        assert!(block.contains("Selected (75):"));
        assert!(block.contains("f19") && !block.contains("f20"));
        assert!(block.contains("…and 55 more"));
    }

    #[test]
    fn nothing_captured_means_no_block() {
        assert_eq!(with_context("hi", None), "hi");
        assert_eq!(with_context("hi", Some(&DesktopContext::default())), "hi");
        let context = DesktopContext {
            window: Some(explorer("Docs")),
            view: None,
            pending: false,
        };
        assert!(with_context("hi", Some(&context)).ends_with("</desktop_context>\n\nhi"));
    }

    #[tokio::test]
    async fn waits_for_the_explorer_lookup() {
        let tracker = Arc::new(DesktopTracker::new());
        let generation = tracker.begin(explorer("Downloads"));
        assert!(tracker.current(Duration::ZERO).await.pending);

        let finisher = Arc::clone(&tracker);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            finisher.finish(generation, Some(view(r"C:\d", &[])));
        });
        let context = tracker.current(Duration::from_secs(2)).await;
        assert!(!context.pending);
        assert_eq!(context.folder(), Some(r"C:\d"));
    }

    #[tokio::test]
    async fn a_late_lookup_never_overwrites_a_newer_capture() {
        let tracker = DesktopTracker::new();
        let old = tracker.begin(explorer("Old"));
        tracker.begin(ForegroundWindow {
            class: "Notepad".into(),
            ..explorer("New")
        });
        tracker.finish(old, Some(view(r"C:\old", &[])));
        let context = tracker.current(Duration::ZERO).await;
        assert_eq!(context.window.unwrap().title, "New");
        assert!(context.view.is_none());
    }
}
