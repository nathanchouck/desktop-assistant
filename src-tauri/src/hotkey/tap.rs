//! Decides what to do with Windows key presses so a lone tap opens the
//! launcher and never reaches Start, while Win+<key> shortcuts keep working.
//!
//! A Windows key press is hidden from Windows until we know what it is
//! (D-022):
//! - another key goes down while Win is held: it's a shortcut, so Win is
//!   pressed for Windows (injected) just before that key, and it works as
//!   usual from there;
//! - Win comes back up with nothing in between: a lone tap. The release is
//!   hidden too, so Windows saw nothing at all and Start can't open.
//!
//! When Windows did see a Win press (a shortcut, or a press our hook missed
//! or answered too late), its release is replayed behind a mask key so
//! Start stays closed then too (D-015). A press we meant to hide but Windows
//! got anyway is still a tap for the launcher; without the masked release
//! Windows would think Win is still held and turn every key into Win+key.
//!
//! Pure logic, so it's tested on any platform; `win_key.rs` feeds it real
//! keyboard events on Windows.

use std::time::Duration;

/// A held Windows key auto-repeats many times a second, so this long without
/// any Windows key event while the detector thinks one is held means its
/// release never reached us (the hook was skipped or removed meanwhile).
pub const LOST_RELEASE_AFTER: Duration = Duration::from_millis(1_500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Side::Left => 0,
            Side::Right => 1,
        }
    }
}

/// The L key: Win+L locks the PC.
pub const VK_L: u32 = 0x4C;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    /// A Windows key went down. `modifiers_held` is true if Shift, Ctrl or
    /// Alt were already down, which makes this a shortcut from the start.
    WinDown { side: Side, modifiers_held: bool },
    WinUp {
        side: Side,
        /// Windows' own key state said this Win key was down when the
        /// release reached the hook: Windows saw the press even if we meant
        /// to hide it (our hook answered after `LowLevelHooksTimeout`).
        os_saw_down: bool,
    },
    /// Any other key went down.
    OtherDown { vk: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Let the event through unchanged.
    Pass,
    /// Hide the event from Windows.
    Swallow,
    /// A lone tap ended: hide this release and open the launcher.
    Tap,
    /// A shortcut is starting while Windows doesn't know Win is down: press
    /// these Windows keys for it first, then let this key through.
    PressWinFirst { left: bool, right: bool },
    /// Win+L while Windows doesn't know Win is down. Windows ignores an
    /// injected Win for locking, so lock the PC directly and hide the L.
    Lock,
    /// Windows saw this Win key go down: hide the real release and replay
    /// it behind a mask key so Start doesn't open.
    MaskedRelease,
    /// A lone tap, but Windows saw the press anyway (a late answer): open
    /// the launcher *and* replay the release masked, so Windows neither
    /// opens Start nor keeps thinking Win is held down.
    MaskedTap,
}

#[derive(Debug, Default)]
pub struct TapDetector {
    /// Which Windows keys are physically down.
    down: [bool; 2],
    /// Which of those Windows knows about.
    shown: [bool; 2],
    /// True while the current press can still turn out to be a lone tap.
    armed: bool,
}

impl TapDetector {
    /// No Windows key is held (safe to re-install the hook).
    pub fn is_idle(&self) -> bool {
        !self.down.iter().any(|&d| d)
    }

    /// Whether a Windows key we think is held was really let go without us
    /// seeing it: nothing for `quiet` and Windows says no Win key is down.
    /// Without this check the detector would stay "held" for good, and the
    /// periodic re-install (which waits for idle, D-025) would never run.
    /// Windows' state matters for a held shortcut (Win+arrows): the arrow
    /// repeats, Win doesn't, but Windows knows Win is down.
    pub fn release_lost(&self, quiet: Duration, os_win_down: bool) -> bool {
        !self.is_idle() && quiet >= LOST_RELEASE_AFTER && !os_win_down
    }

