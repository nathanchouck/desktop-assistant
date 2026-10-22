//! The window commands: names, keywords, and where each puts the window.
//! Pure, so the table and the targets are tested on any OS.

use super::geometry::{self, Rect, Region};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    LeftThird,
    CenterThird,
    RightThird,
    LeftTwoThirds,
    RightTwoThirds,
    Maximize,
    AlmostMaximize,
    Center,
    Restore,
    NextMonitor,
}

const ID_PREFIX: &str = "layout:";

impl Layout {
    pub const ALL: [Self; 14] = [
        Self::LeftHalf,
        Self::RightHalf,
        Self::TopHalf,
        Self::BottomHalf,
        Self::LeftThird,
        Self::CenterThird,
        Self::RightThird,
        Self::LeftTwoThirds,
        Self::RightTwoThirds,
        Self::Maximize,
        Self::AlmostMaximize,
        Self::Center,
        Self::Restore,
        Self::NextMonitor,
    ];

    /// The part after `layout:`.
    fn slug(self) -> &'static str {
        match self {
            Self::LeftHalf => "left-half",
            Self::RightHalf => "right-half",
            Self::TopHalf => "top-half",
            Self::BottomHalf => "bottom-half",
            Self::LeftThird => "left-third",
            Self::CenterThird => "center-third",
            Self::RightThird => "right-third",
            Self::LeftTwoThirds => "left-two-thirds",
            Self::RightTwoThirds => "right-two-thirds",
            Self::Maximize => "maximize",
            Self::AlmostMaximize => "almost-maximize",
            Self::Center => "center",
            Self::Restore => "restore",
            Self::NextMonitor => "next-monitor",
        }
    }

    pub fn id(self) -> String {
        format!("{ID_PREFIX}{}", self.slug())
    }

    pub fn from_id(id: &str) -> Option<Self> {
        let slug = id.strip_prefix(ID_PREFIX)?;
        Self::ALL.into_iter().find(|l| l.slug() == slug)
    }

    pub fn owns(id: &str) -> bool {
        id.starts_with(ID_PREFIX)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::LeftHalf => "Left Half",
            Self::RightHalf => "Right Half",
            Self::TopHalf => "Top Half",
            Self::BottomHalf => "Bottom Half",
            Self::LeftThird => "Left Third",
            Self::CenterThird => "Center Third",
            Self::RightThird => "Right Third",
            Self::LeftTwoThirds => "Left Two Thirds",
            Self::RightTwoThirds => "Right Two Thirds",
            Self::Maximize => "Maximize Window",
            Self::AlmostMaximize => "Almost Maximize",
            Self::Center => "Center Window",
            Self::Restore => "Restore Window",
            Self::NextMonitor => "Move to Next Monitor",
        }
    }

    /// Other words that should find it. "snap" and "window" find them all.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::LeftHalf => &["snap left", "window left", "split left", "tile left"],
            Self::RightHalf => &["snap right", "window right", "split right", "tile right"],
            Self::TopHalf => &["snap top", "window top", "upper half"],
            Self::BottomHalf => &["snap bottom", "window bottom", "lower half"],
            Self::LeftThird => &["snap left third", "first third"],
            Self::CenterThird => &["snap center third", "middle third"],
            Self::RightThird => &["snap right third", "last third"],
            Self::LeftTwoThirds => &["snap left two thirds", "first two thirds"],
            Self::RightTwoThirds => &["snap right two thirds", "last two thirds"],
            Self::Maximize => &["maximize", "full screen", "fill screen", "snap maximize"],
            Self::AlmostMaximize => &["almost maximize", "nearly full", "snap almost"],
            Self::Center => &["center", "centre", "middle", "snap center"],
            Self::Restore => &["restore", "unmaximize", "undo snap", "original size"],
            Self::NextMonitor => &[
                "next monitor",
                "next display",
                "next screen",
                "other monitor",
                "move window",
                "snap monitor",
            ],
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::LeftHalf => "layout-left",
            Self::RightHalf => "layout-right",
            Self::TopHalf => "layout-top",
            Self::BottomHalf => "layout-bottom",
            Self::LeftThird => "layout-left-third",
            Self::CenterThird => "layout-center-third",
            Self::RightThird => "layout-right-third",
            Self::LeftTwoThirds => "layout-left-two-thirds",
            Self::RightTwoThirds => "layout-right-two-thirds",
            Self::Maximize => "maximize",
            Self::AlmostMaximize => "almost-maximize",
            Self::Center => "layout-center",
            Self::Restore => "restore",
            Self::NextMonitor => "monitor",
        }
    }

    /// The share of the screen it fills, for the layouts that are one.
    pub fn region(self) -> Option<Region> {
        Some(match self {
            Self::LeftHalf => Region::LeftHalf,
            Self::RightHalf => Region::RightHalf,
            Self::TopHalf => Region::TopHalf,
            Self::BottomHalf => Region::BottomHalf,
            Self::LeftThird => Region::LeftThird,
            Self::CenterThird => Region::CenterThird,
            Self::RightThird => Region::RightThird,
            Self::LeftTwoThirds => Region::LeftTwoThirds,
            Self::RightTwoThirds => Region::RightTwoThirds,
            Self::AlmostMaximize => Region::AlmostMaximize,
            Self::Center => Region::Center,
            Self::Maximize | Self::Restore | Self::NextMonitor => return None,
        })
    }
}

