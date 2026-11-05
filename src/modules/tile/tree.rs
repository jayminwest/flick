//! The split tree of one workspace: containers that share their area among their children by
//! weight, and window leaves. Pure Rust.
//!
//! Normalization is always on (no setting) and runs at the end of every op, and the fields
//! are private, so a caller only ever sees a normalized tree:
//! - empty containers go, except the root;
//! - a non-root container with one child is replaced by that child, which keeps the
//!   container's weight and index;
//! - a root holding a single container takes that container's children, layout and
//!   orientation (the root may hold a single window);
//! - a nested container has the opposite orientation of its parent;
//! - every weight is positive and finite, each window appears once, and the zoomed window
//!   and the focus history hold only windows in the tree.
//!
//! Each container's most recent child is the one holding the window focused last of those
//! under it (`focus`, `recent_child`): one focus history per tree, so the ops never have to
//! keep a per-container index in step. `nav` reads the tree through `nav::Tree`, with
//! `Handle` for a node.
//!
//! Ported from `AeroSpace` (MIT, commit bc6be84): tree/normalizeContainers.swift and
//! tree/TilingContainer.swift.

mod ops;

use super::WinId;
use super::nav;

/// The axis a container splits along: `H` puts its children side by side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    H,
    V,
}

impl Orientation {
    pub fn opposite(self) -> Self {
        match self {
            Self::H => Self::V,
            Self::V => Self::H,
        }
    }
}

/// How a container shows its children: `Tiles` splits the area by weight, `Accordion`
/// stacks them, each over almost all of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Tiles,
    Accordion,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Window(WinId),
    Split(Container),
}

/// A node and its share of its parent's area, relative to its siblings' weights.
#[derive(Clone, Debug, PartialEq)]
pub struct Child {
    pub weight: f64,
    pub node: Node,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Container {
    pub orientation: Orientation,
    pub layout: Layout,
    pub children: Vec<Child>,
}

impl Child {
    /// Whether this child is `win` or holds it.
    pub fn holds(&self, win: WinId) -> bool {
        match &self.node {
            Node::Window(w) => *w == win,
            Node::Split(inner) => inner.contains(win),
        }
    }
}

impl Container {
    /// Whether `win` is anywhere under this container.
    pub fn contains(&self, win: WinId) -> bool {
        self.children.iter().any(|c| c.holds(win))
    }

    /// The weight a new child gets: the mean of its siblings', or 1 for the first.
    fn mean_weight(&self) -> f64 {
        let n = self.children.len();
        if n == 0 { 1.0 } else { self.children.iter().map(|c| c.weight).sum::<f64>() / n as f64 }
    }

    /// Every window under this container, depth first, into `out`.
    fn windows_into(&self, out: &mut Vec<WinId>) {
        for child in &self.children {
            match &child.node {
                Node::Window(w) => out.push(*w),
                Node::Split(inner) => inner.windows_into(out),
            }
        }
    }
}

/// One workspace's tree. `Default` is the empty workspace.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    root: Container,
    zoom: Option<WinId>,
    /// Windows by focus, most recent first.
    recent: Vec<WinId>,
}

impl Tree {
    /// The tree `root` gives, normalized.
    pub fn from_root(root: Container) -> Self {
        let mut tree = Self { root, zoom: None, recent: vec![] };
        tree.normalize();
        tree
    }

    pub fn root(&self) -> &Container {
        &self.root
    }

    /// The window that takes the whole workspace while zoomed.
    pub fn zoomed(&self) -> Option<WinId> {
        self.zoom
    }

    pub fn contains(&self, win: WinId) -> bool {
        self.root.contains(win)
    }

    /// Mark `win` focused: it becomes the most recent child of every container it is under.
    /// False when it is not in the tree.
    pub fn focus(&mut self, win: WinId) -> bool {
        if !self.contains(win) {
            return false;
        }
        self.recent.retain(|&w| w != win);
        self.recent.insert(0, win);
        true
    }

