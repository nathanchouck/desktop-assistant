//! Decides when the Windows key was tapped on its own (no combo), as opposed
//! to being used for Win+E, Win+V and so on. Pure logic, so it's tested on
//! any platform; `win_key.rs` feeds it real keyboard events on Windows.

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
    /// A lone tap just finished. The caller swallows this Win-up, replays it
    /// behind a masking key so Windows doesn't open Start, and opens the launcher.
    Tap,
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
                if self.down[i] {
                    // Auto-repeat while held: nothing changes.
                    return Verdict::Pass;
                }
                let other_win_down = self.down[1 - i];
                self.down[i] = true;
                self.armed = !modifiers_held && !other_win_down;
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
                let was_down = std::mem::replace(&mut self.down[i], false);
                let tap = was_down && self.armed && !self.down[1 - i];
                self.armed = false;
                if tap {
                    Verdict::Tap
                } else {
                    Verdict::Pass
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
    fn lone_tap_triggers() {
        assert_eq!(
            run(&[down(Left), WinUp { side: Left }]),
            [Verdict::Pass, Verdict::Tap]
        );
        assert_eq!(run(&[down(Right), WinUp { side: Right }])[1], Verdict::Tap);
    }

    #[test]
    fn auto_repeat_still_counts_as_a_tap() {
        let v = run(&[down(Left), down(Left), down(Left), WinUp { side: Left }]);
        assert_eq!(v[3], Verdict::Tap);
    }

    #[test]
    fn combo_does_not_trigger() {
        // Win+E
        let v = run(&[down(Left), OtherDown, WinUp { side: Left }]);
        assert_eq!(v[2], Verdict::Pass);
    }

    #[test]
    fn modifier_held_before_win_does_not_trigger() {
        // Ctrl held, then Win tapped
        let v = run(&[
            WinDown {
                side: Left,
                modifiers_held: true,
            },
            WinUp { side: Left },
        ]);
        assert_eq!(v[1], Verdict::Pass);
    }

    #[test]
    fn both_windows_keys_do_not_trigger() {
        let v = run(&[
            down(Left),
            down(Right),
            WinUp { side: Right },
            WinUp { side: Left },
        ]);
        assert_eq!(v, [Verdict::Pass; 4]);
    }

    #[test]
    fn up_without_down_is_ignored() {
        assert_eq!(run(&[WinUp { side: Left }]), [Verdict::Pass]);
    }

    #[test]
    fn keys_pressed_without_win_do_not_disarm_next_tap() {
        let v = run(&[OtherDown, down(Left), WinUp { side: Left }]);
        assert_eq!(v[2], Verdict::Tap);
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
        assert_eq!(v[2], Verdict::Pass);
        assert_eq!(v[4], Verdict::Tap);
    }
}