/// Where a window is: its visible frame (not maximized or minimized),
/// its monitor's work area, and every monitor's work area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub frame: Rect,
    pub work: Rect,
    pub works: Vec<Rect>,
    /// Windows without a sizing border (dialogs) keep their size.
    pub resizable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoTarget {
    OneMonitor,
    /// Maximize and Restore aren't a rectangle.
    NotARegion,
}

/// The frame `layout` gives a window placed as `at`.
pub fn target(layout: Layout, at: &Placement) -> Result<Rect, NoTarget> {
    if layout == Layout::NextMonitor {
        let next = geometry::next_monitor(&at.works, at.work).ok_or(NoTarget::OneMonitor)?;
        let moved = geometry::move_between(at.frame, at.work, next);
        return Ok(if at.resizable {
            moved
        } else {
            let kept = Rect::new(
                moved.left,
                moved.top,
                moved.left + at.frame.width(),
                moved.top + at.frame.height(),
            );
            geometry::clamp_into(kept, next)
        });
    }
    let region = layout.region().ok_or(NoTarget::NotARegion)?;
    let wanted = geometry::region(region, at.work, at.frame);
    Ok(if at.resizable || region == Region::Center {
        wanted
    } else {
        geometry::fixed_size(wanted, at.frame)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect::new(0, 0, 1920, 1032);

    fn placed(frame: Rect, resizable: bool) -> Placement {
        Placement {
            frame,
            work: WORK,
            works: vec![WORK],
            resizable,
        }
    }

    #[test]
    fn ids_round_trip() {
        for layout in Layout::ALL {
            assert_eq!(Layout::from_id(&layout.id()), Some(layout));
            assert!(Layout::owns(&layout.id()));
        }
        assert_eq!(Layout::from_id("layout:sideways"), None);
        assert_eq!(Layout::from_id("left-half"), None);
        assert!(!Layout::owns("sys:lock"));
    }

    #[test]
    fn every_icon_is_drawn() {
        for layout in Layout::ALL {
            let name = layout.glyph();
            assert!(
                crate::features::system::matching::glyph_exists(name),
                "{name}"
            );
        }
    }

    #[test]
    fn every_layout_but_three_is_a_region() {
        let not_regions: Vec<_> = Layout::ALL
            .into_iter()
            .filter(|l| l.region().is_none())
            .collect();
        assert_eq!(
            not_regions,
            [Layout::Maximize, Layout::Restore, Layout::NextMonitor]
        );
    }

    #[test]
    fn targets_for_resizable_windows() {
        let at = placed(Rect::new(100, 100, 900, 700), true);
        assert_eq!(
            target(Layout::LeftHalf, &at),
            Ok(Rect::new(0, 0, 960, 1032))
        );
        assert_eq!(
            target(Layout::RightTwoThirds, &at),
            Ok(Rect::new(640, 0, 1920, 1032))
        );
        assert_eq!(target(Layout::Maximize, &at), Err(NoTarget::NotARegion));
    }

    #[test]
    fn fixed_size_windows_keep_their_size() {
        let at = placed(Rect::new(0, 0, 400, 300), false);
        let t = target(Layout::RightHalf, &at).unwrap();
        assert_eq!((t.width(), t.height()), (400, 300));
        assert_eq!(t.left, 960 + 280);
        let c = target(Layout::Center, &at).unwrap();
        assert_eq!(c, Rect::new(760, 366, 1160, 666));
    }

    #[test]
    fn next_monitor_needs_a_second_one() {
        let at = placed(Rect::new(0, 0, 960, 1032), true);
        assert_eq!(target(Layout::NextMonitor, &at), Err(NoTarget::OneMonitor));

        let second = Rect::new(1920, 0, 3840, 1032);
        let two = Placement {
            works: vec![WORK, second],
            ..at
        };
        assert_eq!(
            target(Layout::NextMonitor, &two),
            Ok(Rect::new(1920, 0, 2880, 1032))
        );
    }

    #[test]
    fn next_monitor_keeps_a_dialogs_size() {
        let small = Rect::new(1920, 0, 2880, 540);
        let at = Placement {
            frame: Rect::new(100, 100, 600, 500),
            work: WORK,
            works: vec![WORK, small],
            resizable: false,
        };
        let t = target(Layout::NextMonitor, &at).unwrap();
        assert_eq!((t.width(), t.height()), (500, 400));
        assert!(t.left >= small.left && t.bottom <= small.bottom);
    }
}
