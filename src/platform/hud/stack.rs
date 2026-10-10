//! Where each card of the HUD stack goes: pure geometry with a 100% coverage floor. Cards
//! come newest first; the newest sits nearest the configured corner and older ones follow
//! away from it. At most `max_visible` show, fewer when the screen is too short; the rest
//! collapse into a `+N more` pill after the last visible card; a click on it pages through
//! them (`cycle`). Coordinates are `AppKit`
//! screen points (origin bottom left, y up).

/// Inset from the screen edges.
pub const EDGE: f64 = 12.0;
/// Space between two cards, and between the last card and the pill.
pub const SPACING: f64 = 8.0;
pub const PILL_W: f64 = 96.0;
pub const PILL_H: f64 = 24.0;

/// Where the stack sits in the visible area of its screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
    Top,
    Bottom,
}

impl Corner {
    pub fn top(self) -> bool {
        matches!(self, Corner::TopRight | Corner::TopLeft | Corner::Top)
    }
}

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
}

/// The frames of one layout.
#[derive(Debug, PartialEq)]
pub struct Layout {
    /// One per input height, in the same order; `None`: hidden in the overflow.
    pub cards: Vec<Option<Rect>>,
    /// The `+N more` pill and N, when some cards are hidden.
    pub pill: Option<(Rect, usize)>,
}

/// The pill's text.
pub fn more(hidden: usize) -> String {
    format!("+{hidden} more")
}

/// Reorder `cards` (newest first) for a click on the pill: the `shown` visible ones go to
/// the back, so the hidden ones come into view in order. Clicks page through the whole
/// stack and come back to the start; the relative order never changes, only where it starts.
pub fn cycle<T>(cards: &mut [T], shown: usize) {
    let shown = shown.min(cards.len());
    cards.rotate_left(shown);
}

