//! Decides what to do with Windows key presses. Every release of a Windows
//! key we saw go down gets replayed behind a mask key so Start never opens;
//! a lone tap (no other key in between) also opens the launcher.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    /// A Windows key went down. `modifiers_held` is true if Shift, Ctrl or
    /// Alt were already down, which makes this a combo from the start.
    WinDown {
        side: Side,
        modifiers_held: bool,
    },
    WinUp {
        side: Side,
    },
    /// Any other key went down.
    OtherDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Let the event through unchanged.
    Pass,
    /// A Windows key we saw go down is being released. The caller swallows
    /// this key-up and replays it as mask-down, mask-up, Win-up in one batch,
    /// so Windows sees a combo and never opens Start. `tap` is true for a
    /// lone tap, which also opens the launcher.
    MaskedRelease { tap: bool },
}

#[derive(Debug, Default)]
pub struct TapDetector {
    /// Which Windows keys are currently down.
    down: [bool; 2],
    /// True while the current press can still turn out to be a lone tap.
    armed: bool,
}

impl TapDetector {
    pub fn on_event(&mut self, event: KeyEvent) -> Verdict {
        match event {
            KeyEvent::WinDown {
                side,
                modifiers_held,
            } => {
                let i = side.index();
                if !self.down[i] {
                    // First press (auto-repeat while held changes nothing).
                    let other_win_down = self.down[1 - i];
                    self.down[i] = true;
                    self.armed = !modifiers_held && !other_win_down;
                }
                Verdict::Pass
            }
            KeyEvent::OtherDown => {
                if self.down.iter().any(|&d| d) {
                    self.armed = false;
                }
                Verdict::Pass
            }
            KeyEvent::WinUp { side } => {
                let i = side.index();
                if !std::mem::replace(&mut self.down[i], false) {
                    // We never saw this key go down (e.g. it went to an admin
                    // window): leave it alone.
                    return Verdict::Pass;
                }
                let tap = self.armed && !self.down[1 - i];
                self.armed = false;
                Verdict::MaskedRelease { tap }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyEvent::*;
    use Side::*;

    const TAP: Verdict = Verdict::MaskedRelease { tap: true };
    const NO_TAP: Verdict = Verdict::MaskedRelease { tap: false };

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

    #[test]
    fn lone_tap_opens_the_launcher() {
        assert_eq!(
            run(&[down(Left), WinUp { side: Left }]),
            [Verdict::Pass, TAP]
        );
        assert_eq!(run(&[down(Right), WinUp { side: Right }])[1], TAP);
    }

    #[test]
    fn auto_repeat_still_counts_as_a_tap() {
        let v = run(&[down(Left), down(Left), down(Left), WinUp { side: Left }]);
        assert_eq!(v, [Verdict::Pass, Verdict::Pass, Verdict::Pass, TAP]);
    }

    #[test]
    fn combo_is_masked_but_not_a_tap() {
        // Win+E: the release is still masked, so Start can't slip through
        // even if a combo is misjudged.
        let v = run(&[down(Left), OtherDown, WinUp { side: Left }]);
        assert_eq!(v, [Verdict::Pass, Verdict::Pass, NO_TAP]);
    }

    #[test]
    fn modifier_held_before_win_is_not_a_tap() {
        let v = run(&[
            WinDown {
                side: Left,
                modifiers_held: true,
            },
            WinUp { side: Left },
        ]);
        assert_eq!(v[1], NO_TAP);
    }

    #[test]
    fn both_windows_keys_are_masked_but_not_a_tap() {
        let v = run(&[
            down(Left),
            down(Right),
            WinUp { side: Right },
            WinUp { side: Left },
        ]);
        assert_eq!(v, [Verdict::Pass, Verdict::Pass, NO_TAP, NO_TAP]);
    }

    #[test]
    fn up_without_down_is_left_alone() {
        assert_eq!(run(&[WinUp { side: Left }]), [Verdict::Pass]);
    }

    #[test]
    fn keys_pressed_without_win_do_not_disarm_next_tap() {
        let v = run(&[OtherDown, down(Left), WinUp { side: Left }]);
        assert_eq!(v[2], TAP);
    }

    #[test]
    fn a_combo_does_not_poison_the_next_tap() {
        let v = run(&[
            down(Left),
            OtherDown,
            WinUp { side: Left },
            down(Left),
            WinUp { side: Left },
        ]);
        assert_eq!(v[2], NO_TAP);
        assert_eq!(v[4], TAP);
    }
}
