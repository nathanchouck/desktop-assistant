//! Where the launcher sits on a monitor (pure, so it's tested on Linux).
//! Everything is in physical pixels; `scale` is the monitor's (1.5 at 150%).

/// A rectangle in physical pixels: a monitor's work area or the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }
}

/// The window's frame in `area` (a monitor's work area, without the
/// taskbar) for a window of `logical` (width, height) at `scale`:
/// - centered across, its top edge `top_fraction` of the way down, so it
///   grows downward when results appear;
/// - moved up when its bottom would go past the work area (a short screen,
///   or a large scale, with results showing), but never above its top;
/// - never bigger than the work area.
pub fn frame(area: Rect, scale: f64, logical: (f64, f64), top_fraction: f64) -> Rect {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let to_physical = |size: f64, room: u32| {
        let px = (size * scale).round();
        // `as` saturates: NaN and negatives become 0, then at least 1.
        (px as u32).clamp(1, room.max(1))
    };
    let width = to_physical(logical.0, area.width);
    let height = to_physical(logical.1, area.height);
    let x = i64::from(area.x) + (i64::from(area.width) - i64::from(width)) / 2;
    let anchor = i64::from(area.y) + (f64::from(area.height) * top_fraction).round() as i64;
    let y = anchor
        .min(area.bottom() - i64::from(height))
        .max(i64::from(area.y));
    Rect {
        x: clamp_i32(x),
        y: clamp_i32(y),
        width,
        height,
    }
}

fn clamp_i32(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOP: f64 = 0.22;

    fn area(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn at(frame: Rect) -> (i32, i32) {
        (frame.x, frame.y)
    }

    #[test]
    fn centers_on_primary_1080p() {
        let f = frame(area(0, 0, 1920, 1040), 1.0, (720.0, 64.0), TOP);
        assert_eq!(f, area(600, 229, 720, 64));
    }

    #[test]
    fn scales_to_the_monitor() {
        // 4K at 150%: the window is 1080 physical pixels wide.
        let f = frame(area(0, 0, 3840, 2100), 1.5, (720.0, 64.0), TOP);
        assert_eq!(f, area(1380, 462, 1080, 96));
        // The same logical window on a 100% monitor next to it.
        let f = frame(area(3840, 0, 1920, 1040), 1.0, (720.0, 64.0), TOP);
        assert_eq!(f, area(4440, 229, 720, 64));
    }

    #[test]
    fn respects_monitor_offset() {
        // A second monitor to the left of the primary, and one above it.
        assert_eq!(
            at(frame(area(-1920, 0, 1920, 1080), 1.0, (720.0, 64.0), TOP)),
            (-1320, 238)
        );
        assert_eq!(
            at(frame(area(0, -1440, 2560, 1400), 1.25, (720.0, 64.0), TOP)),
            (830, -1132)
        );
    }

    #[test]
    fn work_area_leaves_out_the_taskbar() {
        // Taskbar at the top: the work area starts 48 px down.
        let f = frame(area(0, 48, 1920, 1032), 1.0, (720.0, 64.0), TOP);
        assert_eq!(at(f), (600, 48 + 227));
    }

    #[test]
    fn moves_up_when_the_results_wouldnt_fit() {
        // 1366×768 at 150% with results showing (450 logical px tall).
        let short = area(0, 0, 1366, 728);
        let f = frame(short, 1.5, (720.0, 450.0), TOP);
        assert_eq!(f.height, 675);
        assert_eq!(f.y, 728 - 675);
        // The bar alone stays at the usual height.
        assert_eq!(frame(short, 1.5, (720.0, 64.0), TOP).y, 160);
    }

    #[test]
    fn never_bigger_than_the_work_area() {
        // 200% on a 1366 px wide screen: 1440 px wouldn't fit.
        let f = frame(area(100, 0, 1366, 600), 2.0, (720.0, 450.0), TOP);
        assert_eq!(f, area(100, 0, 1366, 600));
    }

    #[test]
    fn odd_scales_fall_back_to_100_percent() {
        let a = area(0, 0, 1920, 1040);
        let expected = frame(a, 1.0, (720.0, 64.0), TOP);
        assert_eq!(frame(a, 0.0, (720.0, 64.0), TOP), expected);
        assert_eq!(frame(a, f64::NAN, (720.0, 64.0), TOP), expected);
        assert_eq!(frame(a, -2.0, (720.0, 64.0), TOP), expected);
    }

    #[test]
    fn an_empty_area_still_gives_a_window() {
        let f = frame(area(0, 0, 0, 0), 1.0, (720.0, 64.0), TOP);
        assert_eq!((f.width, f.height), (1, 1));
    }
}
