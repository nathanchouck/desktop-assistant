//! Rectangle math for window layouts, in physical screen pixels. Pure, so
//! every layout is tested on any OS.
//!
//! "Frame" means the part of a window you see. On Windows 10 and 11 a
//! window's rectangle is larger: DWM adds invisible resize borders on the
//! left, right and bottom (`Insets`). Layouts are computed for the frame,
//! then grown by the insets, so snapped windows meet edge to edge.

/// A rectangle; `right` and `bottom` are exclusive, as in Win32 `RECT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    fn at(left: i32, top: i32, width: i32, height: i32) -> Self {
        Self::new(left, top, left + width, top + height)
    }

    /// Whether two rectangles are the same give or take `slack` pixels on
    /// each edge (apps round sizes to their own steps, e.g. terminals).
    pub fn near(&self, other: &Rect, slack: i32) -> bool {
        (self.left - other.left).abs() <= slack
            && (self.top - other.top).abs() <= slack
            && (self.right - other.right).abs() <= slack
            && (self.bottom - other.bottom).abs() <= slack
    }
}

/// How far a window's rectangle reaches past its visible frame on each
/// side (DWM's invisible borders, about 7 px at 100% scale). Measured
/// in `win.rs`; elsewhere only the tests use it.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Larger "borders" than this aren't borders: the frame bounds were
/// wrong (some apps draw their own frame), so none are assumed.
#[cfg_attr(not(windows), allow(dead_code))]
const MAX_INSET: i32 = 64;

#[cfg_attr(not(windows), allow(dead_code))]
impl Insets {
    /// The insets between a window's rectangle and its visible frame.
    pub fn between(window: Rect, frame: Rect) -> Self {
        let insets = Self {
            left: frame.left - window.left,
            top: frame.top - window.top,
            right: window.right - frame.right,
            bottom: window.bottom - frame.bottom,
        };
        let sides = [insets.left, insets.top, insets.right, insets.bottom];
        if sides.iter().all(|&s| (0..=MAX_INSET).contains(&s)) {
            insets
        } else {
            Self::default()
        }
    }

    /// The window rectangle whose visible frame is `frame`.
    pub fn grow(&self, frame: Rect) -> Rect {
        Rect::new(
            frame.left - self.left,
            frame.top - self.top,
            frame.right + self.right,
            frame.bottom + self.bottom,
        )
    }
}

/// Where a layout puts a window, as a share of the work area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    LeftThird,
    CenterThird,
    RightThird,
    LeftTwoThirds,
    RightTwoThirds,
    /// 90% of the work area, centered.
    AlmostMaximize,
    /// The window's own size, centered.
    Center,
}

/// "Almost maximize" leaves this share of the work area around the window.
const ALMOST_PERCENT: i32 = 90;

/// The frame for `region` on a monitor whose work area (the screen minus
/// the taskbar) is `work`. `frame` is the window's current frame; only
/// `Center` uses it.
pub fn region(region: Region, work: Rect, frame: Rect) -> Rect {
    let (l, t, r, b) = (work.left, work.top, work.right, work.bottom);
    // Split points from the work area's edges, so neighbors share an edge
    // with no gap or overlap whatever the rounding.
    let x = |n: i32, d: i32| l + work.width() * n / d;
    let y = |n: i32, d: i32| t + work.height() * n / d;
    match region {
        Region::LeftHalf => Rect::new(l, t, x(1, 2), b),
        Region::RightHalf => Rect::new(x(1, 2), t, r, b),
        Region::TopHalf => Rect::new(l, t, r, y(1, 2)),
        Region::BottomHalf => Rect::new(l, y(1, 2), r, b),
        Region::LeftThird => Rect::new(l, t, x(1, 3), b),
        Region::CenterThird => Rect::new(x(1, 3), t, x(2, 3), b),
        Region::RightThird => Rect::new(x(2, 3), t, r, b),
        Region::LeftTwoThirds => Rect::new(l, t, x(2, 3), b),
        Region::RightTwoThirds => Rect::new(x(1, 3), t, r, b),
        Region::AlmostMaximize => centered(
            work,
            work.width() * ALMOST_PERCENT / 100,
            work.height() * ALMOST_PERCENT / 100,
        ),
        Region::Center => centered(work, frame.width(), frame.height()),
    }
}

/// A `width` × `height` rectangle centered in `area` (shrunk to fit).
fn centered(area: Rect, width: i32, height: i32) -> Rect {
    let width = width.clamp(1, area.width().max(1));
    let height = height.clamp(1, area.height().max(1));
    Rect::at(
        area.left + (area.width() - width) / 2,
        area.top + (area.height() - height) / 2,
        width,
        height,
    )
}

