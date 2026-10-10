//! Where the panel's read-only text block is scrolled to (flick-0b6a). Pure: offsets in
//! points from the top of the text, no `AppKit`.

/// A scroll of the text block, by keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextScroll {
    /// Lines down (negative: up).
    Lines(isize),
    /// Half the visible height down (negative: up).
    HalfPages(isize),
    Top,
    Bottom,
}

/// The size of the text block: its visible height, the full text's height and one line's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub visible: f64,
    pub content: f64,
    pub line: f64,
}

impl Extent {
    /// The offset that shows the end of the text.
    pub fn bottom(self) -> f64 {
        (self.content - self.visible).max(0.0)
    }

    /// `offset` shows the end of the text (within half a line).
    pub fn at_bottom(self, offset: f64) -> bool {
        offset >= self.bottom() - self.line / 2.0
    }

    fn clamp(self, offset: f64) -> f64 {
        offset.clamp(0.0, self.bottom())
    }
}

/// The tallest height of whole `line`s that fits in `height`, so no line shows cut in half.
pub fn whole_lines(height: f64, line: f64) -> f64 {
    (height.max(0.0) / line).floor() * line
}

/// The offset after scrolling `by` from `offset`.
pub fn scrolled(by: TextScroll, offset: f64, e: Extent) -> f64 {
    let step = |n: isize, size: f64| offset + n as f64 * size;
    e.clamp(match by {
        TextScroll::Lines(n) => step(n, e.line),
        // Whole lines, at least one, so a half page never cuts a line in two.
        TextScroll::HalfPages(n) => step(n, ((e.visible / 2.0 / e.line).floor().max(1.0)) * e.line),
        TextScroll::Top => 0.0,
        TextScroll::Bottom => e.bottom(),
    })
}

/// What the text block showed before a frame replaced its text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Before {
    /// It had no text: the new text is a fresh start.
    pub empty: bool,
    /// It showed the end of its text.
    pub at_bottom: bool,
    pub offset: f64,
}

/// The offset for new text of extent `e`. Fresh text starts at its top, or at its end when
/// `tail`; tail text the user was at the end of stays at the end as it grows. Otherwise the
/// offset holds.
pub fn after_change(before: Before, tail: bool, e: Extent) -> f64 {
    if tail && (before.empty || before.at_bottom) {
        e.bottom()
    } else if before.empty {
        0.0
    } else {
        e.clamp(before.offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: Extent = Extent { visible: 100.0, content: 300.0, line: 15.0 };

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn keys_scroll_by_lines_and_half_pages_and_stop_at_both_ends() {
        assert!(near(scrolled(TextScroll::Lines(1), 0.0, E), 15.0));
        assert!(near(scrolled(TextScroll::Lines(-1), 10.0, E), 0.0));
        assert!(near(scrolled(TextScroll::Lines(3), 190.0, E), 200.0));
        // Half of 100 pt is three whole 15 pt lines.
        assert!(near(scrolled(TextScroll::HalfPages(1), 0.0, E), 45.0));
        assert!(near(scrolled(TextScroll::HalfPages(-1), 60.0, E), 15.0));
        assert!(near(scrolled(TextScroll::Top, 120.0, E), 0.0));
        assert!(near(scrolled(TextScroll::Bottom, 0.0, E), 200.0));
        // A block shorter than a line still moves a line; text that fits never moves.
        let tiny = Extent { visible: 10.0, ..E };
        assert!(near(scrolled(TextScroll::HalfPages(1), 0.0, tiny), 15.0));
        let fits = Extent { content: 80.0, ..E };
        assert!(near(scrolled(TextScroll::Bottom, 0.0, fits), 0.0));
        assert!(near(scrolled(TextScroll::Lines(2), 0.0, fits), 0.0));
    }

    #[test]
    fn the_block_shows_whole_lines() {
        assert!(near(whole_lines(100.0, 15.0), 90.0));
        assert!(near(whole_lines(90.0, 15.0), 90.0));
        assert!(near(whole_lines(-4.0, 15.0), 0.0));
    }

    #[test]
    fn the_bottom_is_within_half_a_line() {
        assert!(near(E.bottom(), 200.0));
        assert!(E.at_bottom(200.0) && E.at_bottom(193.0));
        assert!(!E.at_bottom(185.0));
        assert!(Extent { content: 50.0, ..E }.at_bottom(0.0));
    }

    #[test]
    fn new_text_starts_at_the_top_or_follows_the_tail() {
        let fresh = Before { empty: true, at_bottom: true, offset: 0.0 };
        assert!(near(after_change(fresh, false, E), 0.0));
        assert!(near(after_change(fresh, true, E), 200.0));
        // At the end of tail text: still at the end after it grew.
        let end = Before { empty: false, at_bottom: true, offset: 120.0 };
        assert!(near(after_change(end, true, E), 200.0));
        assert!(near(after_change(end, false, E), 120.0));
        // Scrolled back: the offset holds, within the new text.
        let back = Before { empty: false, at_bottom: false, offset: 60.0 };
        assert!(near(after_change(back, true, E), 60.0));
        let short = Extent { content: 120.0, ..E };
        assert!(near(after_change(back, false, short), 20.0));
    }
}
