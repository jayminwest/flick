//! A workspace's tree laid out in its screen area: one frame per window, in whole pixels, in
//! the Accessibility coordinate space (origin top-left, y grows downward). Pure Rust.
//!
//! Tiles split a container along its orientation by weight, rescaled to fill it on every
//! layout, with edges rounded so the tiles meet with no gaps or overlaps. Accordion gives
//! each child the whole container but for a padding at each end that leaves its neighbours
//! peeking out. Gaps are 0. Ported from `AeroSpace` (MIT, commit bc6be84):
//! layout/layoutRecursive.swift.

use crate::modules::tile::tree::{Container, Layout, Node, Orientation, Tree, WinId};

/// A frame in whole pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// How far an accordion's children stand off each other. A constant, not a setting.
pub const ACCORDION_PADDING: i32 = 30;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Opts {
    /// The focused window: an accordion shows the child that holds it on top, with its
    /// neighbours peeking out further.
    pub focused: Option<WinId>,
    /// The screens stack vertically. macOS can then refuse a window on the upper screen the
    /// full height, so the workspace loses its bottom pixel (layoutRecursive.swift:9-12).
    pub stacked: bool,
}

/// The frame of each window of `tree`, depth first, laid out in `area`. A zoomed window
/// gets the whole workspace; the others keep their tiles behind it.
pub fn layout(tree: &Tree, area: Frame, opts: Opts) -> Vec<(WinId, Frame)> {
    let rect = workspace(area, opts);
    let mut out = vec![];
    place(tree.root(), rect, opts.focused, &mut out);
    if let Some(zoom) = tree.zoomed() {
        out.iter_mut().filter(|(w, _)| *w == zoom).for_each(|(_, f)| *f = rect);
    }
    out
}

/// The frame of the container at `path` (child indices from the root), laid out in `area`.
pub fn rect_at(tree: &Tree, path: &[usize], area: Frame, opts: Opts) -> Option<Frame> {
    let mut c = tree.root();
    let mut rect = workspace(area, opts);
    for &i in path {
        rect = *frames(c, rect, opts.focused).get(i)?;
        let Node::Split(inner) = &c.children.get(i)?.node else { return None };
        c = inner;
    }
    Some(rect)
}

fn workspace(area: Frame, opts: Opts) -> Frame {
    if opts.stacked { Frame { h: area.h - 1, ..area } } else { area }
}

fn place(c: &Container, rect: Frame, focused: Option<WinId>, out: &mut Vec<(WinId, Frame)>) {
    for (child, frame) in c.children.iter().zip(frames(c, rect, focused)) {
        match &child.node {
            Node::Window(w) => out.push((*w, frame)),
            Node::Split(inner) => place(inner, frame, focused, out),
        }
    }
}

/// The frame of each child of `c` in `rect`.
fn frames(c: &Container, rect: Frame, focused: Option<WinId>) -> Vec<Frame> {
    let (start, extent) = match c.orientation {
        Orientation::H => (rect.x, rect.w),
        Orientation::V => (rect.y, rect.h),
    };
    let spans = match c.layout {
        Layout::Tiles => {
            tiles(start, extent, &c.children.iter().map(|ch| ch.weight).collect::<Vec<_>>())
        }
        Layout::Accordion => {
            let top = focused.and_then(|f| {
                c.children.iter().position(|ch| match &ch.node {
                    Node::Window(w) => *w == f,
                    Node::Split(inner) => inner.contains(f),
                })
            });
            accordion(start, extent, c.children.len(), top.unwrap_or(0))
        }
    };
    spans
        .into_iter()
        .map(|(at, len)| match c.orientation {
            Orientation::H => Frame { x: at, w: len, ..rect },
            Orientation::V => Frame { y: at, h: len, ..rect },
        })
        .collect()
}

/// `(start, length)` of each tile splitting `extent` pixels from `start` by `weights`
/// (positive). Each edge rounds on its own and the last one is the end, so the lengths sum
/// to `extent` exactly.
pub fn tiles(start: i32, extent: i32, weights: &[f64]) -> Vec<(i32, i32)> {
    let total: f64 = weights.iter().sum();
    let mut sum = 0.0;
    let mut edge = start;
    weights
        .iter()
        .enumerate()
        .map(|(i, w)| {
            sum += w;
            let next = if i + 1 == weights.len() {
                start + extent
            } else {
                start + (f64::from(extent) * sum / total).round() as i32
            };
            let span = (edge, next - edge);
            edge = next;
            span
        })
        .collect()
}

/// `(start, length)` of each of `n` accordion children over `extent` pixels from `start`,
/// child `top` on top: the first is inset by the padding at its far end, the last at its
/// near end, `top`'s neighbours by twice the padding on the side away from it, and the
/// others at both ends (layoutRecursive.swift:153-186).
fn accordion(start: i32, extent: i32, n: usize, top: usize) -> Vec<(i32, i32)> {
    let p = ACCORDION_PADDING;
    (0..n)
        .map(|i| {
            let (near, far) = match i {
                0 if n == 1 => (0, 0),
                0 => (0, p),
                _ if i == n - 1 => (p, 0),
                _ if i + 1 == top => (0, 2 * p),
                _ if i == top + 1 => (2 * p, 0),
                _ => (p, p),
            };
            // A container narrower than the padding keeps its children inside it.
            let near = near.clamp(0, extent.max(0));
            (start + near, (extent - near - far).max(0))
        })
        .collect()
}

#[cfg(test)]
mod tests;
