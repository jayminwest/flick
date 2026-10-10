//! Pure geometry of a surface: where its window goes on the screen under the pointer, its
//! minimum size, and the frames of its parts inside the content view. No `AppKit`.

/// A rectangle. Screen frames use `AppKit` screen coordinates (origin bottom-left); content
/// frames are top-down (origin top-left of the flipped content view).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    fn contains(&self, (px, py): (f64, f64)) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// One display: its whole frame (for the pointer test) and the part outside the menu bar and
/// Dock (where the window goes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub frame: Rect,
    pub visible: Rect,
}

/// The smallest a surface may get: header, a little body, chips, notice and a multi-line
/// input still fit.
pub const MIN: Size = Size { w: 320.0, h: 260.0 };

/// A spec's minimum size, raised to `MIN`.
pub fn min_size(asked: Size) -> Size {
    Size { w: asked.w.max(MIN.w), h: asked.h.max(MIN.h) }
}

/// Where a window with frame `frame` goes when it shows. It stays where it is (moved fully
/// into the visible area) when its centre is on the screen under `mouse`; otherwise it is
/// centred on that screen. Its size keeps to `min` and to the visible area. Off every screen,
/// the pointer counts as on the first.
pub fn place(frame: Rect, min: Size, screens: &[Screen], mouse: (f64, f64)) -> Rect {
    let Some(screen) = screens.iter().find(|s| s.frame.contains(mouse)).or(screens.first()) else {
        return frame;
    };
    let area = screen.visible;
    let width = frame.w.max(min.w).min(area.w);
    let height = frame.h.max(min.h).min(area.h);
    let centre = (frame.x + frame.w / 2.0, frame.y + frame.h / 2.0);
    let (x, y) = if screen.frame.contains(centre) {
        (frame.x, frame.y)
    } else {
        (area.x + (area.w - width) / 2.0, area.y + (area.h - height) / 2.0)
    };
    let x = x.clamp(area.x, area.x + area.w - width);
    Rect::new(x, y.clamp(area.y, area.y + area.h - height), width, height)
}

const PAD: f64 = 14.0;
const GAP: f64 = 8.0;
pub const HEADER_H: f64 = 52.0;
const TITLE_TOP: f64 = 10.0;
const TITLE_H: f64 = 18.0;
const SUBTITLE_TOP: f64 = 29.0;
const SUBTITLE_H: f64 = 15.0;
const DOT: f64 = 8.0;
pub const CHIP_H: f64 = 22.0;
const CHIP_GAP: f64 = 6.0;
const NOTICE_H: f64 = 16.0;
/// Input heights: one line, or about four.
const SINGLE_H: f64 = 24.0;
const MULTI_H: f64 = 76.0;

/// What the content holds besides the fixed header and input.
#[derive(Clone, Copy, Debug)]
pub struct Parts<'a> {
    /// Each chip's natural width, left to right.
    pub chips: &'a [f64],
    pub notice: bool,
    pub multi: bool,
}

/// Frames of a surface's parts, top-down in a content view.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub title: Rect,
    pub subtitle: Rect,
    /// The status dot, right of the title.
    pub dot: Rect,
    /// The 1pt rule under the header.
    pub rule: Rect,
    /// The rows area between header and the bottom parts; height 0 when there is no room.
    pub body: Rect,
    pub notice: Option<Rect>,
    /// One per chip; `None` for a chip that does not fit on the row (it and all after it).
    pub chips: Vec<Option<Rect>>,
    /// The input box (its scroll view).
    pub input: Rect,
}

