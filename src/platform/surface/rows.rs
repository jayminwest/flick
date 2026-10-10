//! The pure side of a surface's transcript: the rows a module describes (`Row`) and their
//! owned copies, the diff that decides which row views survive a `set_rows`, whether the
//! scroll is pinned to the bottom, the 50 ms coalescing of redraws, a bubble's state badge
//! and the frames of every row. No `AppKit`.

use std::collections::{HashMap, VecDeque};

use super::geometry::{Rect, Size};
use crate::core::card::Card;
use crate::platform::hud::CardUi;

/// Whose a bubble is: the user's (right), the peer's (left) or the surface's own (centred,
/// no fill).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Mine,
    Theirs,
    System,
}

/// Where a bubble's text is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BubbleState {
    Done,
    /// Still arriving: a spinner by the header.
    Streaming,
    /// Waiting for its text: a spinner and dimmed text.
    Pending,
    /// A red mark by the header.
    Failed,
}

/// One transcript row. Rows are identified by kind, `key` and `version`: a row whose three
/// match the last `set_rows` keeps its views, so `version` must change whenever anything the
/// row shows changes (text, state, the card or its `CardUi`).
#[derive(Clone, Copy, Debug)]
pub enum Row<'a> {
    /// Markdown-lite text in a bubble, with a header and a time above it (either may be empty).
    Bubble {
        key: &'a str,
        version: u64,
        side: Side,
        header: &'a str,
        time: &'a str,
        md: &'a str,
        state: BubbleState,
    },
    /// A card, drawn by the HUD's card renderer; presses go to `Handlers::card_action`.
    Card { key: &'a str, version: u64, card: &'a Card, ui: CardUi<'a> },
    /// A centred caption across the transcript ("Yesterday").
    Divider { text: &'a str },
}

/// A `CardUi` the surface can keep.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ui {
    pending: bool,
    error: Option<String>,
    confirm: Option<String>,
    note: Option<String>,
}

impl Ui {
    fn new(ui: &CardUi) -> Ui {
        let own = |s: Option<&str>| s.map(str::to_string);
        Ui {
            pending: ui.pending,
            error: own(ui.error),
            confirm: own(ui.confirm),
            note: own(ui.note),
        }
    }

    pub fn get(&self) -> CardUi<'_> {
        CardUi {
            pending: self.pending,
            error: self.error.as_deref(),
            confirm: self.confirm.as_deref(),
            note: self.note.as_deref(),
        }
    }

    /// The same, showing `error`.
    pub fn with_error(&self, error: &str) -> Ui {
        Ui { error: Some(error.to_string()), ..self.clone() }
    }
}

/// A `Row` the surface keeps until the coalesced redraw.
#[derive(Clone, Debug, PartialEq)]
pub enum Owned {
    Bubble {
        key: String,
        version: u64,
        side: Side,
        header: String,
        time: String,
        md: String,
        state: BubbleState,
    },
    Card {
        key: String,
        version: u64,
        card: Box<Card>,
        ui: Ui,
    },
    Divider {
        text: String,
    },
}

/// What kind of row an `Ident` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Bubble,
    Card,
    Divider,
}

/// What `diff` compares: a divider's key is its text, its version 0.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ident {
    pub kind: Kind,
    pub key: String,
    pub version: u64,
}

impl Owned {
    pub fn new(row: &Row) -> Owned {
        match *row {
            Row::Bubble { key, version, side, header, time, md, state } => Owned::Bubble {
                key: key.into(),
                version,
                side,
                header: header.into(),
                time: time.into(),
                md: md.into(),
                state,
            },
            Row::Card { key, version, card, ui } => Owned::Card {
                key: key.into(),
                version,
                card: Box::new(card.clone()),
                ui: Ui::new(&ui),
            },
            Row::Divider { text } => Owned::Divider { text: text.into() },
        }
    }