/// Where a window that can't be resized goes: its own size, centered in
/// the region it was meant to fill.
pub fn fixed_size(target: Rect, frame: Rect) -> Rect {
    centered(target, frame.width(), frame.height())
}

/// The next monitor's work area after `current`, going left to right
/// (then top to bottom) and wrapping around. `None` with one monitor.
pub fn next_monitor(works: &[Rect], current: Rect) -> Option<Rect> {
    let mut order: Vec<Rect> = works.to_vec();
    order.sort_by_key(|w| (w.left, w.top));
    order.dedup();
    if order.len() < 2 {
        return None;
    }
    let here = order.iter().position(|w| *w == current)?;
    Some(order[(here + 1) % order.len()])
}

/// `frame` moved from one work area to another, keeping its position and
/// size relative to the work area: a window on the left half of one
/// monitor lands on the left half of the next, whatever their sizes.
pub fn move_between(frame: Rect, from: Rect, to: Rect) -> Rect {
    if from.width() <= 0 || from.height() <= 0 {
        return centered(to, frame.width(), frame.height());
    }
    let scale_x = f64::from(to.width()) / f64::from(from.width());
    let scale_y = f64::from(to.height()) / f64::from(from.height());
    let map_x = |v: i32| to.left + (f64::from(v - from.left) * scale_x).round() as i32;
    let map_y = |v: i32| to.top + (f64::from(v - from.top) * scale_y).round() as i32;
    let moved = Rect::new(
        map_x(frame.left),
        map_y(frame.top),
        map_x(frame.right),
        map_y(frame.bottom),
    );
    clamp_into(moved, to)
}