    /// Forgets everything, after `release_lost`.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn on_event(&mut self, event: KeyEvent) -> Verdict {
        match event {
            KeyEvent::WinDown {
                side,
                modifiers_held,
            } => {
                let i = side.index();
                if self.down[i] {
                    // Auto-repeat: Windows gets it only if it saw the press.
                    return if self.shown[i] {
                        Verdict::Pass
                    } else {
                        Verdict::Swallow
                    };
                }
                self.down[i] = true;
                let other_down = self.down[1 - i];
                if modifiers_held || other_down {
                    // Shift+Win+S and the like: a shortcut from the start.
                    self.shown[i] = true;
                    self.armed = false;
                    Verdict::Pass
                } else {
                    self.shown[i] = false;
                    self.armed = true;
                    Verdict::Swallow
                }
            }
            KeyEvent::OtherDown { vk } => {
                if !self.down.iter().any(|&d| d) {
                    return Verdict::Pass;
                }
                self.armed = false;
                let hidden = [
                    self.down[0] && !self.shown[0],
                    self.down[1] && !self.shown[1],
                ];
                if !hidden.iter().any(|&h| h) {
                    return Verdict::Pass;
                }
                if vk == VK_L {
                    // The Win press stays hidden; its release is hidden too.
                    return Verdict::Lock;
                }
                self.shown = [self.shown[0] || hidden[0], self.shown[1] || hidden[1]];
                Verdict::PressWinFirst {
                    left: hidden[0],
                    right: hidden[1],
                }
            }
            KeyEvent::WinUp { side, os_saw_down } => {
                let i = side.index();
                let was_down = std::mem::replace(&mut self.down[i], false);
                let was_shown = std::mem::replace(&mut self.shown[i], false);
                if !was_down || was_shown || os_saw_down {
                    // Windows knows this key is down (or we missed the press
                    // and it might): mask the release so Start stays closed.
                    // A press we meant to hide that Windows saw anyway is
                    // still a tap for the launcher.
                    let tap = was_down && !was_shown && self.armed && !self.down[1 - i];
                    if !self.down.iter().any(|&d| d) {
                        self.armed = false;
                    }
                    return if tap {
                        Verdict::MaskedTap
                    } else {
                        Verdict::MaskedRelease
                    };
                }
                // Windows never saw this press: hide the release too.
                let tap = self.armed && !self.down[1 - i];
                self.armed = false;
                if tap {
                    Verdict::Tap
                } else {
                    Verdict::Swallow
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyEvent::*;
    use Side::*;
    use Verdict::*;

    fn run(events: &[KeyEvent]) -> Vec<Verdict> {
        let mut d = TapDetector::default();
        events.iter().map(|&e| d.on_event(e)).collect()
    }

    fn down(side: Side) -> KeyEvent {
        WinDown {
            side,
            modifiers_held: false,
        }
    }

    fn up(side: Side) -> KeyEvent {
        WinUp {
            side,
            os_saw_down: false,
        }
    }

    /// A release where Windows' key state says it got the press.
    fn up_seen(side: Side) -> KeyEvent {
        WinUp {
            side,
            os_saw_down: true,
        }
    }

    const E: KeyEvent = OtherDown { vk: 0x45 };

    #[test]
    fn a_lone_tap_is_hidden_from_windows_entirely() {
        assert_eq!(run(&[down(Left), up(Left)]), [Swallow, Tap]);
        assert_eq!(run(&[down(Right), up(Right)]), [Swallow, Tap]);
    }

    #[test]
    fn auto_repeat_stays_hidden_and_still_taps() {
        let v = run(&[down(Left), down(Left), down(Left), up(Left)]);
        assert_eq!(v, [Swallow, Swallow, Swallow, Tap]);
    }

    #[test]
    fn a_shortcut_presses_win_just_in_time() {
        // Win+E: Win goes to Windows right before E, then works as usual.
        let v = run(&[down(Left), E, down(Left), up(Left)]);
        assert_eq!(
            v,
            [
                Swallow,
                PressWinFirst {
                    left: true,
                    right: false
                },
                Pass,
                MaskedRelease
            ]
        );
    }

    #[test]
    fn later_keys_in_a_shortcut_pass_straight_through() {
        // Win+Shift+S pressed in that order.
        let v = run(&[down(Left), OtherDown { vk: 0x10 }, OtherDown { vk: 0x53 }]);
        assert_eq!(v[2], Pass);
    }

    #[test]
    fn win_l_locks_without_showing_win() {
        let v = run(&[down(Left), OtherDown { vk: VK_L }, up(Left)]);
        assert_eq!(v, [Swallow, Lock, Swallow]);
    }

    #[test]
    fn modifier_held_first_makes_a_plain_shortcut() {
        let v = run(&[
            WinDown {
                side: Left,
                modifiers_held: true,
            },
            E,
            up(Left),
        ]);
        assert_eq!(v, [Pass, Pass, MaskedRelease]);
    }

    #[test]
    fn both_windows_keys_never_tap() {
        let v = run(&[down(Left), down(Right), up(Right), up(Left)]);
        assert_eq!(v, [Swallow, Pass, MaskedRelease, Swallow]);
    }

    #[test]
    fn a_release_we_never_saw_pressed_is_masked() {
        // The hook was too slow for the press, so Windows may have it.
        assert_eq!(run(&[up(Left)]), [MaskedRelease]);
    }

    #[test]
    fn keys_without_win_pass_and_do_not_disarm_the_next_tap() {
        let v = run(&[E, down(Left), up(Left)]);
        assert_eq!(v, [Pass, Swallow, Tap]);
    }

    #[test]
    fn a_shortcut_does_not_poison_the_next_tap() {
        let v = run(&[down(Left), E, up(Left), down(Left), up(Left)]);
        assert_eq!(v[2], MaskedRelease);
        assert_eq!(&v[3..], [Swallow, Tap]);
    }

    #[test]
    fn idle_only_when_no_windows_key_is_held() {
        let mut d = TapDetector::default();
        assert!(d.is_idle());
        d.on_event(down(Left));
        assert!(!d.is_idle());
        d.on_event(up(Left));
        assert!(d.is_idle());
    }

    #[test]
    fn fast_repeated_taps_each_tap() {
        let v = run(&[
            down(Left),
            up(Left),
            down(Left),
            up(Left),
            down(Left),
            up(Left),
        ]);
        assert_eq!(v, [Swallow, Tap, Swallow, Tap, Swallow, Tap]);
    }

    #[test]
    fn a_tap_windows_saw_anyway_still_taps_but_masks_the_release() {
        // The press reached Windows (late answer): open the launcher and
        // replay the release masked, or Win stays stuck down in Windows.
        assert_eq!(run(&[down(Left), up_seen(Left)]), [Swallow, MaskedTap]);
        assert_eq!(
            run(&[down(Right), down(Right), up_seen(Right)]),
            [Swallow, Swallow, MaskedTap]
        );
    }

    #[test]
    fn a_shortcut_release_is_never_a_tap_even_if_windows_saw_it() {
        let v = run(&[down(Left), E, up_seen(Left)]);
        assert_eq!(v[2], MaskedRelease);
        let v = run(&[down(Left), OtherDown { vk: VK_L }, up_seen(Left)]);
        assert_eq!(v, [Swallow, Lock, MaskedRelease]);
    }

    #[test]
    fn a_release_windows_saw_without_our_press_is_masked_not_a_tap() {
        assert_eq!(run(&[up_seen(Left)]), [MaskedRelease]);
    }

    #[test]
    fn a_late_tap_does_not_poison_the_next_one() {
        let v = run(&[down(Left), up_seen(Left), down(Left), up(Left)]);
        assert_eq!(v, [Swallow, MaskedTap, Swallow, Tap]);
    }

    #[test]
    fn both_keys_with_a_late_release_never_tap() {
        let v = run(&[down(Left), down(Right), up_seen(Right), up_seen(Left)]);
        assert_eq!(v, [Swallow, Pass, MaskedRelease, MaskedRelease]);
    }

    #[test]
    fn a_lost_release_is_noticed_only_when_quiet_and_windows_agrees() {
        let mut d = TapDetector::default();
        let quiet = LOST_RELEASE_AFTER;
        assert!(!d.release_lost(quiet, false), "idle: nothing to lose");
        d.on_event(down(Left));
        assert!(!d.release_lost(quiet - Duration::from_millis(1), false));
        assert!(!d.release_lost(quiet, true), "Windows says Win is held");
        assert!(d.release_lost(quiet, false));
    }

    #[test]
    fn a_reset_detector_taps_again() {
        let mut d = TapDetector::default();
        d.on_event(down(Left));
        d.on_event(E);
        d.reset();
        assert!(d.is_idle());
        assert_eq!(d.on_event(down(Left)), Swallow);
        assert_eq!(d.on_event(up(Left)), Tap);
    }
}