/// Lay out cards of `sizes` (width, height; newest first) in screen area `area`.
pub fn layout(area: Rect, corner: Corner, sizes: &[(f64, f64)], max_visible: usize) -> Layout {
    let room = (area.h - 2.0 * EDGE).max(0.0);
    let heights: Vec<f64> = sizes.iter().map(|s| s.1.min(room)).collect();
    let used = |k: usize| heights[..k].iter().sum::<f64>() + SPACING * k.saturating_sub(1) as f64;
    let mut shown = heights.len().min(max_visible.max(1));
    while shown > 1 && used(shown) > room {
        shown -= 1;
    }
    while shown > 1 && shown < heights.len() && used(shown) + SPACING + PILL_H > room {
        shown -= 1;
    }
    let x = |w: f64| match corner {
        Corner::TopLeft | Corner::BottomLeft => area.x + EDGE,
        Corner::TopRight | Corner::BottomRight => area.x + area.w - w - EDGE,
        Corner::Top | Corner::Bottom => area.x + (area.w - w) / 2.0,
    };
    // The edge of the next frame nearest the corner: its top when stacking down from a top
    // corner, its bottom when stacking up from a bottom one.
    let mut edge = if corner.top() { area.y + area.h - EDGE } else { area.y + EDGE };
    let mut place = |w: f64, h: f64| {
        let y = if corner.top() { edge - h } else { edge };
        edge = if corner.top() { y - SPACING } else { y + h + SPACING };
        Rect::new(x(w), y, w, h)
    };
    let cards = sizes.iter().zip(&heights).enumerate();
    let cards = cards.map(|(i, (s, &h))| (i < shown).then(|| place(s.0, h))).collect();
    let hidden = heights.len() - shown;
    let pill = (hidden > 0).then(|| (place(PILL_W, PILL_H), hidden));
    Layout { cards, pill }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(-1000.0, 50.0, 1000.0, 800.0);

    fn w300(heights: &[f64]) -> Vec<(f64, f64)> {
        heights.iter().map(|&h| (300.0, h)).collect()
    }

    fn origins(l: &Layout) -> Vec<Option<(f64, f64)>> {
        l.cards.iter().map(|r| r.map(|r| (r.x, r.y))).collect()
    }

    #[test]
    fn one_card_sits_in_its_corner() {
        let at = |c| origins(&layout(AREA, c, &w300(&[100.0]), 4))[0];
        assert_eq!(at(Corner::TopRight), Some((-312.0, 738.0)));
        assert_eq!(at(Corner::TopLeft), Some((-988.0, 738.0)));
        assert_eq!(at(Corner::BottomRight), Some((-312.0, 62.0)));
        assert_eq!(at(Corner::BottomLeft), Some((-988.0, 62.0)));
        assert_eq!(at(Corner::Top), Some((-650.0, 738.0)));
        assert_eq!(at(Corner::Bottom), Some((-650.0, 62.0)));
    }

    #[test]
    fn newer_cards_sit_nearer_the_corner() {
        let top = layout(AREA, Corner::TopRight, &w300(&[100.0, 50.0]), 4);
        assert_eq!(origins(&top), [Some((-312.0, 738.0)), Some((-312.0, 680.0))]);
        assert_eq!(top.pill, None);
        let bottom = layout(AREA, Corner::BottomLeft, &w300(&[100.0, 50.0]), 4);
        assert_eq!(origins(&bottom), [Some((-988.0, 62.0)), Some((-988.0, 170.0))]);
    }

    #[test]
    fn extra_cards_collapse_into_a_pill() {
        let l = layout(AREA, Corner::TopRight, &w300(&[100.0; 5]), 4);
        assert_eq!(l.cards.iter().filter(|c| c.is_some()).count(), 4);
        assert_eq!(l.cards[4], None);
        // Four cards: 738, 630, 522, 414; the pill under the last, right aligned.
        assert_eq!(l.pill, Some((Rect::new(-108.0, 382.0, PILL_W, PILL_H), 1)));
        assert_eq!(more(1), "+1 more");
        let centered = layout(AREA, Corner::Bottom, &w300(&[100.0; 3]), 1);
        assert_eq!(centered.pill, Some((Rect::new(-548.0, 170.0, PILL_W, PILL_H), 2)));
        // A max of 0 still shows the newest card.
        assert_eq!(layout(AREA, Corner::Top, &w300(&[100.0; 2]), 0).pill.map(|p| p.1), Some(1));
    }

    #[test]
    fn a_pill_click_pages_through_every_card() {
        let mut cards = ['a', 'b', 'c', 'd', 'e'];
        cycle(&mut cards, 3);
        assert_eq!(cards, ['d', 'e', 'a', 'b', 'c']);
        cycle(&mut cards, 3);
        assert_eq!(cards, ['b', 'c', 'd', 'e', 'a']);
        // Each click brings the next three to the front; the order is kept.
        cycle(&mut cards, 3);
        assert_eq!(cards, ['e', 'a', 'b', 'c', 'd']);
        // A stale count larger than the stack leaves it as it is.
        let mut two = ['a', 'b'];
        cycle(&mut two, 9);
        assert_eq!(two, ['a', 'b']);
        cycle::<char>(&mut [], 1);
    }

    #[test]
    fn a_short_screen_shows_fewer_cards() {
        let short = Rect::new(0.0, 0.0, 800.0, 344.0); // room 320
        // Three of 100 fit (316), but then the pill does not: two and a pill.
        let l = layout(short, Corner::TopLeft, &w300(&[100.0; 4]), 4);
        assert_eq!(l.pill.map(|p| p.1), Some(2));
        // Exactly three, no pill needed.
        assert_eq!(layout(short, Corner::TopLeft, &w300(&[100.0; 3]), 4).pill, None);
        // Four that never fit drop to what does.
        let l = layout(short, Corner::TopLeft, &w300(&[200.0; 4]), 4);
        assert_eq!(l.pill.map(|p| p.1), Some(3));
    }

    #[test]
    fn a_card_taller_than_the_screen_is_clamped() {
        let tiny = Rect::new(0.0, 0.0, 800.0, 200.0);
        let l = layout(tiny, Corner::BottomRight, &w300(&[500.0]), 4);
        assert_eq!(l.cards[0], Some(Rect::new(488.0, 12.0, 300.0, 176.0)));
        let none = layout(Rect::new(0.0, 0.0, 800.0, 10.0), Corner::TopLeft, &w300(&[50.0]), 4);
        assert_eq!(none.cards[0], Some(Rect::new(12.0, -2.0, 300.0, 0.0)));
        assert_eq!(layout(tiny, Corner::Top, &w300(&[]), 4), Layout { cards: vec![], pill: None });
    }
}
