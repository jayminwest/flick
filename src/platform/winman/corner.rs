//! Where `hide` parks a window: its top-left corner 1 px inside a bottom corner of its screen's
//! visible area, at the corner that touches fewer other screens (`AeroSpace`
//! `MacWindow.swift:139-171`, `layout/refresh.swift:162-186`). macOS pulls a window that is
//! wholly off screen back on; the 1 px edge stays put and lets the user drag the window back.
//! Pure geometry, so it has a 100% floor.

use super::WinId;
use crate::platform::Rect;

/// How far a parked window's origin may sit from its corner and still count as parked: apps
/// round frames.
const SLACK: f64 = 2.0;
/// The diagonal probe outweighs the two side probes.
const DIAGONAL: u32 = 10;
/// How far outside a corner the probes sit.
const OUT: f64 = 2.0;

/// A bottom corner of a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    BottomRight,
    BottomLeft,
}

/// One screen: the whole display and the part without the menu bar and Dock, in
/// Accessibility coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub frame: Rect,
    pub visible: Rect,
}

/// The corner to park screen `frame`'s windows at. Probes sit just outside each bottom
/// corner: beside it, below it and diagonally out (which counts `DIAGONAL` times); the corner
/// whose probes land on fewer of `screens` wins, a tie goes to bottom-right.
pub fn corner(frame: Rect, screens: &[Rect]) -> Corner {
    let bottom = frame.y + frame.h;
    let (x_in, y_in) = (frame.w * 0.1, frame.h * 0.1);
    let score = |edge: f64, inward: f64, outward: f64| {
        hits(screens, (edge + outward, bottom - y_in))
            + hits(screens, (edge + inward, bottom + OUT))
            + DIAGONAL * hits(screens, (edge + outward, bottom + OUT))
    };
    let right = score(frame.x + frame.w, -x_in, OUT);
    let left = score(frame.x, x_in, -OUT);
    if left < right { Corner::BottomLeft } else { Corner::BottomRight }
}

/// The top-left origin that parks a `w`-wide window at `corner` of `visible`.
pub fn parked(corner: Corner, visible: Rect, w: f64) -> (f64, f64) {
    let y = visible.y + visible.h - 1.0;
    match corner {
        Corner::BottomRight => (visible.x + visible.w - 1.0, y),
        Corner::BottomLeft => (visible.x + 1.0 - w, y),
    }
}

/// Each window `(id, frame)` parked at the corner of its screen (`screen_of`), size kept.
/// Empty without screens.
pub fn park(windows: &[(WinId, Rect)], screens: &[Area]) -> Vec<(WinId, Rect)> {
    let frames: Vec<Rect> = screens.iter().map(|s| s.frame).collect();
    windows
        .iter()
        .filter_map(|&(id, f)| {
            let screen = screens[screen_of(f, &frames)?];
            let (x, y) = parked(corner(screen.frame, &frames), screen.visible, f.w);
            Some((id, Rect { x, y, ..f }))
        })
        .collect()
}

/// The frame for a window at `frame` that sits parked at a bottom corner of one of `screens`
/// (left there by a Flick that was killed, or by another window manager): centred on that
/// screen, the one its 1 px edge is on. None for any other window.
pub fn rescue(frame: Rect, screens: &[Area]) -> Option<Rect> {
    let at = |s: &&Area| {
        [Corner::BottomRight, Corner::BottomLeft].into_iter().any(|c| {
            let (x, y) = parked(c, s.visible, frame.w);
            (frame.x - x).abs() <= SLACK && (frame.y - y).abs() <= SLACK
        })
    };
    screens.iter().find(at).map(|s| centred(frame, s.visible))
}

/// `frame` centred on `visible`, shrunk to fit it.
pub fn centred(frame: Rect, visible: Rect) -> Rect {
    let (w, h) = (frame.w.min(visible.w), frame.h.min(visible.h));
    Rect { x: visible.x + (visible.w - w) / 2.0, y: visible.y + (visible.h - h) / 2.0, w, h }
}

/// The index of the screen that holds `frame`'s centre, else of the nearest one. None without
/// screens.
pub fn screen_of(frame: Rect, screens: &[Rect]) -> Option<usize> {
    let c = (frame.x + frame.w / 2.0, frame.y + frame.h / 2.0);
    let far = |s: &Rect| distance(c, *s);
    screens.iter().position(|s| contains(*s, c)).or_else(|| {
        (0..screens.len()).min_by(|&a, &b| far(&screens[a]).total_cmp(&far(&screens[b])))
    })
}

fn hits(screens: &[Rect], p: (f64, f64)) -> u32 {
    u32::from(screens.iter().any(|s| contains(*s, p)))
}

fn contains(r: Rect, (x, y): (f64, f64)) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

