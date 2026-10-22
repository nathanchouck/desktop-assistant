//! What the picker does with each click and key while it waits: the left
//! button picks (its press and release are both kept from the app under
//! the cursor), Enter picks where the cursor is, a right click or Esc
//! cancels. Pure, so it's tested on any OS; `win.rs` feeds it from
//! low-level hooks.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    LeftDown {
        x: i32,
        y: i32,
    },
    LeftUp,
    RightDown,
    RightUp,
    /// Enter, with the cursor's position.
    Enter {
        x: i32,
        y: i32,
    },
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// Let it through to the app.
    Pass,
    /// Keep it from the app.
    Swallow,
    /// Keep it from the app; picking is over.
    Finish,
}

#[derive(Debug, Default)]
pub struct Picking {
    point: Option<(i32, i32)>,
    cancelled: bool,
    /// The button release still to swallow before finishing.
    waiting_for: Option<Input>,
}

impl Picking {
    pub fn on(&mut self, input: Input) -> Reply {
        if let Some(up) = self.waiting_for {
            if input == up {
                self.waiting_for = None;
                return Reply::Finish;
            }
            // Between press and release nothing else counts.
            return Reply::Pass;
        }
        match input {
            Input::LeftDown { x, y } => {
                self.point = Some((x, y));
                self.waiting_for = Some(Input::LeftUp);
                Reply::Swallow
            }
            Input::RightDown => {
                self.cancelled = true;
                self.waiting_for = Some(Input::RightUp);
                Reply::Swallow
            }
            Input::Enter { x, y } => {
                self.point = Some((x, y));
                Reply::Finish
            }
            Input::Escape => {
                self.cancelled = true;
                Reply::Finish
            }
            Input::LeftUp | Input::RightUp => Reply::Pass,
        }
    }

    /// Where to read the color, unless picking was cancelled.
    pub fn point(&self) -> Option<(i32, i32)> {
        if self.cancelled {
            None
        } else {
            self.point
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_picks_and_both_halves_are_swallowed() {
        let mut p = Picking::default();
        assert_eq!(p.on(Input::LeftDown { x: 10, y: -20 }), Reply::Swallow);
        assert_eq!(p.on(Input::RightDown), Reply::Pass);
        assert_eq!(p.on(Input::LeftUp), Reply::Finish);
        assert_eq!(p.point(), Some((10, -20)));
    }

    #[test]
    fn a_stray_release_from_before_is_let_through() {
        let mut p = Picking::default();
        assert_eq!(p.on(Input::LeftUp), Reply::Pass);
        assert_eq!(p.point(), None);
    }

    #[test]
    fn enter_picks_where_the_cursor_is() {
        let mut p = Picking::default();
        assert_eq!(p.on(Input::Enter { x: 5, y: 6 }), Reply::Finish);
        assert_eq!(p.point(), Some((5, 6)));
    }

    #[test]
    fn escape_and_right_click_cancel() {
        let mut p = Picking::default();
        assert_eq!(p.on(Input::Escape), Reply::Finish);
        assert_eq!(p.point(), None);

        let mut p = Picking::default();
        assert_eq!(p.on(Input::RightDown), Reply::Swallow);
        assert_eq!(p.on(Input::RightUp), Reply::Finish);
        assert_eq!(p.point(), None);
    }
}
