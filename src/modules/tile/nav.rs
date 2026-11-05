//! Directional neighbours: the window `h/j/k/l` reaches from a window, by a walk of its
//! workspace's tree (`AeroSpace`'s `closestParent(hasChildrenInDirection:)` and
//! `findLeafWindowRecursive(snappedTo:)`), crossing at the tree's edge to the workspace
//! visible on the screen that way. Pure Rust. The walk reads the tree through `Tree`, so it
//! does not depend on how the tree is stored; `tree::Tree` implements it.

use super::WinId;
use super::tree::Orientation;
use super::workspace::{Workspaces, Ws};
use crate::platform::Rect;

/// A direction, as `h/j/k/l` name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Down,
    Up,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Left, Dir::Down, Dir::Up, Dir::Right];

    /// The direction of vim key `h`, `j`, `k` or `l`.
    pub fn from_key(key: char) -> Option<Dir> {
        Some(match key {
            'h' => Dir::Left,
            'j' => Dir::Down,
            'k' => Dir::Up,
            'l' => Dir::Right,
            _ => return None,
        })
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Left => Dir::Right,
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::Right => Dir::Left,
        }
    }

    /// The orientation of the containers it moves along.
    pub fn orientation(self) -> Orientation {
        match self {
            Dir::Left | Dir::Right => Orientation::H,
            Dir::Down | Dir::Up => Orientation::V,
        }
    }

    /// Towards later children: right or down.
    fn positive(self) -> bool {
        matches!(self, Dir::Right | Dir::Down)
    }

    /// A unit step in Accessibility coordinates (y grows downward).
    fn unit(self) -> (f64, f64) {
        match self {
            Dir::Left => (-1.0, 0.0),
            Dir::Down => (0.0, 1.0),
            Dir::Up => (0.0, -1.0),
            Dir::Right => (1.0, 0.0),
        }
    }
}

/// What the walk reads from one workspace's tree: containers with ordered children, and
/// window leaves.
pub trait Tree {
    type Node: Copy;
    fn root(&self) -> Self::Node;
    /// The leaf holding `w`, when `w` is in this tree.
    fn leaf(&self, w: WinId) -> Option<Self::Node>;
    /// `n`'s parent and `n`'s index among its children; `None` for the root.
    fn parent(&self, n: Self::Node) -> Option<(Self::Node, usize)>;
    /// Child `i` of container `n`.
    fn child(&self, n: Self::Node, i: usize) -> Option<Self::Node>;
    /// How many children `n` has (0 for a leaf).
    fn len(&self, n: Self::Node) -> usize;
    /// Container `n`'s orientation (an accordion's too).
    fn orientation(&self, n: Self::Node) -> Orientation;
    /// The child of `n` focused most recently.
    fn recent(&self, n: Self::Node) -> Option<Self::Node>;
    /// Leaf `n`'s window; `None` for a container.
    fn window(&self, n: Self::Node) -> Option<WinId>;
}

/// Where a directional focus lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Window(WinId),
    /// Screen `i` (of `Workspaces::screens`), whose visible workspace has no tiles.
    Screen(usize),
}

/// The node beside `n`'s branch in `dir`: from `n` up, the first container of `dir`'s
/// orientation with a child next to the branch that way; that child.
fn sibling<T: Tree>(t: &T, n: T::Node, dir: Dir) -> Option<T::Node> {
    let (parent, i) = t.parent(n)?;
    let j = if dir.positive() { Some(i + 1) } else { i.checked_sub(1) };
    let beside =
        j.filter(|_| t.orientation(parent) == dir.orientation()).and_then(|j| t.child(parent, j));
    beside.or_else(|| sibling(t, parent, dir))
}

/// The window in `n` against its `side` edge: in a container of `side`'s orientation the
/// child on that edge, across it the child focused most recently.
fn leaf_at<T: Tree>(t: &T, n: T::Node, side: Dir) -> Option<WinId> {
    if let Some(w) = t.window(n) {
        return Some(w);
    }
    let next = if t.orientation(n) != side.orientation() {
        t.recent(n)
    } else if side.positive() {
        t.child(n, t.len(n).checked_sub(1)?)
    } else {
        t.child(n, 0)
    };
    leaf_at(t, next?, side)
}

/// The window `dir` of window `from` in tree `t`, for focus and swap; `None` at the
/// tree's edge.
pub fn neighbour<T: Tree>(t: &T, from: WinId, dir: Dir) -> Option<WinId> {
    leaf_at(t, sibling(t, t.leaf(from)?, dir)?, dir.opposite())
}

/// The node window `from` joins with going `dir`: the neighbouring branch, which may be a
/// container. `None` at the tree's edge.
pub fn join_target<T: Tree>(t: &T, from: WinId, dir: Dir) -> Option<T::Node> {
    sibling(t, t.leaf(from)?, dir)
}

/// The window focus enters tree `t` at when it comes in going `dir`: the one against the
/// edge it comes through.
pub fn entry<T: Tree>(t: &T, dir: Dir) -> Option<WinId> {
    leaf_at(t, t.root(), dir.opposite())
}

fn center(r: Rect) -> (f64, f64) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// The nearest of `areas` whose center lies within 45 degrees of `dir` from the center of
/// `areas[from]`. A staggered screen still counts as to the left; one mostly above does not.
pub fn screen_toward(areas: &[Rect], from: usize, dir: Dir) -> Option<usize> {
    let (fx, fy) = center(*areas.get(from)?);
    let (dx, dy) = dir.unit();
    let distance = |a: Rect| {
        let (x, y) = center(a);
        let along = (x - fx) * dx + (y - fy) * dy;
        let across = (x - fx) * dy - (y - fy) * dx;
        (along > across.abs()).then(|| (x - fx).hypot(y - fy))
    };
    areas
        .iter()
        .enumerate()
        .filter_map(|(i, a)| Some((distance(*a)?, i)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, i)| i)
}

/// Where focus goes from window `from` (`None`: the focused workspace has no focused
/// window) going `dir`: its neighbour in the focused workspace's tree, else across to the
/// screen that way, entering its visible workspace at the near edge. `tree` gives each
/// workspace's tree. `None` when there is nothing that way.
pub fn focus_target<'a, T: Tree + 'a>(
    ws: &Workspaces,
    from: Option<WinId>,
    dir: Dir,
    tree: impl Fn(Ws) -> Option<&'a T>,
) -> Option<Target> {
    let here = from.zip(tree(ws.focused())).and_then(|(w, t)| neighbour(t, w, dir));
    if let Some(w) = here {
        return Some(Target::Window(w));
    }
    let areas: Vec<Rect> = ws.screens().iter().map(|s| s.area).collect();
    let screen = screen_toward(&areas, ws.focused_screen()?, dir)?;
    let entered = ws.shown_on(screen).and_then(&tree).and_then(|t| entry(t, dir));
    Some(entered.map_or(Target::Screen(screen), Target::Window))
}

#[cfg(test)]
mod tests;
