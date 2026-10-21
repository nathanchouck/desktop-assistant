//! Closes Start when it opens anyway right after a Windows key tap the hook
//! handled. The hook hides a lone tap from Windows, but when it answers too
//! late Windows uses the key regardless (it stops waiting after
//! `LowLevelHooksTimeout`), so both the launcher and Start open. For a short
//! while after each tap, `start_guard_win.rs` watches the window in front
//! and asks this file what to do. Pure logic, tested on any platform.
//!
//! Only a Start (or Windows Search) that comes to the front within
//! `WATCH_FOR` of a handled tap is closed. Start opened with the mouse, with
//! Ctrl+Esc, or with the override off doesn't follow a handled tap, and a
//! Windows key press or Escape in between ends the watch (so tap, then
//! Win+S, keeps Search open). A tap made while Start was already open isn't
//! watched either: the tap didn't open it.

use std::fmt;
use std::time::{Duration, Instant};

/// After a tap, a Start that comes to the front this soon counts as caused
/// by it. Windows acts on a key it gave up waiting for about 0.3 s after
/// the key, and Start takes a moment to appear, more so on a busy PC.
pub const WATCH_FOR: Duration = Duration::from_millis(1_000);
/// Once Start is in front, how long we keep trying to close it.
pub const CLOSE_WITHIN: Duration = Duration::from_millis(1_500);
/// Start takes a moment to go away after Escape; don't send a second one
/// into its closing animation.
pub const RETRY_AFTER: Duration = Duration::from_millis(300);
/// Escapes per tap, at most (one, plus one retry).
pub const MAX_ESCAPES: u8 = 2;
/// Put the launcher back after closing Start only if a tap opened it this
/// recently (a tap can also close the launcher; then nothing comes back).
pub const RESTORE_WITHIN: Duration = Duration::from_millis(3_000);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flyout {
    Start,
    Search,
}

impl Flyout {
    fn name(self) -> &'static str {
        match self {
            Flyout::Start => "Start",
            Flyout::Search => "Windows Search",
        }
    }
}

/// Which shell flyout a process is, from its full image path. Only the
/// shell's own hosts under `<windows>\SystemApps\<package>\` count, so a
/// program that happens to be called SearchHost.exe never matches. Window
/// titles are localized and the window class (`Windows.UI.Core.CoreWindow`)
/// is shared by other shell UI, so neither is used. On recent Windows 11
/// builds Start's foreground window belongs to SearchHost.exe.
pub fn flyout_for_process(image: &str, windows_dir: &str) -> Option<Flyout> {
    let image = image.replace('/', "\\").to_ascii_lowercase();
    let root = format!(
        "{}\\systemapps\\",
        windows_dir
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    );
    let rest = image.strip_prefix(&root)?;
    let (package, exe) = rest.split_once('\\')?;
    if package.is_empty() || exe.contains('\\') {
        return None;
    }
    match exe {
        "startmenuexperiencehost.exe" => Some(Flyout::Start),
        // SearchHost: Windows 11 (also Start's window there). SearchApp:
        // Windows 10 2004 and later.
        "searchhost.exe" | "searchapp.exe" => Some(Flyout::Search),
        _ => None,
    }
}

/// What's in front, as far as the guard cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Front {
    Flyout(Flyout),
    /// Any other window, the launcher included.
    Other,
    /// No foreground window at the moment (focus is moving).
    Unknown,
}

/// A tap the hook handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tap {
    /// Close Start if it shows up (the setting); otherwise only report it.
    pub close: bool,
    /// How long after each key event the hook answered the press and the
    /// release, in ms (`None`: the press wasn't seen).
    pub press_answered_ms: Option<u32>,
    pub release_answered_ms: u32,
    /// Windows got the press anyway, so its release was replayed masked.
    pub windows_saw_press: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// We pressed Escape and it went away.
    Closed,
    /// It went away without our Escape (the launcher took the focus, or
    /// you closed it yourself).
    WentAway,
    /// The setting is off, so it was left open.
    LeftOpen,
    /// Still in front after our Escapes; we gave up.
    StayedOpen,
}