    /// The index of `c`'s child focused most recently: the one holding the window focused
    /// last of those under it, else the first.
    pub fn recent_child(&self, c: &Container) -> usize {
        self.recent.iter().find_map(|&w| c.children.iter().position(|ch| ch.holds(w))).unwrap_or(0)
    }

    /// Every window, depth first (left to right, top to bottom).
    pub fn windows(&self) -> Vec<WinId> {
        let mut out = vec![];
        self.root.windows_into(&mut out);
        out
    }

    /// The child indices from the root down to `win`'s leaf.
    pub fn path(&self, win: WinId) -> Option<Vec<usize>> {
        fn walk(c: &Container, win: WinId, path: &mut Vec<usize>) -> bool {
            c.children.iter().enumerate().any(|(i, child)| {
                path.push(i);
                let found = match &child.node {
                    Node::Window(w) => *w == win,
                    Node::Split(inner) => walk(inner, win, path),
                };
                if !found {
                    path.pop();
                }
                found
            })
        }
        let mut path = vec![];
        walk(&self.root, win, &mut path).then_some(path)
    }

    /// The container at `path` (child indices from the root), if every step is one.
    pub fn container(&self, path: &[usize]) -> Option<&Container> {
        path.iter().try_fold(&self.root, |c, &i| match &c.children.get(i)?.node {
            Node::Split(inner) => Some(inner),
            Node::Window(_) => None,
        })
    }

    fn container_mut(&mut self, path: &[usize]) -> Option<&mut Container> {
        path.iter().try_fold(&mut self.root, |c, &i| match &mut c.children.get_mut(i)?.node {
            Node::Split(inner) => Some(inner),
            Node::Window(_) => None,
        })
    }

    /// Add `win` right after the window focused last, in its container, or at the end of the
    /// root when none has been. False when `win` is already in the tree.
    pub fn insert(&mut self, win: WinId) -> bool {
        if self.contains(win) {
            return false;
        }
        let at = self.recent.first().and_then(|&f| self.path(f)).and_then(|p| {
            let (i, parent) = p.split_last()?;
            Some((parent.to_vec(), i + 1))
        });
        let (parent, index) = at.unwrap_or_else(|| (vec![], self.root.children.len()));
        let Some(c) = self.container_mut(&parent) else { return false };
        let weight = c.mean_weight();
        c.children.insert(index, Child { weight, node: Node::Window(win) });
        self.normalize();
        true
    }

    /// Take `win` out of the tree; its siblings share its space. False when it is not there.
    pub fn remove(&mut self, win: WinId) -> bool {
        if !self.unlink(win) {
            return false;
        }
        self.normalize();
        true
    }

    /// Take `win`'s leaf out without normalizing, so the indices of other nodes stay put
    /// (a container it empties stays until `normalize`).
    fn unlink(&mut self, win: WinId) -> bool {
        let Some(path) = self.path(win) else { return false };
        let Some((&i, parent)) = path.split_last() else { return false };
        let Some(c) = self.container_mut(parent) else { return false };
        c.children.remove(i);
        true
    }

    /// Exchange the places of windows `a` and `b`; each slot keeps its weight. False unless
    /// both are in the tree.
    pub fn swap(&mut self, a: WinId, b: WinId) -> bool {
        fn exchange(c: &mut Container, a: WinId, b: WinId) {
            for child in &mut c.children {
                match &mut child.node {
                    Node::Window(w) if *w == a => *w = b,
                    Node::Window(w) if *w == b => *w = a,
                    Node::Window(_) => {}
                    Node::Split(inner) => exchange(inner, a, b),
                }
            }
        }
        if !self.contains(a) || !self.contains(b) {
            return false;
        }
        exchange(&mut self.root, a, b);
        self.normalize();
        true
    }

    fn normalize(&mut self) {
        let mut seen = vec![];
        flatten(&mut self.root, &mut seen);
        if let [Child { node: Node::Split(only), .. }] = &mut self.root.children[..] {
            let only = std::mem::take(only);
            self.root = only;
        }
        orient(&mut self.root);
        self.zoom = self.zoom.filter(|&z| seen.contains(&z));
        self.recent.retain(|w| seen.contains(w));
    }