    pub fn ident(&self) -> Ident {
        let (kind, key, version) = match self {
            Owned::Bubble { key, version, .. } => (Kind::Bubble, key, *version),
            Owned::Card { key, version, .. } => (Kind::Card, key, *version),
            Owned::Divider { text } => (Kind::Divider, text, 0),
        };
        Ident { kind, key: key.clone(), version }
    }
}

/// What to do for one new row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Reuse the old row at this index, views and all.
    Keep(usize),
    /// Build new views.
    Build,
}

/// For each of `new`, the old row with the same ident to reuse (each at most once, in order),
/// or `Build`. Old rows no plan keeps are dropped.
pub fn diff(old: &[Ident], new: &[Ident]) -> Vec<Plan> {
    let mut by: HashMap<&Ident, VecDeque<usize>> = HashMap::new();
    for (i, id) in old.iter().enumerate() {
        by.entry(id).or_default().push_back(i);
    }
    new.iter()
        .map(|id| by.get_mut(id).and_then(VecDeque::pop_front).map_or(Plan::Build, Plan::Keep))
        .collect()
}

/// How close to the end counts as the bottom, in points.
const PIN_SLACK: f64 = 4.0;

/// Whether a scroll at `offset` (top of the visible part, top-down) showing `visible` points
/// of a document `doc` tall is at the bottom, so it should stay there.
pub fn at_bottom(offset: f64, visible: f64, doc: f64) -> bool {
    offset + visible >= doc - PIN_SLACK
}

/// The offset that shows the end of a document `doc` tall through `visible` points.
pub fn bottom(visible: f64, doc: f64) -> f64 {
    (doc - visible).max(0.0)
}

/// Seconds a redraw waits for more `set_rows` calls.
pub const COALESCE_SECS: f64 = 0.05;

/// Whether a redraw is scheduled. `request` says when to arm the timer: only for the first
/// call since the last redraw, so a burst of updates costs one layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coalesce {
    armed: bool,
}

impl Coalesce {
    /// A redraw is wanted; true when the caller must arm the timer.
    pub fn request(&mut self) -> bool {
        !std::mem::replace(&mut self.armed, true)
    }

    /// The redraw ran (from the timer or early); the next request arms again.
    pub fn done(&mut self) {
        self.armed = false;
    }
}

/// What a bubble's Copy Message puts on the pasteboard: the message as written (its
/// markdown-lite source), without the blank lines and spaces around it.
pub fn message_text(md: &str) -> &str {
    md.trim()
}

/// What a bubble shows for its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Badge {
    pub spinner: bool,
    pub failed: bool,
    /// Text in the secondary color.
    pub dim: bool,
}

pub fn badge(state: BubbleState) -> Badge {
    let (spinner, failed, dim) = match state {
        BubbleState::Done => (false, false, false),
        BubbleState::Streaming => (true, false, false),
        BubbleState::Pending => (true, false, true),
        BubbleState::Failed => (false, true, false),
    };
    Badge { spinner, failed, dim }
}

/// Side margin of the rows.
pub const PAD_X: f64 = 14.0;
/// Space above the first row and below the last.
const PAD_Y: f64 = 10.0;
const ROW_GAP: f64 = 10.0;
/// Text inset inside a bubble.
pub const BUBBLE_X: f64 = 10.0;
pub const BUBBLE_Y: f64 = 6.0;
/// The header line above a bubble.
pub const META_H: f64 = 15.0;
const META_GAP: f64 = 3.0;
/// The spinner or failure mark beside the header.
pub const BADGE: f64 = 12.0;
const BADGE_GAP: f64 = 5.0;
/// A peer or user bubble takes at most this share of the row.
const BUBBLE_SHARE: f64 = 0.8;
pub const DIVIDER_H: f64 = 18.0;

