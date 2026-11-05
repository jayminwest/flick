//! The tree ops beyond insert, remove and swap: join, reset, balance, layout, zoom and
//! resize. Each ends normalized, like the ops in `tree`.

use super::{Child, Container, Layout, Node, Orientation, Tree, WinId};
use crate::modules::tile::layout::{self, Frame, Opts};

/// The smallest share, in pixels, a resize leaves a tile (unless the container is too small
/// to give every child that much). A constant, not a setting.
pub const MIN_TILE: f64 = 20.0;

impl Tree {
    /// Join `win` with its neighbour along `orientation` (`forward`: right or down): the
    /// first node from `win` up whose parent splits along `orientation` and has a sibling
    /// that way; that sibling and `win` move into a new container of the opposite
    /// orientation, in the sibling's slot, with `win` on the near side. False when there is
    /// no such sibling (`AeroSpace`'s join-with).
    pub fn join(&mut self, win: WinId, orientation: Orientation, forward: bool) -> bool {
        let Some(path) = self.path(win) else { return false };
        let found = (0..path.len()).rev().find_map(|k| {
            let c = self.container(&path[..k])?;
            let j = if forward { path[k].checked_add(1) } else { path[k].checked_sub(1) }?;
            (c.orientation == orientation && j < c.children.len()).then_some((k, j))
        });
        let Some((k, j)) = found else { return false };
        self.unlink(win);
        // Unlinking `win` itself (not a deeper leaf) from this container shifts the slot back.
        let j = if path.len() == k + 1 && path[k] < j { j - 1 } else { j };
        let Some(c) = self.container_mut(&path[..k]) else { return false };
        let target = &mut c.children[j];
        let near = Child { weight: 1.0, node: Node::Window(win) };
        let far =
            Child { weight: 1.0, node: std::mem::replace(&mut target.node, Node::Window(win)) };
        target.node = Node::Split(Container {
            orientation: orientation.opposite(),
            layout: Layout::Tiles,
            children: if forward { vec![near, far] } else { vec![far, near] },
        });
        self.normalize();
        true
    }

    /// Flatten the workspace: every window straight under the root, in order, at equal
    /// weights. The root keeps its layout and orientation.
    pub fn reset(&mut self) {
        let windows = self.windows();
        self.root.children =
            windows.into_iter().map(|w| Child { weight: 1.0, node: Node::Window(w) }).collect();
        self.normalize();
    }

    /// Equal weights everywhere.
    pub fn balance(&mut self) {
        fn even(c: &mut Container) {
            for child in &mut c.children {
                child.weight = 1.0;
                if let Node::Split(inner) = &mut child.node {
                    even(inner);
                }
            }
        }
        even(&mut self.root);
        self.normalize();
    }

    /// Give `win`'s container `layout`; when it already has it, flip the orientation
    /// instead (the key pressed again). Nested orientations alternate from the root, so a
    /// flip turns the root, which turns every container under it. False when `win` is not in
    /// the tree.
    pub fn set_layout(&mut self, win: WinId, layout: Layout) -> bool {
        let Some(path) = self.path(win) else { return false };
        let Some(c) = self.container_mut(&path[..path.len() - 1]) else { return false };
        if c.layout == layout {
            self.root.orientation = self.root.orientation.opposite();
        } else {
            c.layout = layout;
        }
        self.normalize();
        true
    }

    /// Zoom `win` (it takes the whole workspace), or unzoom it when it is zoomed. False when
    /// it is not in the tree.
    pub fn toggle_zoom(&mut self, win: WinId) -> bool {
        if !self.contains(win) {
            return false;
        }
        self.zoom = if self.zoom == Some(win) { None } else { Some(win) };
        true
    }

    /// Grow `win`'s tile by `delta` pixels along `orientation` (negative shrinks it), in the
    /// nearest tiles container from `win` up that splits that way, as laid out in `area`.
    /// The siblings give or take the space in proportion to their sizes, and each tile keeps
    /// at least `MIN_TILE`. The container's weights become its pixel sizes. False when no
    /// container from `win` up can resize that way.
    pub fn resize(
        &mut self,
        win: WinId,
        orientation: Orientation,
        delta: i32,
        area: Frame,
        opts: Opts,
    ) -> bool {
        let Some(path) = self.path(win) else { return false };
        let found = (0..path.len()).rev().find(|&k| {
            self.container(&path[..k]).is_some_and(|c| {
                c.orientation == orientation && c.layout == Layout::Tiles && c.children.len() > 1
            })
        });
        let Some(k) = found else { return false };
        let Some(rect) = layout::rect_at(self, &path[..k], area, opts) else { return false };
        let extent = f64::from(if orientation == Orientation::H { rect.w } else { rect.h });
        let Some(c) = self.container_mut(&path[..k]).filter(|_| extent > 0.0) else {
            return false;
        };
        let (i, n) = (path[k], c.children.len() as f64);
        let total: f64 = c.children.iter().map(|ch| ch.weight).sum();
        let size = |w: f64| w * extent / total;
        let old = size(c.children[i].weight);
        let lo = MIN_TILE.min(extent / n);
        let new = (old + f64::from(delta)).clamp(lo, (extent - lo * (n - 1.0)).max(lo));
        let scale = (extent - new) / (extent - old);
        for (at, child) in c.children.iter_mut().enumerate() {
            child.weight = if at == i { new } else { size(child.weight) * scale };
        }
        self.normalize();
        true
    }
}