    /// The path from the root to node `n`, when `n` is in this tree.
    fn locate(&self, n: Handle) -> Option<Vec<usize>> {
        let mut path = match n.first {
            Some(w) => self.path(w)?,
            None => vec![],
        };
        (path.len() >= n.depth).then(|| {
            path.truncate(n.depth);
            path
        })
    }

    /// The handle of the node at `path`.
    fn handle(&self, path: &[usize]) -> Option<Handle> {
        let Some((&i, up)) = path.split_last() else { return Some(Handle::ROOT) };
        let first = first_window(&self.container(up)?.children.get(i)?.node)?;
        Some(Handle { first: Some(first), depth: path.len() })
    }

    /// Node `n`, when it is a container.
    fn split(&self, n: Handle) -> Option<&Container> {
        self.container(&self.locate(n)?)
    }
}

/// A node of a `Tree`, as `nav` reads it: the root, or the node `depth` steps down the path
/// to window `first`, the first window under it, so every node has exactly one handle. A
/// handle is valid until the tree changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle {
    first: Option<WinId>,
    depth: usize,
}

impl Handle {
    const ROOT: Handle = Handle { first: None, depth: 0 };
}

impl nav::Tree for Tree {
    type Node = Handle;

    fn root(&self) -> Handle {
        Handle::ROOT
    }

    fn leaf(&self, w: WinId) -> Option<Handle> {
        Some(Handle { first: Some(w), depth: self.path(w)?.len() })
    }

    fn parent(&self, n: Handle) -> Option<(Handle, usize)> {
        let path = self.locate(n)?;
        let (&i, up) = path.split_last()?;
        Some((self.handle(up)?, i))
    }

    fn child(&self, n: Handle, i: usize) -> Option<Handle> {
        let mut path = self.locate(n)?;
        path.push(i);
        self.handle(&path)
    }

    fn len(&self, n: Handle) -> usize {
        self.split(n).map_or(0, |c| c.children.len())
    }

    fn orientation(&self, n: Handle) -> Orientation {
        self.split(n).map_or(Orientation::H, |c| c.orientation)
    }

    fn recent(&self, n: Handle) -> Option<Handle> {
        let c = self.split(n)?;
        self.child(n, self.recent_child(c))
    }

    fn window(&self, n: Handle) -> Option<WinId> {
        let path = self.locate(n)?;
        n.first.filter(|_| self.container(&path).is_none())
    }
}

/// The first window under `node` (`node` itself for a window).
fn first_window(node: &Node) -> Option<WinId> {
    match node {
        Node::Window(w) => Some(*w),
        Node::Split(c) => first_window(&c.children.first()?.node),
    }
}

/// Normalize the subtrees under `c`: fix bad weights, drop windows already `seen` and empty
/// containers, and replace a single-child container by its child.
fn flatten(c: &mut Container, seen: &mut Vec<WinId>) {
    c.children = std::mem::take(&mut c.children)
        .into_iter()
        .filter_map(|mut child| {
            if !child.weight.is_finite() || child.weight <= 0.0 {
                child.weight = 1.0;
            }
            match &mut child.node {
                Node::Window(w) if seen.contains(w) => return None,
                Node::Window(w) => seen.push(*w),
                Node::Split(inner) => {
                    flatten(inner, seen);
                    match inner.children.len() {
                        0 => return None,
                        1 => child.node = inner.children.remove(0).node,
                        _ => {}
                    }
                }
            }
            Some(child)
        })
        .collect();
}

/// Give every container under `c` the opposite orientation of its parent.
fn orient(c: &mut Container) {
    let nested = c.orientation.opposite();
    for child in &mut c.children {
        if let Node::Split(inner) = &mut child.node {
            inner.orientation = nested;
            orient(inner);
        }
    }
}

#[cfg(test)]
mod tests;