/// What happened to a Start that opened after a tap, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub flyout: Flyout,
    pub ending: Ending,
    /// From the tap reaching the guard to Start being seen in front.
    pub seen_after: Duration,
    /// From Start being seen to the guard being done with it.
    pub open_for: Duration,
    pub escapes: u8,
    pub tap: Tap,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} opened {} ms after a Windows key tap; the hook answered ",
            self.flyout.name(),
            self.seen_after.as_millis(),
        )?;
        if let Some(press) = self.tap.press_answered_ms {
            write!(f, "the press after {press} ms and ")?;
        }
        write!(f, "the release after {} ms", self.tap.release_answered_ms)?;
        if self.tap.windows_saw_press {
            write!(f, " (Windows had seen the press)")?;
        }
        write!(f, "; ")?;
        let open = self.open_for.as_millis();
        match self.ending {
            Ending::Closed => write!(
                f,
                "closed it with {} Escape{} in {open} ms",
                self.escapes,
                if self.escapes == 1 { "" } else { "s" }
            ),
            Ending::WentAway => write!(f, "it went away by itself after {open} ms"),
            Ending::LeftOpen => write!(f, "left it open (closing Start is turned off)"),
            Ending::StayedOpen => write!(
                f,
                "it was still open after {} Escape{} and {open} ms; gave up",
                self.escapes,
                if self.escapes == 1 { "" } else { "s" }
            ),
        }
    }
}

/// What the guard thread should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Not watching: the poll can stop.
    Idle,
    /// Keep watching.
    Wait,
    /// Start or Search is in front: press Escape, after checking once more
    /// that it still is, then call `escape_pressed`.
    PressEscape,
    /// The watch is over; `Some` if Start or Search opened during it.
    Done(Option<Report>),
}

#[derive(Debug, Default)]
pub struct StartGuard {
    watch: Option<Watch>,
    /// Start or Search was already in front when the Windows key went down.
    open_at_press: bool,
}

#[derive(Debug)]
struct Watch {
    tap: Tap,
    tapped_at: Instant,
    seen: Option<(Instant, Flyout)>,
    escapes: u8,
    last_escape: Option<Instant>,
}

impl StartGuard {
    /// A Windows key went down (or repeated): whatever opens next is your
    /// doing (Win+S, a second tap), so the watch ends. `front` is what's in
    /// front now: if that's Start or Search, the tap this press may turn
    /// into didn't open it (and the launcher taking focus closes it anyway).
    pub fn win_key_down(&mut self, front: Front) {
        self.watch = None;
        self.open_at_press = matches!(front, Front::Flyout(_));
    }

    /// Escape went down: Ctrl+Esc opens Start on purpose, and Escape alone
    /// closes it anyway.
    pub fn escape_down(&mut self) {
        self.watch = None;
    }

    /// A tap was handled: watch for Start from now on (a new tap replaces
    /// the watch for the one before). Returns whether it's watching.
    pub fn tapped(&mut self, now: Instant, tap: Tap) -> bool {
        if std::mem::take(&mut self.open_at_press) {
            self.watch = None;
            return false;
        }
        self.watch = Some(Watch {
            tap,
            tapped_at: now,
            seen: None,
            escapes: 0,
            last_escape: None,
        });
        true
    }

    pub fn is_watching(&self) -> bool {
        self.watch.is_some()
    }

    pub fn escape_pressed(&mut self, now: Instant) {
        if let Some(w) = self.watch.as_mut() {
            w.escapes = w.escapes.saturating_add(1);
            w.last_escape = Some(now);
        }
    }