/// The widest a bubble's text may wrap at in a transcript `width` wide.
pub fn text_max(width: f64, side: Side) -> f64 {
    let inner = (width - 2.0 * PAD_X).max(0.0);
    let share = if side == Side::System { 1.0 } else { BUBBLE_SHARE };
    (inner * share - 2.0 * BUBBLE_X).max(1.0)
}

/// The width a card or divider is drawn at.
pub fn full_width(width: f64) -> f64 {
    (width - 2.0 * PAD_X).max(1.0)
}

/// A row's measured size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// `text`: the wrapped text's size; `meta_w`: the header line's width (0: none).
    Bubble { side: Side, text: Size, meta_w: f64, badge: bool },
    /// A card or divider: full width, `h` tall.
    Full { h: f64 },
}

/// Where a row and its parts go. `row` is in the document (top-down); the rest are inside
/// the row, and zero for a full-width row (and for a missing header or badge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frames {
    pub row: Rect,
    pub meta: Rect,
    pub badge: Rect,
    pub bubble: Rect,
    pub text: Rect,
}

const ZERO: Rect = Rect::new(0.0, 0.0, 0.0, 0.0);

/// `w` points placed in a row `width` wide on `side`'s edge (centred for `System`).
fn align(side: Side, width: f64, w: f64) -> f64 {
    match side {
        Side::Theirs => PAD_X,
        Side::Mine => width - PAD_X - w,
        Side::System => (width - w) / 2.0,
    }
}

/// A bubble row's parts, `width` wide; returns them and the row's height.
fn bubble(width: f64, side: Side, text: Size, meta_w: f64, badge: bool) -> (Frames, f64) {
    let gap = if meta_w > 0.0 && badge { BADGE_GAP } else { 0.0 };
    let group = meta_w + gap + if badge { BADGE } else { 0.0 };
    let (meta, badge_at, top) = if group > 0.0 {
        let x = align(side, width, group);
        let b = Rect::new(x + meta_w + gap, (META_H - BADGE) / 2.0, BADGE, BADGE);
        (Rect::new(x, 0.0, meta_w, META_H), b, META_H + META_GAP)
    } else {
        (ZERO, ZERO, 0.0)
    };
    let (bw, bh) = (text.w + 2.0 * BUBBLE_X, text.h + 2.0 * BUBBLE_Y);
    let b = Rect::new(align(side, width, bw), top, bw, bh);
    let t = Rect::new(b.x + BUBBLE_X, b.y + BUBBLE_Y, text.w, text.h);
    let meta = if meta_w > 0.0 { meta } else { ZERO };
    let badge_at = if badge { badge_at } else { ZERO };
    (Frames { row: ZERO, meta, badge: badge_at, bubble: b, text: t }, top + bh)
}

/// Stack `shapes` top-down in a transcript `width` wide whose visible part is `visible`
/// tall. Rows sit at the bottom while they do not fill it, as in a chat. Returns each row's
/// frames and the document height (at least `visible`).
pub fn stack(width: f64, visible: f64, shapes: &[Shape]) -> (Vec<Frames>, f64) {
    let mut y = PAD_Y;
    let mut out: Vec<Frames> = shapes
        .iter()
        .map(|shape| {
            let f = match *shape {
                Shape::Bubble { side, text, meta_w, badge } => {
                    let (f, h) = bubble(width, side, text, meta_w, badge);
                    Frames { row: Rect::new(0.0, y, width, h), ..f }
                }
                Shape::Full { h } => Frames {
                    row: Rect::new(PAD_X, y, full_width(width), h),
                    meta: ZERO,
                    badge: ZERO,
                    bubble: ZERO,
                    text: ZERO,
                },
            };
            y += f.row.h + ROW_GAP;
            f
        })
        .collect();
    let content = if shapes.is_empty() { 0.0 } else { y - ROW_GAP + PAD_Y };
    let shift = (visible - content).max(0.0);
    for f in &mut out {
        f.row.y += shift;
    }
    (out, content.max(visible))
}

#[cfg(test)]
mod tests;