/// `rect` shifted (and if need be shrunk) to lie inside `area`.
pub fn clamp_into(rect: Rect, area: Rect) -> Rect {
    let width = rect.width().clamp(1, area.width().max(1));
    let height = rect.height().clamp(1, area.height().max(1));
    // min then max rather than clamp: an empty area must not panic.
    let left = rect.left.min(area.right - width).max(area.left);
    let top = rect.top.min(area.bottom - height).max(area.top);
    Rect::at(left, top, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1920×1080 monitor with a 48 px taskbar at the bottom.
    const WORK: Rect = Rect::new(0, 0, 1920, 1032);
    const FRAME: Rect = Rect::new(100, 100, 900, 700);

    fn at(region_: Region) -> Rect {
        region(region_, WORK, FRAME)
    }

    #[test]
    fn halves_split_the_work_area() {
        assert_eq!(at(Region::LeftHalf), Rect::new(0, 0, 960, 1032));
        assert_eq!(at(Region::RightHalf), Rect::new(960, 0, 1920, 1032));
        assert_eq!(at(Region::TopHalf), Rect::new(0, 0, 1920, 516));
        assert_eq!(at(Region::BottomHalf), Rect::new(0, 516, 1920, 1032));
    }

    #[test]
    fn thirds_meet_without_gaps() {
        let left = at(Region::LeftThird);
        let center = at(Region::CenterThird);
        let right = at(Region::RightThird);
        assert_eq!(left, Rect::new(0, 0, 640, 1032));
        assert_eq!(center, Rect::new(640, 0, 1280, 1032));
        assert_eq!(right, Rect::new(1280, 0, 1920, 1032));
        assert_eq!(at(Region::LeftTwoThirds), Rect::new(0, 0, 1280, 1032));
        assert_eq!(at(Region::RightTwoThirds), Rect::new(640, 0, 1920, 1032));

        // A width that doesn't divide by three: still edge to edge.
        let odd = Rect::new(-1000, 0, 0, 800);
        let l = region(Region::LeftThird, odd, FRAME);
        let c = region(Region::CenterThird, odd, FRAME);
        let r = region(Region::RightThird, odd, FRAME);
        assert_eq!((l.left, l.right), (-1000, -667));
        assert_eq!(l.right, c.left);
        assert_eq!(c.right, r.left);
        assert_eq!(r.right, 0);
    }

    #[test]
    fn layouts_follow_a_monitor_left_of_the_main_one() {
        let work = Rect::new(-2560, 40, 0, 1440);
        assert_eq!(
            region(Region::RightHalf, work, FRAME),
            Rect::new(-1280, 40, 0, 1440)
        );
        assert_eq!(
            region(Region::BottomHalf, work, FRAME),
            Rect::new(-2560, 740, 0, 1440)
        );
    }

    #[test]
    fn almost_maximize_and_center() {
        assert_eq!(at(Region::AlmostMaximize), Rect::new(96, 52, 1824, 980));
        // Center keeps the size.
        let c = at(Region::Center);
        assert_eq!((c.width(), c.height()), (800, 600));
        assert_eq!(c, Rect::new(560, 216, 1360, 816));
        // A window larger than the screen is shrunk to fit.
        let huge = region(Region::Center, WORK, Rect::new(0, 0, 4000, 3000));
        assert_eq!(huge, WORK);
    }

    #[test]
    fn insets_grow_the_frame_into_the_window_rect() {
        // Windows 11 at 100%: 7 px invisible borders, none at the top.
        let window = Rect::new(93, 100, 907, 707);
        let insets = Insets::between(window, FRAME);
        assert_eq!(
            insets,
            Insets {
                left: 7,
                top: 0,
                right: 7,
                bottom: 7
            }
        );
        let left_half = at(Region::LeftHalf);
        assert_eq!(insets.grow(left_half), Rect::new(-7, 0, 967, 1039));
        // Growing the frame we measured gives back the window rect.
        assert_eq!(insets.grow(FRAME), window);
    }

    #[test]
    fn odd_frame_bounds_mean_no_insets() {
        // A frame outside the window rect, or far inside it.
        let window = Rect::new(0, 0, 800, 600);
        assert_eq!(
            Insets::between(window, Rect::new(-5, 0, 800, 600)),
            Insets::default()
        );
        assert_eq!(
            Insets::between(window, Rect::new(200, 0, 800, 600)),
            Insets::default()
        );
    }

    #[test]
    fn fixed_size_windows_are_centered_in_the_region() {
        let dialog = Rect::new(0, 0, 400, 300);
        let target = at(Region::LeftHalf);
        assert_eq!(fixed_size(target, dialog), Rect::new(280, 366, 680, 666));
    }

    #[test]
    fn next_monitor_goes_left_to_right_and_wraps() {
        let left = Rect::new(-1920, 0, 0, 1040);
        let main = Rect::new(0, 0, 2560, 1400);
        let right = Rect::new(2560, 200, 4480, 1240);
        let works = [main, right, left];
        assert_eq!(next_monitor(&works, left), Some(main));
        assert_eq!(next_monitor(&works, main), Some(right));
        assert_eq!(next_monitor(&works, right), Some(left));
        assert_eq!(next_monitor(&[main], main), None);
        assert_eq!(next_monitor(&works, Rect::new(1, 2, 3, 4)), None);
    }

    #[test]
    fn moving_keeps_relative_position_and_size() {
        let from = Rect::new(0, 0, 1920, 1080);
        let to = Rect::new(1920, 0, 4480, 1440);
        // The left half of a 1080p monitor lands on the left half of a
        // 1440p one.
        let left_half = Rect::new(0, 0, 960, 1080);
        assert_eq!(
            move_between(left_half, from, to),
            Rect::new(1920, 0, 3200, 1440)
        );
        // Something in the middle stays in the middle.
        let middle = Rect::new(480, 270, 1440, 810);
        assert_eq!(
            move_between(middle, from, to),
            Rect::new(2560, 360, 3840, 1080)
        );
        // Back again.
        assert_eq!(
            move_between(Rect::new(1920, 0, 3200, 1440), to, from),
            left_half
        );
    }

    #[test]
    fn moving_keeps_the_window_on_screen() {
        let from = Rect::new(0, 0, 1920, 1080);
        let to = Rect::new(1920, 0, 3840, 1080);
        // Hanging off the right edge: pulled back inside.
        let off = Rect::new(1800, 100, 2400, 600);
        let moved = move_between(off, from, to);
        assert_eq!(moved.right, 3840);
        assert_eq!(moved.width(), 600);
        // A degenerate source area: just center it.
        let moved = move_between(FRAME, Rect::default(), to);
        assert_eq!((moved.width(), moved.height()), (800, 600));
    }

    #[test]
    fn clamping() {
        let area = Rect::new(0, 0, 100, 100);
        assert_eq!(
            clamp_into(Rect::new(-10, 90, 40, 140), area),
            Rect::new(0, 50, 50, 100)
        );
        assert_eq!(clamp_into(Rect::new(0, 0, 500, 500), area), area);
        // An empty area gives a 1 px rect at its corner rather than a panic.
        let empty = Rect::new(10, 10, 10, 10);
        assert_eq!(clamp_into(area, empty), Rect::new(10, 10, 11, 11));
    }

    #[test]
    fn near_allows_a_little_rounding() {
        let a = Rect::new(0, 0, 100, 100);
        assert!(a.near(&Rect::new(1, -1, 101, 99), 2));
        assert!(!a.near(&Rect::new(0, 0, 110, 100), 2));
    }
}
