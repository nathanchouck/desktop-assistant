//! Where each window was before the launcher first moved it, so "Restore
//! Window" can put it back (after several snaps in a row, back to where it
//! started). Pure.

use std::collections::HashMap;

use super::geometry::Rect;

/// Windows remembered at most; the oldest is forgotten first.
const MAX_WINDOWS: usize = 64;
/// A window still this close to where we put it hasn't been moved by you.
const SLACK: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Remembered {
    /// Where it was before the launcher moved it.
    pub frame: Rect,
    /// It was maximized then.
    pub maximized: bool,
    /// Where the launcher last put it.
    placed: Rect,
    order: u64,
}

#[derive(Debug, Default)]
pub struct RestoreMemory {
    windows: HashMap<isize, Remembered>,
    counter: u64,
}

impl RestoreMemory {
    /// Call before moving `window`, with where it is now. If it's still
    /// where we last put it, the place before that is kept, so Restore goes
    /// all the way back.
    pub fn before_move(&mut self, window: isize, frame: Rect, maximized: bool) {
        let ours = self
            .windows
            .get(&window)
            .is_some_and(|r| !maximized && r.placed.near(&frame, SLACK));
        if !ours {
            self.counter += 1;
            self.windows.insert(
                window,
                Remembered {
                    frame,
                    maximized,
                    placed: frame,
                    order: self.counter,
                },
            );
            self.forget_oldest();
        }
    }

    /// Call after moving `window`, with where it ended up.
    pub fn after_move(&mut self, window: isize, placed: Rect) {
        if let Some(r) = self.windows.get_mut(&window) {
            r.placed = placed;
        }
    }

    /// Where to put `window` back to, forgetting it.
    pub fn take(&mut self, window: isize) -> Option<Remembered> {
        self.windows.remove(&window)
    }

    fn forget_oldest(&mut self) {
        while self.windows.len() > MAX_WINDOWS {
            let oldest = self
                .windows
                .iter()
                .min_by_key(|(_, r)| r.order)
                .map(|(&w, _)| w);
            match oldest {
                Some(w) => {
                    self.windows.remove(&w);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Rect = Rect::new(100, 100, 900, 700);
    const LEFT: Rect = Rect::new(0, 0, 960, 1032);
    const RIGHT: Rect = Rect::new(960, 0, 1920, 1032);

    #[test]
    fn restore_goes_back_to_before_the_first_move() {
        let mut m = RestoreMemory::default();
        m.before_move(1, START, false);
        m.after_move(1, LEFT);
        // Snapped again from where we put it: the start is kept.
        m.before_move(1, LEFT, false);
        m.after_move(1, RIGHT);
        let r = m.take(1).unwrap();
        assert_eq!((r.frame, r.maximized), (START, false));
        assert_eq!(m.take(1), None);
    }

    #[test]
    fn moving_it_yourself_starts_over() {
        let mut m = RestoreMemory::default();
        m.before_move(1, START, false);
        m.after_move(1, LEFT);
        // You dragged it somewhere else, then snapped it again.
        let dragged = Rect::new(300, 300, 800, 800);
        m.before_move(1, dragged, false);
        assert_eq!(m.take(1).unwrap().frame, dragged);
    }

    #[test]
    fn remembers_a_maximized_window() {
        let mut m = RestoreMemory::default();
        m.before_move(7, Rect::new(0, 0, 1920, 1032), true);
        assert!(m.take(7).unwrap().maximized);
    }

    #[test]
    fn small_rounding_still_counts_as_ours() {
        let mut m = RestoreMemory::default();
        m.before_move(1, START, false);
        m.after_move(1, LEFT);
        m.before_move(1, Rect::new(1, 0, 959, 1032), false);
        assert_eq!(m.take(1).unwrap().frame, START);
    }

    #[test]
    fn forgets_the_oldest_windows() {
        let mut m = RestoreMemory::default();
        for w in 0..(MAX_WINDOWS as isize + 5) {
            m.before_move(w, START, false);
        }
        assert_eq!(m.windows.len(), MAX_WINDOWS);
        assert_eq!(m.take(0), None);
        assert!(m.take(MAX_WINDOWS as isize + 4).is_some());
    }
}