/// The squared distance from `p` to the nearest point of `r`.
fn distance((x, y): (f64, f64), r: Rect) -> f64 {
    let dx = (r.x - x).max(x - (r.x + r.w)).max(0.0);
    let dy = (r.y - y).max(y - (r.y + r.h)).max(0.0);
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    /// A screen with a 25 px menu bar.
    fn area(frame: Rect) -> Area {
        Area { frame, visible: Rect { y: frame.y + 25.0, h: frame.h - 25.0, ..frame } }
    }

    const LAPTOP: Rect = Rect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 };

    #[test]
    fn a_lone_screen_parks_bottom_right() {
        assert_eq!(corner(LAPTOP, &[LAPTOP]), Corner::BottomRight);
    }

    #[test]
    fn a_screen_to_the_right_or_below_right_moves_parking_left() {
        let right = rect(1440.0, 0.0, 1920.0, 1080.0);
        assert_eq!(corner(LAPTOP, &[LAPTOP, right]), Corner::BottomLeft);
        // The laptop ends above the right screen's bottom corners: neither touches it.
        assert_eq!(corner(right, &[LAPTOP, right]), Corner::BottomRight);
        let below_right = rect(1440.0, 900.0, 1920.0, 1080.0);
        assert_eq!(corner(LAPTOP, &[LAPTOP, below_right]), Corner::BottomLeft);
    }

    #[test]
    fn screens_on_both_sides_tie_to_bottom_right() {
        let left = rect(-1920.0, 0.0, 1920.0, 900.0);
        let right = rect(1440.0, 0.0, 1920.0, 900.0);
        assert_eq!(corner(LAPTOP, &[left, LAPTOP, right]), Corner::BottomRight);
        // A screen below counts against both corners the same.
        let below = rect(0.0, 900.0, 1440.0, 900.0);
        assert_eq!(corner(LAPTOP, &[LAPTOP, below]), Corner::BottomRight);
    }

    #[test]
    fn parking_leaves_one_pixel_inside_the_visible_area() {
        let v = area(LAPTOP).visible;
        assert_eq!(parked(Corner::BottomRight, v, 800.0), (1439.0, 899.0));
        assert_eq!(parked(Corner::BottomLeft, v, 800.0), (-799.0, 899.0));
    }

    #[test]
    fn park_keeps_sizes_and_skips_windows_without_screens() {
        let screens = [area(LAPTOP), area(rect(1440.0, 0.0, 1920.0, 1080.0))];
        let on_laptop = rect(100.0, 100.0, 800.0, 600.0);
        let on_right = rect(1600.0, 100.0, 800.0, 600.0);
        assert_eq!(
            park(&[(1, on_laptop), (2, on_right)], &screens),
            [(1, rect(-799.0, 899.0, 800.0, 600.0)), (2, rect(3359.0, 1079.0, 800.0, 600.0))]
        );
        assert!(park(&[(1, on_laptop)], &[]).is_empty());
    }

    #[test]
    fn a_window_off_every_screen_belongs_to_the_nearest() {
        let screens = [LAPTOP, rect(1440.0, 0.0, 1920.0, 1080.0)];
        assert_eq!(screen_of(rect(5000.0, 0.0, 10.0, 10.0), &screens), Some(1));
        assert_eq!(screen_of(rect(-500.0, 2000.0, 10.0, 10.0), &screens), Some(0));
        assert_eq!(screen_of(rect(0.0, 0.0, 10.0, 10.0), &[]), None);
    }

    #[test]
    fn only_a_parked_window_is_rescued_onto_its_screen() {
        let screens = [area(LAPTOP), area(rect(1440.0, 0.0, 1920.0, 1080.0))];
        let centred_on_laptop = rect(320.0, 162.5, 800.0, 600.0);
        assert_eq!(rescue(rect(1439.0, 899.0, 800.0, 600.0), &screens), Some(centred_on_laptop));
        assert_eq!(rescue(rect(-800.5, 900.0, 800.0, 600.0), &screens), Some(centred_on_laptop));
        let on_right = rescue(rect(3359.0, 1079.0, 800.0, 600.0), &screens);
        assert_eq!(on_right, Some(rect(1440.0 + 560.0, 25.0 + 227.5, 800.0, 600.0)));
        assert_eq!(rescue(rect(100.0, 100.0, 800.0, 600.0), &screens), None);
        assert_eq!(rescue(rect(1439.0, 895.0, 800.0, 600.0), &screens), None);
        assert_eq!(rescue(rect(1439.0, 899.0, 800.0, 600.0), &[]), None);
    }

    #[test]
    fn centring_shrinks_a_window_larger_than_the_screen() {
        let v = area(LAPTOP).visible;
        assert_eq!(centred(rect(0.0, 0.0, 3000.0, 400.0), v), rect(0.0, 262.5, 1440.0, 400.0));
        assert_eq!(centred(rect(0.0, 0.0, 400.0, 3000.0), v), rect(520.0, 25.0, 400.0, 875.0));
    }
}