    /// Called on every foreground change, Start visibility event and poll
    /// tick while watching. `modifiers_down`: Ctrl, Alt, Shift or Win is
    /// held, so an Escape now would mean something else (Ctrl+Esc opens
    /// Start, Alt+Esc switches windows).
    pub fn check(&mut self, now: Instant, front: Front, modifiers_down: bool) -> Step {
        let Some(w) = self.watch.as_mut() else {
            return Step::Idle;
        };
        if let Front::Flyout(flyout) = front {
            let seen_at = w.seen.get_or_insert((now, flyout)).0;
            if !w.tap.close {
                return self.finish(now, Ending::LeftOpen);
            }
            if now.saturating_duration_since(seen_at) >= CLOSE_WITHIN {
                return self.finish(now, Ending::StayedOpen);
            }
            let closing = w
                .last_escape
                .is_some_and(|t| now.saturating_duration_since(t) < RETRY_AFTER);
            if modifiers_down || closing || w.escapes >= MAX_ESCAPES {
                return Step::Wait;
            }
            return Step::PressEscape;
        }
        match (front, w.seen) {
            (Front::Other, Some(_)) => {
                let ending = if w.escapes > 0 {
                    Ending::Closed
                } else {
                    Ending::WentAway
                };
                self.finish(now, ending)
            }
            (_, Some((seen_at, _))) if now.saturating_duration_since(seen_at) >= CLOSE_WITHIN => {
                self.finish(now, Ending::StayedOpen)
            }
            (_, None) if now.saturating_duration_since(w.tapped_at) >= WATCH_FOR => {
                // Nothing opened: the ending isn't reported.
                self.finish(now, Ending::WentAway)
            }
            _ => Step::Wait,
        }
    }

    fn finish(&mut self, now: Instant, ending: Ending) -> Step {
        let Some(w) = self.watch.take() else {
            return Step::Idle;
        };
        Step::Done(w.seen.map(|(seen_at, flyout)| Report {
            flyout,
            ending,
            seen_after: seen_at.saturating_duration_since(w.tapped_at),
            open_for: now.saturating_duration_since(seen_at),
            escapes: w.escapes,
            tap: w.tap,
        }))
    }
}