/// Lay out a content view of `size`.
pub fn layout(size: Size, parts: Parts) -> Layout {
    let inner = (size.w - 2.0 * PAD).max(0.0);
    let input_h = if parts.multi { MULTI_H } else { SINGLE_H };
    let input = Rect::new(PAD, size.h - PAD - input_h, inner, input_h);
    let mut bottom = input.y - GAP;

    let mut x = PAD;
    let mut fits = true;
    let chips_top = bottom - CHIP_H;
    let chips: Vec<Option<Rect>> = parts
        .chips
        .iter()
        .map(|&w| {
            let w = w.min(inner);
            fits = fits && x + w <= PAD + inner;
            let r = fits.then(|| Rect::new(x, chips_top, w, CHIP_H));
            x += w + CHIP_GAP;
            r
        })
        .collect();
    if !chips.is_empty() {
        bottom = chips_top - GAP;
    }
    let notice = parts.notice.then(|| Rect::new(PAD, bottom - NOTICE_H, inner, NOTICE_H));
    if let Some(n) = notice {
        bottom = n.y - GAP;
    }

    let text_w = (inner - DOT - GAP).max(0.0);
    Layout {
        title: Rect::new(PAD, TITLE_TOP, text_w, TITLE_H),
        subtitle: Rect::new(PAD, SUBTITLE_TOP, text_w, SUBTITLE_H),
        dot: Rect::new(size.w - PAD - DOT, TITLE_TOP + (TITLE_H - DOT) / 2.0, DOT, DOT),
        rule: Rect::new(0.0, HEADER_H, size.w, 1.0),
        body: Rect::new(0.0, HEADER_H + 1.0, size.w, (bottom - HEADER_H - 1.0).max(0.0)),
        notice,
        chips,
        input,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn screen(x: f64, y: f64, w: f64, h: f64) -> Screen {
        // A 25pt menu bar at the top.
        Screen { frame: Rect::new(x, y, w, h), visible: Rect::new(x, y, w, h - 25.0) }
    }

    const SCREENS: [Screen; 2] =
        [screen(0.0, 0.0, 1440.0, 900.0), screen(1440.0, 0.0, 1920.0, 1080.0)];
    const SMALL: Size = Size { w: 300.0, h: 200.0 };

    #[test]
    fn min_size_never_drops_below_the_floor() {
        assert_eq!(min_size(Size { w: 100.0, h: 900.0 }), Size { w: MIN.w, h: 900.0 });
        assert_eq!(min_size(Size { w: 500.0, h: 10.0 }), Size { w: 500.0, h: MIN.h });
    }

    #[test]
    fn a_window_on_the_pointers_screen_stays_put() {
        let f = Rect::new(100.0, 100.0, 500.0, 600.0);
        assert_eq!(place(f, SMALL, &SCREENS, (10.0, 10.0)), f);
        // The const helper, run once at run time so coverage sees it (mx-5a5c3f).
        assert_eq!(screen(0.0, 0.0, 1440.0, 900.0), SCREENS[0]);
    }

    #[test]
    fn a_window_on_another_screen_moves_to_the_middle_of_the_pointers() {
        let f = Rect::new(100.0, 100.0, 500.0, 600.0);
        let p = place(f, SMALL, &SCREENS, (2000.0, 500.0));
        assert_eq!(p, Rect::new(1440.0 + 710.0, (1055.0 - 600.0) / 2.0, 500.0, 600.0));
    }

    #[test]
    fn a_window_hanging_off_the_edge_is_pulled_in_and_sized_to_fit() {
        // Centre on screen 1, top past the menu bar and right edge past the screen.
        let f = Rect::new(1200.0, 500.0, 400.0, 380.0);
        assert_eq!(place(f, SMALL, &SCREENS, (5.0, 5.0)), Rect::new(1040.0, 495.0, 400.0, 380.0));
        // Taller than the screen: the visible height; smaller than min: min.
        let tall = Rect::new(0.0, -2000.0, 100.0, 5000.0);
        assert_eq!(place(tall, SMALL, &SCREENS, (5.0, 5.0)), Rect::new(0.0, 0.0, 300.0, 875.0));
    }

    #[test]
    fn the_pointer_off_every_screen_counts_as_the_first_and_no_screens_change_nothing() {
        let f = Rect::new(3000.0, 100.0, 400.0, 300.0);
        let p = place(f, SMALL, &SCREENS, (-50.0, -50.0));
        assert_eq!(p, Rect::new(520.0, 287.5, 400.0, 300.0));
        assert_eq!(place(f, SMALL, &[], (0.0, 0.0)), f);
    }

    #[test]
    fn single_line_layout_without_chips_or_notice() {
        let l =
            layout(Size { w: 400.0, h: 500.0 }, Parts { chips: &[], notice: false, multi: false });
        assert_eq!(l.input, Rect::new(14.0, 462.0, 372.0, 24.0));
        assert_eq!(l.body, Rect::new(0.0, 53.0, 400.0, 401.0));
        assert_eq!(l.title, Rect::new(14.0, 10.0, 356.0, 18.0));
        assert_eq!(l.subtitle, Rect::new(14.0, 29.0, 356.0, 15.0));
        assert_eq!(l.dot, Rect::new(378.0, 15.0, 8.0, 8.0));
        assert_eq!(l.rule, Rect::new(0.0, 52.0, 400.0, 1.0));
        assert_eq!(l.notice, None);
        assert!(l.chips.is_empty());
    }

    #[test]
    fn chips_and_notice_stack_above_a_multi_line_input_and_overflow_hides() {
        let parts = Parts { chips: &[100.0, 150.0, 120.0, 10.0], notice: true, multi: true };
        let l = layout(Size { w: 400.0, h: 500.0 }, parts);
        assert_eq!(l.input, Rect::new(14.0, 410.0, 372.0, 76.0));
        let top = 410.0 - 8.0 - 22.0;
        assert_eq!(
            l.chips,
            vec![
                Some(Rect::new(14.0, top, 100.0, 22.0)),
                Some(Rect::new(120.0, top, 150.0, 22.0)),
                None,
                // Small enough to fit, but it goes after a hidden one: hidden too.
                None,
            ]
        );
        assert_eq!(l.notice, Some(Rect::new(14.0, top - 8.0 - 16.0, 372.0, 16.0)));
        assert_eq!(l.body, Rect::new(0.0, 53.0, 400.0, top - 8.0 - 16.0 - 8.0 - 53.0));
    }

    #[test]
    fn a_chip_wider_than_the_row_is_cut_to_it_and_a_tiny_view_has_no_body() {
        let l = layout(
            Size { w: 100.0, h: 60.0 },
            Parts { chips: &[500.0], notice: false, multi: true },
        );
        assert_eq!(l.chips, vec![Some(Rect::new(14.0, -60.0, 72.0, 22.0))]);
        assert!(l.body.h.abs() < f64::EPSILON);
        let l = layout(Size { w: 0.0, h: 0.0 }, Parts { chips: &[], notice: false, multi: false });
        assert!(l.input.w.abs() < f64::EPSILON && l.title.w.abs() < f64::EPSILON);
    }
}
