//! Window rows ("Left Half", "Move to Next Monitor", ...) and the steps
//! each takes on the window you were in.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use super::geometry::Rect;
use super::layouts::{self, Layout, NoTarget};
use super::restore::RestoreMemory;
use super::{can_move, Show, WindowState};
use crate::desktop::DesktopTracker;
use crate::features::system::matching;
use crate::fuzzy::FuzzyQuery;
use crate::platform::{ForegroundWindow, Shell};
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};

#[cfg(not(windows))]
use super::fallback as os;
#[cfg(windows)]
use super::win as os;

const NAME: &str = "window_layout";
/// After the launcher hides, before the moved window is brought forward.
const FOCUS_DELAY: Duration = Duration::from_millis(80);
/// Window titles longer than this are cut in the subtitle.
const TITLE_CHARS: usize = 40;

pub struct LayoutProvider {
    desktop: Arc<DesktopTracker>,
    shell: Arc<Shell>,
    memory: Arc<Mutex<RestoreMemory>>,
}

impl LayoutProvider {
    pub fn new(desktop: Arc<DesktopTracker>, shell: Arc<Shell>) -> Self {
        Self {
            desktop,
            shell,
            memory: Arc::default(),
        }
    }
}

/// Rows for `query`; the subtitle names the window they'd move.
fn rows(query: &str, window: Option<&ForegroundWindow>) -> Vec<ResultItem> {
    if !matching::long_enough(query) {
        return Vec::new();
    }
    let subtitle = match window {
        Some(w) if can_move(&w.class) => format!("Moves {}", describe(w)),
        _ => "Window layout".to_owned(),
    };
    let mut fuzzy = FuzzyQuery::new(query);
    Layout::ALL
        .into_iter()
        .filter_map(|layout| {
            let score = matching::score(&mut fuzzy, layout.title(), layout.keywords())?;
            Some(
                ResultItem::new(layout.id(), ItemKind::Command, layout.title())
                    .with_subtitle(subtitle.clone())
                    .with_icon(glyph(layout.glyph()))
                    .with_score(score)
                    .with_actions([Action::Run]),
            )
        })
        .collect()
}

/// “Untitled - Notepad”, or the program when the title is empty.
fn describe(window: &ForegroundWindow) -> String {
    let title = window.title.trim();
    if title.is_empty() {
        return window.app.trim_end_matches(".exe").to_owned();
    }
    let cut: String = title.chars().take(TITLE_CHARS).collect();
    let more = if title.chars().count() > TITLE_CHARS {
        "…"
    } else {
        ""
    };
    format!("“{cut}{more}”")
}