/// Whether to bring the launcher back after closing Start: only if a tap
/// opened it (not closed it) moments ago.
pub fn should_restore(tap_opened_at: Option<Instant>, now: Instant) -> bool {
    tap_opened_at.is_some_and(|t| now.saturating_duration_since(t) <= RESTORE_WITHIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: &str = r"C:\Windows";
    const START: &str = r"C:\Windows\SystemApps\Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy\StartMenuExperienceHost.exe";
    const SEARCH: &str =
        r"C:\Windows\SystemApps\MicrosoftWindows.Client.CBS_cw5n1h2txyewy\SearchHost.exe";

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn tap(close: bool) -> Tap {
        Tap {
            close,
            press_answered_ms: Some(20),
            release_answered_ms: 320,
            windows_saw_press: false,
        }
    }

    const START_UP: Front = Front::Flyout(Flyout::Start);

    #[test]
    fn matches_the_shell_hosts_only_under_system_apps() {
        assert_eq!(flyout_for_process(START, WIN), Some(Flyout::Start));
        assert_eq!(flyout_for_process(SEARCH, WIN), Some(Flyout::Search));
        // Windows 10's search host, any case, either slash, trailing slash.
        let win10 = r"c:/windows/systemapps/Microsoft.Windows.Search_cw5n1h2txyewy/SearchApp.EXE";
        assert_eq!(
            flyout_for_process(win10, r"C:\WINDOWS\"),
            Some(Flyout::Search)
        );
    }

    #[test]
    fn ignores_look_alikes_and_other_shell_apps() {
        let cases = [
            r"C:\Tools\SystemApps\x\SearchHost.exe",
            r"D:\Windows\SystemApps\x\SearchHost.exe",
            r"C:\Windows\SystemApps\SearchHost.exe",
            r"C:\Windows\SystemApps\a\b\SearchHost.exe",
            r"C:\Windows\SystemApps\x\ShellExperienceHost.exe",
            r"C:\Windows\explorer.exe",
            r"C:\Program Files\App\StartMenuExperienceHost.exe",
            "",
        ];
        for case in cases {
            assert_eq!(flyout_for_process(case, WIN), None, "{case}");
        }
    }

    #[test]
    fn closes_start_that_follows_a_tap_then_reports_it() {
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.tapped(t0, tap(true));
        assert_eq!(g.check(t0 + ms(25), Front::Other, false), Step::Wait);
        assert_eq!(g.check(t0 + ms(140), START_UP, false), Step::PressEscape);
        g.escape_pressed(t0 + ms(140));
        // Still fading out: no second Escape yet.
        assert_eq!(g.check(t0 + ms(165), START_UP, false), Step::Wait);
        let Step::Done(Some(report)) = g.check(t0 + ms(200), Front::Other, false) else {
            panic!("expected a report");
        };
        assert_eq!(report.ending, Ending::Closed);
        assert_eq!(report.flyout, Flyout::Start);
        assert_eq!(report.seen_after, ms(140));
        assert_eq!(report.open_for, ms(60));
        assert_eq!(report.escapes, 1);
        assert!(!g.is_watching());
        assert_eq!(g.check(t0 + ms(225), START_UP, false), Step::Idle);
    }

    #[test]
    fn retries_once_then_gives_up() {
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.tapped(t0, tap(true));
        assert_eq!(g.check(t0, START_UP, false), Step::PressEscape);
        g.escape_pressed(t0);
        assert_eq!(
            g.check(t0 + RETRY_AFTER, START_UP, false),
            Step::PressEscape
        );
        g.escape_pressed(t0 + RETRY_AFTER);
        assert_eq!(g.check(t0 + ms(900), START_UP, false), Step::Wait);
        let Step::Done(Some(report)) = g.check(t0 + CLOSE_WITHIN, START_UP, false) else {
            panic!("expected a report");
        };
        assert_eq!(report.ending, Ending::StayedOpen);
        assert_eq!(report.escapes, 2);
    }

    #[test]
    fn never_presses_escape_while_a_modifier_is_held() {
        // Ctrl+Esc would open Start again; Alt+Esc switches windows.
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.tapped(t0, tap(true));
        assert_eq!(g.check(t0 + ms(100), START_UP, true), Step::Wait);
        assert_eq!(g.check(t0 + ms(125), START_UP, false), Step::PressEscape);
    }

    #[test]
    fn a_start_long_after_the_tap_is_left_alone() {
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.tapped(t0, tap(true));
        assert_eq!(g.check(t0 + ms(500), Front::Unknown, false), Step::Wait);
        assert_eq!(
            g.check(t0 + WATCH_FOR, Front::Other, false),
            Step::Done(None)
        );
        // Clicked the Start button later: not ours.
        assert_eq!(g.check(t0 + ms(2_000), START_UP, false), Step::Idle);
    }

    #[test]
    fn without_a_tap_nothing_is_closed() {
        // Start button, Ctrl+Esc, or the override off: no tap was handled.
        let mut g = StartGuard::default();
        assert_eq!(g.check(Instant::now(), START_UP, false), Step::Idle);
    }

    #[test]
    fn a_windows_key_press_cancels_the_watch() {
        // Tap, then Win+S: Search is yours.
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        assert!(g.tapped(t0, tap(true)));
        g.win_key_down(Front::Other);
        assert_eq!(
            g.check(t0 + ms(100), Front::Flyout(Flyout::Search), false),
            Step::Idle
        );
    }

    #[test]
    fn escape_cancels_the_watch() {
        // Tap, then Ctrl+Esc: Start is yours.
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.tapped(t0, tap(true));
        g.escape_down();
        assert_eq!(g.check(t0 + ms(100), START_UP, false), Step::Idle);
    }

    #[test]
    fn a_tap_while_start_is_open_is_not_watched() {
        // Start opened with the mouse, then a tap to get the launcher: the
        // launcher taking focus closes Start; no Escape from us.
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.win_key_down(START_UP);
        assert!(!g.tapped(t0, tap(true)));
        assert_eq!(g.check(t0 + ms(30), START_UP, false), Step::Idle);
        // The next tap, from somewhere else, is watched again.
        g.win_key_down(Front::Other);
        assert!(g.tapped(t0 + ms(500), tap(true)));
        assert_eq!(g.check(t0 + ms(600), START_UP, false), Step::PressEscape);
    }

    #[test]
    fn with_the_setting_off_start_is_only_reported() {
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.tapped(t0, tap(false));
        let Step::Done(Some(report)) = g.check(t0 + ms(80), START_UP, false) else {
            panic!("expected a report");
        };
        assert_eq!(report.ending, Ending::LeftOpen);
        assert_eq!(report.escapes, 0);
    }

    #[test]
    fn start_that_closes_by_itself_is_reported_as_such() {
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.tapped(t0, tap(true));
        assert_eq!(g.check(t0 + ms(50), START_UP, true), Step::Wait);
        let Step::Done(Some(report)) = g.check(t0 + ms(90), Front::Other, false) else {
            panic!("expected a report");
        };
        assert_eq!(report.ending, Ending::WentAway);
    }

    #[test]
    fn focus_in_flux_is_not_mistaken_for_closed() {
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.tapped(t0, tap(true));
        assert_eq!(g.check(t0 + ms(100), START_UP, false), Step::PressEscape);
        g.escape_pressed(t0 + ms(100));
        assert_eq!(g.check(t0 + ms(130), Front::Unknown, false), Step::Wait);
        // Seen Start keeps the watch going past WATCH_FOR until it's gone.
        assert!(matches!(
            g.check(t0 + ms(1_200), Front::Other, false),
            Step::Done(Some(Report {
                ending: Ending::Closed,
                ..
            }))
        ));
    }

    #[test]
    fn a_new_tap_starts_a_fresh_watch() {
        let t0 = Instant::now();
        let mut g = StartGuard::default();
        g.tapped(t0, tap(true));
        g.escape_pressed(t0);
        g.escape_pressed(t0);
        g.tapped(t0 + ms(900), tap(true));
        assert_eq!(g.check(t0 + ms(1_500), START_UP, false), Step::PressEscape);
    }

    #[test]
    fn restores_the_launcher_only_after_a_recent_opening_tap() {
        let now = Instant::now() + ms(10_000);
        assert!(should_restore(Some(now - ms(500)), now));
        assert!(!should_restore(Some(now - ms(5_000)), now));
        assert!(!should_restore(None, now));
    }

    #[test]
    fn reports_read_well_in_the_log() {
        let report = Report {
            flyout: Flyout::Search,
            ending: Ending::Closed,
            seen_after: ms(140),
            open_for: ms(60),
            escapes: 1,
            tap: Tap {
                close: true,
                press_answered_ms: Some(410),
                release_answered_ms: 350,
                windows_saw_press: true,
            },
        };
        assert_eq!(
            report.to_string(),
            "Windows Search opened 140 ms after a Windows key tap; the hook answered the \
             press after 410 ms and the release after 350 ms (Windows had seen the press); \
             closed it with 1 Escape in 60 ms"
        );
        let left = Report {
            ending: Ending::LeftOpen,
            tap: Tap {
                press_answered_ms: None,
                windows_saw_press: false,
                ..report.tap
            },
            ..report
        };
        assert_eq!(
            left.to_string(),
            "Windows Search opened 140 ms after a Windows key tap; the hook answered the \
             release after 350 ms; left it open (closing Start is turned off)"
        );
    }
}
