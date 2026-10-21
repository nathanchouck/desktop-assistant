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
//! because it was too slow), its release is replayed behind a mask key so
//! Start stays closed then too (D-015).
//!
//! Pure logic, so it's tested on any platform; `win_key.rs` feeds it real
//! keyboard events on Windows.

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
    WinDown {
        side: Side,
        modifiers_held: bool,
    },
    WinUp {
        side: Side,
    },
    /// Any other key went down.
    OtherDown {
        vk: u32,
    },
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
            KeyEvent::WinUp { side } => {
                let i = side.index();
                let was_down = std::mem::replace(&mut self.down[i], false);
                let was_shown = std::mem::replace(&mut self.shown[i], false);
                if !was_down || was_shown {
                    // Windows knows this key is down (or we missed the press
                    // and it might): mask the release so Start stays closed.
                    if !self.down.iter().any(|&d| d) {
                        self.armed = false;
                    }
                    return Verdict::MaskedRelease;
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
        WinUp { side }
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
}