#[async_trait]
impl SearchProvider for LayoutProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn query(&self, query: &str, _limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        Ok(rows(query, self.desktop.window().as_ref()))
    }

    fn owns(&self, id: &str) -> bool {
        Layout::owns(id)
    }

    async fn activate(&self, id: &str, _action: Action) -> Result<Activation, SearchError> {
        let layout = Layout::from_id(id).ok_or_else(|| SearchError::Provider {
            provider: NAME,
            message: format!("{id} is unknown"),
        })?;
        if !os::SUPPORTED {
            return Ok(notice("Window layouts work only on Windows."));
        }
        let Some(window) = self.desktop.window().filter(|w| can_move(&w.class)) else {
            return Ok(notice(
                "Open the launcher from the window you want to move, then try again.",
            ));
        };
        let handle = window.handle;
        let memory = Arc::clone(&self.memory);
        let done = tokio::task::spawn_blocking(move || {
            let mut memory = match memory.lock() {
                Ok(m) => m,
                Err(poisoned) => poisoned.into_inner(),
            };
            apply(&Os, layout, handle, &mut memory)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        if let Err(message) = done {
            return Ok(notice(message));
        }
        tracing::info!("window layout: {}", layout.id());
        // The launcher is still in front; bring the window back once it hides.
        let shell = Arc::clone(&self.shell);
        tokio::spawn(async move {
            tokio::time::sleep(FOCUS_DELAY).await;
            if let Err(e) = shell.focus_window(handle).await {
                tracing::debug!("could not bring the window to the front: {e}");
            }
        });
        Ok(Activation::Hide)
    }
}

fn notice(message: impl Into<String>) -> Activation {
    Activation::Notice {
        message: message.into(),
    }
}

/// What a layout does to a window, so its steps can be tested with a fake.
trait Desk {
    fn read(&self, handle: isize) -> Result<WindowState, String>;
    fn show(&self, handle: isize, show: Show) -> Result<(), String>;
    /// Moves the window's visible frame to `target`; returns where it landed.
    fn place(&self, handle: isize, target: Rect) -> Result<Rect, String>;
}

/// The real windows.
struct Os;

impl Desk for Os {
    fn read(&self, handle: isize) -> Result<WindowState, String> {
        os::read(handle)
    }

    fn show(&self, handle: isize, show: Show) -> Result<(), String> {
        os::show(handle, show)
    }

    fn place(&self, handle: isize, target: Rect) -> Result<Rect, String> {
        os::place(handle, target)
    }
}

/// Runs `layout` on the window. `Err` is a message for the footer.
fn apply(
    desk: &impl Desk,
    layout: Layout,
    handle: isize,
    memory: &mut RestoreMemory,
) -> Result<(), String> {
    let mut state = desk.read(handle)?;
    match layout {
        Layout::Maximize => {
            // Windows remembers the size to restore to.
            desk.show(handle, Show::Maximize)
        }
        Layout::Restore => {
            if state.maximized || state.minimized {
                return desk.show(handle, Show::Restore);
            }
            let before = memory
                .take(handle)
                .ok_or("Nothing to restore: the launcher hasn't moved that window.")?;
            if before.maximized {
                desk.show(handle, Show::Maximize)
            } else {
                desk.place(handle, before.frame).map(|_| ())
            }
        }
        _ => {
            if state.minimized {
                // Back to normal (or maximized, if it was) to see where it is.
                desk.show(handle, Show::Restore)?;
                state = desk.read(handle)?;
            }
            let was_maximized = state.maximized;
            memory.before_move(handle, state.placement.frame, was_maximized);
            if was_maximized {
                // Layouts start from the window's normal size.
                desk.show(handle, Show::Restore)?;
                state = desk.read(handle)?;
            }
            let target = layouts::target(layout, &state.placement).map_err(|e| match e {
                NoTarget::OneMonitor => "There's only one monitor.".to_owned(),
                NoTarget::NotARegion => format!("{} isn't a layout.", layout.title()),
            })?;
            let landed = desk.place(handle, target)?;
            memory.after_move(handle, landed);
            if layout == Layout::NextMonitor && was_maximized {
                desk.show(handle, Show::Maximize)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const WORK: Rect = Rect::new(0, 0, 1920, 1032);
    const SECOND: Rect = Rect::new(1920, 0, 3840, 1032);
    const NORMAL: Rect = Rect::new(100, 100, 900, 700);

    /// A window that does what it's told, and a log of what it was told.
    struct Fake {
        state: RefCell<WindowState>,
        /// Where Restore puts it back to.
        normal: RefCell<Rect>,
        log: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(maximized: bool, minimized: bool) -> Self {
            Self {
                state: RefCell::new(WindowState {
                    placement: layouts::Placement {
                        frame: if maximized { WORK } else { NORMAL },
                        work: WORK,
                        works: vec![WORK, SECOND],
                        resizable: true,
                    },
                    maximized,
                    minimized,
                }),
                normal: RefCell::new(NORMAL),
                log: RefCell::default(),
            }
        }

        fn log(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
    }

    impl Desk for Fake {
        fn read(&self, _handle: isize) -> Result<WindowState, String> {
            Ok(self.state.borrow().clone())
        }

        fn show(&self, _handle: isize, show: Show) -> Result<(), String> {
            self.log.borrow_mut().push(format!("{show:?}"));
            let mut s = self.state.borrow_mut();
            match show {
                Show::Restore => {
                    s.maximized = false;
                    s.minimized = false;
                    s.placement.frame = *self.normal.borrow();
                }
                Show::Maximize => {
                    *self.normal.borrow_mut() = s.placement.frame;
                    s.maximized = true;
                    s.placement.frame = s.placement.work;
                }
            }
            Ok(())
        }

        fn place(&self, _handle: isize, target: Rect) -> Result<Rect, String> {
            let f = target;
            self.log.borrow_mut().push(format!(
                "place {},{},{},{}",
                f.left, f.top, f.right, f.bottom
            ));
            let mut s = self.state.borrow_mut();
            s.placement.frame = target;
            if SECOND.left <= target.left {
                s.placement.work = SECOND;
            }
            *self.normal.borrow_mut() = target;
            Ok(target)
        }
    }

    #[test]
    fn snaps_a_normal_window() {
        let fake = Fake::new(false, false);
        let mut memory = RestoreMemory::default();
        apply(&fake, Layout::LeftHalf, 1, &mut memory).unwrap();
        assert_eq!(fake.log(), ["place 0,0,960,1032"]);
    }

    #[test]
    fn restores_a_maximized_window_before_snapping() {
        let fake = Fake::new(true, false);
        let mut memory = RestoreMemory::default();
        apply(&fake, Layout::RightHalf, 1, &mut memory).unwrap();
        assert_eq!(fake.log(), ["Restore", "place 960,0,1920,1032"]);
        // Restore Window maximizes it again, as it was.
        apply(&fake, Layout::Restore, 1, &mut memory).unwrap();
        assert_eq!(fake.log().last().unwrap(), "Maximize");
    }

    #[test]
    fn restore_puts_it_back_where_it_started() {
        let fake = Fake::new(false, false);
        let mut memory = RestoreMemory::default();
        apply(&fake, Layout::LeftHalf, 1, &mut memory).unwrap();
        apply(&fake, Layout::BottomHalf, 1, &mut memory).unwrap();
        apply(&fake, Layout::Restore, 1, &mut memory).unwrap();
        assert_eq!(fake.log().last().unwrap(), "place 100,100,900,700");
        // Twice: nothing left to restore.
        let again = apply(&fake, Layout::Restore, 1, &mut memory);
        assert!(again.unwrap_err().starts_with("Nothing to restore"));
    }

    #[test]
    fn restore_on_a_maximized_window_unmaximizes_it() {
        let fake = Fake::new(true, false);
        let mut memory = RestoreMemory::default();
        apply(&fake, Layout::Restore, 1, &mut memory).unwrap();
        assert_eq!(fake.log(), ["Restore"]);
    }

    #[test]
    fn a_minimized_window_comes_back_first() {
        let fake = Fake::new(false, true);
        let mut memory = RestoreMemory::default();
        apply(&fake, Layout::Center, 1, &mut memory).unwrap();
        assert_eq!(fake.log(), ["Restore", "place 560,216,1360,816"]);
    }

    #[test]
    fn next_monitor_keeps_a_window_maximized() {
        let fake = Fake::new(true, false);
        let mut memory = RestoreMemory::default();
        apply(&fake, Layout::NextMonitor, 1, &mut memory).unwrap();
        assert_eq!(
            fake.log(),
            ["Restore", "place 2020,100,2820,700", "Maximize"]
        );
    }

    #[test]
    fn next_monitor_with_one_monitor_says_so() {
        let fake = Fake::new(false, false);
        fake.state.borrow_mut().placement.works = vec![WORK];
        let mut memory = RestoreMemory::default();
        let err = apply(&fake, Layout::NextMonitor, 1, &mut memory).unwrap_err();
        assert_eq!(err, "There's only one monitor.");
        assert!(fake.log().is_empty());
    }

    #[test]
    fn maximize_leaves_restoring_to_windows() {
        let fake = Fake::new(false, false);
        let mut memory = RestoreMemory::default();
        apply(&fake, Layout::Maximize, 1, &mut memory).unwrap();
        apply(&fake, Layout::Restore, 1, &mut memory).unwrap();
        assert_eq!(fake.log(), ["Maximize", "Restore"]);
    }

    fn window(title: &str, class: &str) -> ForegroundWindow {
        ForegroundWindow {
            handle: 1,
            title: title.into(),
            class: class.into(),
            app: "notepad.exe".into(),
        }
    }

    fn ids(rows: &[ResultItem]) -> Vec<String> {
        rows.iter().map(|r| r.id.clone()).collect()
    }

    #[test]
    fn rows_name_the_window() {
        let notepad = window("Untitled - Notepad", "Notepad");
        let found = rows("left half", Some(&notepad));
        assert_eq!(found[0].id, "layout:left-half");
        assert_eq!(found[0].subtitle, "Moves “Untitled - Notepad”");
        let desktop = window("", "Progman");
        assert_eq!(
            rows("left half", Some(&desktop))[0].subtitle,
            "Window layout"
        );
        assert_eq!(rows("left half", None)[0].subtitle, "Window layout");
        let untitled = window("  ", "Notepad");
        assert_eq!(describe(&untitled), "notepad");
        let long = window(&"x".repeat(60), "Notepad");
        assert_eq!(describe(&long).chars().count(), TITLE_CHARS + 3);
    }

    #[test]
    fn finds_layouts_by_name_and_keyword() {
        let best = |q: &str| {
            let mut found = rows(q, None);
            found.sort_by(|a, b| b.score.total_cmp(&a.score));
            found.first().map(|r| r.id.clone())
        };
        assert_eq!(best("right half").as_deref(), Some("layout:right-half"));
        assert_eq!(best("snap left").as_deref(), Some("layout:left-half"));
        assert_eq!(best("maximize").as_deref(), Some("layout:maximize"));
        assert_eq!(best("next monitor").as_deref(), Some("layout:next-monitor"));
        assert_eq!(best("next display").as_deref(), Some("layout:next-monitor"));
        assert_eq!(best("center third").as_deref(), Some("layout:center-third"));
        assert!(ids(&rows("snap", None)).len() >= 10, "snap finds them all");
        assert!(rows("photoshop", None).is_empty());
        assert!(rows("l", None).is_empty());
    }

    #[tokio::test]
    async fn without_a_window_it_says_what_to_do() {
        let p = LayoutProvider::new(Arc::new(DesktopTracker::new()), Arc::new(Shell::new()));
        assert!(p.owns("layout:left-half"));
        assert!(!p.owns("sys:lock"));
        let done = p.activate("layout:left-half", Action::Run).await.unwrap();
        assert!(matches!(done, Activation::Notice { .. }));
    }
}
