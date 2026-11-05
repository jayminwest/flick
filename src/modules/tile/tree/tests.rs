use super::*;
use crate::modules::tile::layout::{self, Frame, Opts};
use crate::modules::tile::tree::ops::MIN_TILE;

fn win(n: u32) -> Child {
    Child { weight: 1.0, node: Node::Window(n) }
}

fn weighted(weight: f64, child: Child) -> Child {
    Child { weight, ..child }
}

fn split(orientation: Orientation, layout: Layout, children: Vec<Child>) -> Child {
    Child { weight: 1.0, node: Node::Split(Container { orientation, layout, children }) }
}

fn h(children: Vec<Child>) -> Child {
    split(Orientation::H, Layout::Tiles, children)
}

fn v(children: Vec<Child>) -> Child {
    split(Orientation::V, Layout::Tiles, children)
}

fn tree(orientation: Orientation, children: Vec<Child>) -> Tree {
    Tree::from_root(Container { orientation, layout: Layout::Tiles, children })
}

/// The tree as text: `h[1 v[2 3]]`; an accordion container is marked `a`, as in `ha[1 2]`.
fn shape(t: &Tree) -> String {
    fn show(c: &Container) -> String {
        let o = if c.orientation == Orientation::H { "h" } else { "v" };
        let a = if c.layout == Layout::Accordion { "a" } else { "" };
        let inner: Vec<String> = c
            .children
            .iter()
            .map(|ch| match &ch.node {
                Node::Window(w) => w.to_string(),
                Node::Split(inner) => show(inner),
            })
            .collect();
        format!("{o}{a}[{}]", inner.join(" "))
    }
    show(t.root())
}

fn weights(c: &Container) -> Vec<f64> {
    c.children.iter().map(|ch| ch.weight).collect()
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

/// Every normalization invariant the module doc promises.
fn check(t: &Tree) {
    fn walk(c: &Container, root: bool, seen: &mut Vec<WinId>) {
        assert!(root || c.children.len() > 1, "non-root container with < 2 children");
        for ch in &c.children {
            assert!(ch.weight.is_finite() && ch.weight > 0.0, "bad weight {}", ch.weight);
            match &ch.node {
                Node::Window(w) => {
                    assert!(!seen.contains(w), "window {w:?} twice");
                    seen.push(*w);
                }
                Node::Split(inner) => {
                    assert_ne!(inner.orientation, c.orientation, "nested same orientation");
                    walk(inner, false, seen);
                }
            }
        }
    }
    let mut seen = vec![];
    walk(t.root(), true, &mut seen);
    if let [only] = &t.root().children[..] {
        assert!(matches!(only.node, Node::Window(_)), "root holds a single container");
    }
    assert!(t.zoomed().is_none_or(|z| seen.contains(&z)), "zoomed window not in tree");
    assert!(t.recent.iter().all(|w| seen.contains(w)), "focus history outside the tree");
    let mut history = t.recent.clone();
    history.sort_unstable();
    history.dedup();
    assert_eq!(history.len(), t.recent.len(), "a window twice in the focus history");
    assert_eq!(seen, t.windows());
}

#[test]
fn empty_containers_go_and_single_children_replace_their_container() {
    let t = tree(Orientation::H, vec![win(1), v(vec![]), weighted(3.0, v(vec![win(2)])), win(3)]);
    assert_eq!(shape(&t), "h[1 2 3]");
    assert!(close(&weights(t.root()), &[1.0, 3.0, 1.0]));
    let t = tree(Orientation::H, vec![win(1), v(vec![h(vec![v(vec![win(2)])]), v(vec![])])]);
    assert_eq!(shape(&t), "h[1 2]");
}

#[test]
fn root_absorbs_a_single_container_but_may_hold_a_single_window() {
    let only = split(Orientation::V, Layout::Accordion, vec![win(1), win(2)]);
    assert_eq!(shape(&tree(Orientation::H, vec![only])), "va[1 2]");
    assert_eq!(shape(&tree(Orientation::V, vec![win(1)])), "v[1]");
    assert_eq!(shape(&tree(Orientation::H, vec![h(vec![v(vec![])])])), "h[]");
    assert_eq!(shape(&Tree::default()), "h[]");
}

#[test]
fn nested_containers_alternate_orientation() {
    let t = tree(Orientation::V, vec![win(1), v(vec![win(2), v(vec![win(3), win(4)])])]);
    assert_eq!(shape(&t), "v[1 h[2 v[3 4]]]");
}

#[test]
fn bad_weights_reset_and_duplicate_windows_drop() {
    let t = tree(
        Orientation::H,
        vec![weighted(0.0, win(1)), weighted(f64::NAN, win(2)), weighted(-2.0, win(1))],
    );
    assert_eq!(shape(&t), "h[1 2]");
    assert!(close(&weights(t.root()), &[1.0, 1.0]));
    let t = tree(Orientation::H, vec![weighted(f64::INFINITY, win(1)), win(2)]);
    assert!(close(&weights(t.root()), &[1.0, 1.0]));
}

#[test]
fn reads_paths_containers_and_windows() {
    let t = tree(Orientation::H, vec![win(1), v(vec![win(2), win(3)])]);
    assert_eq!(t.path(3), Some(vec![1, 1]));
    assert_eq!(t.path(9), None);
    assert_eq!(t.container(&[1]).map(|c| c.orientation), Some(Orientation::V));
    assert_eq!(t.container(&[0]), None);
    assert_eq!(t.container(&[5]), None);
    assert!(t.clone().container_mut(&[0]).is_none());
    assert_eq!(t.windows(), [1, 2, 3]);
    assert!(t.contains(2) && !t.contains(4));
}

#[test]
fn insert_goes_after_the_focused_window_at_the_mean_weight() {
    let mut t = Tree::default();
    assert!(t.insert(1, None));
    assert!(t.insert(2, Some(1)));
    assert!(t.insert(3, Some(1)));
    assert_eq!(shape(&t), "h[1 3 2]");
    assert!(!t.insert(3, None));
    let mut t =
        tree(Orientation::H, vec![win(1), v(vec![weighted(1.0, win(2)), weighted(3.0, win(3))])]);
    t.insert(4, Some(2));
    assert_eq!(shape(&t), "h[1 v[2 4 3]]");
    assert!(close(&weights(t.container(&[1]).unwrap()), &[1.0, 2.0, 3.0]));
    t.insert(5, Some(9));
    assert_eq!(shape(&t), "h[1 v[2 4 3] 5]");
}

#[test]
fn remove_lets_the_siblings_fill_and_unzooms() {
    let mut t = tree(Orientation::H, vec![win(1), weighted(2.0, v(vec![win(2), win(3)]))]);
    t.toggle_zoom(2);
    assert!(t.remove(2));
    assert_eq!(shape(&t), "h[1 3]");
    assert!(close(&weights(t.root()), &[1.0, 2.0]));
    assert_eq!(t.zoomed(), None);
    assert!(!t.remove(2));
    t.remove(1);
    t.remove(3);
    assert_eq!(shape(&t), "h[]");
}

#[test]
fn swap_exchanges_windows_and_slots_keep_their_weights() {
    let mut t = tree(Orientation::H, vec![weighted(2.0, win(1)), v(vec![win(2), win(3)])]);
    assert!(t.swap(1, 3));
    assert_eq!(shape(&t), "h[3 v[2 1]]");
    assert!(close(&weights(t.root()), &[2.0, 1.0]));
    assert!(!t.swap(1, 9));
    assert!(!t.swap(9, 1));
}

#[test]
fn join_nests_a_window_with_its_neighbour() {
    let three = || tree(Orientation::H, vec![win(1), win(2), win(3)]);
    let mut t = three();
    assert!(t.join(2, Orientation::H, true));
    assert_eq!(shape(&t), "h[1 v[2 3]]");
    let mut t = three();
    assert!(t.join(2, Orientation::H, false));
    assert_eq!(shape(&t), "h[v[1 2] 3]");
    let mut t = three();
    assert!(t.join(3, Orientation::H, false));
    assert_eq!(shape(&t), "h[1 v[2 3]]");
    let mut t = three();
    assert!(!t.join(3, Orientation::H, true));
    assert!(!t.join(1, Orientation::H, false));
    assert!(!t.join(1, Orientation::V, true));
    assert!(!t.join(9, Orientation::H, true));
    assert_eq!(shape(&t), "h[1 2 3]");
}

#[test]
fn join_climbs_to_a_container_that_splits_that_way() {
    let nested = || tree(Orientation::H, vec![win(1), v(vec![win(2), win(3)])]);
    let mut t = nested();
    assert!(t.join(3, Orientation::H, false));
    assert_eq!(shape(&t), "h[v[1 3] 2]");
    let mut t = nested();
    assert!(t.join(1, Orientation::H, true));
    assert_eq!(shape(&t), "v[1 h[2 3]]");
    let mut t = tree(Orientation::H, vec![win(1), win(2)]);
    assert!(t.join(1, Orientation::H, true));
    assert_eq!(shape(&t), "v[1 2]");
}

#[test]
fn reset_flattens_and_balance_evens_weights() {
    let root = Container {
        orientation: Orientation::V,
        layout: Layout::Accordion,
        children: vec![weighted(3.0, win(1)), v(vec![weighted(2.0, win(2)), win(3)])],
    };
    let mut t = Tree::from_root(root.clone());
    t.balance();
    assert!(close(&weights(t.root()), &[1.0, 1.0]));
    assert!(close(&weights(t.container(&[1]).unwrap()), &[1.0, 1.0]));
    let mut t = Tree::from_root(root);
    t.reset();
    assert_eq!(shape(&t), "va[1 2 3]");
    assert!(close(&weights(t.root()), &[1.0, 1.0, 1.0]));
}

#[test]
fn set_layout_sets_it_then_flips_the_orientation() {
    let mut t = tree(Orientation::H, vec![win(1), win(2)]);
    assert!(t.set_layout(1, Layout::Accordion));
    assert_eq!(shape(&t), "ha[1 2]");
    t.set_layout(1, Layout::Accordion);
    assert_eq!(shape(&t), "va[1 2]");
    t.set_layout(2, Layout::Tiles);
    assert_eq!(shape(&t), "v[1 2]");
    assert!(!t.set_layout(9, Layout::Tiles));
    let mut t = tree(Orientation::H, vec![win(1), v(vec![win(2), win(3)])]);
    t.set_layout(2, Layout::Tiles);
    assert_eq!(shape(&t), "v[1 h[2 3]]");
}

#[test]
fn toggle_zoom_zooms_and_unzooms() {
    let mut t = tree(Orientation::H, vec![win(1), win(2)]);
    assert!(t.toggle_zoom(1));
    assert_eq!(t.zoomed(), Some(1));
    t.toggle_zoom(2);
    assert_eq!(t.zoomed(), Some(2));
    t.toggle_zoom(2);
    assert_eq!(t.zoomed(), None);
    assert!(!t.toggle_zoom(9));
}

#[test]
fn focus_makes_a_window_the_most_recent_child_of_each_container_above_it() {
    let mut t = tree(Orientation::H, vec![win(1), v(vec![win(2), win(3)]), win(4)]);
    let column = |t: &Tree| t.recent_child(t.container(&[1]).unwrap());
    assert_eq!((t.recent_child(t.root()), column(&t)), (0, 0), "nothing focused: the first");
    assert!(t.focus(3));
    assert_eq!((t.recent_child(t.root()), column(&t)), (1, 1));
    t.focus(4);
    assert_eq!((t.recent_child(t.root()), column(&t)), (2, 1), "the column keeps 3");
    assert!(!t.focus(9));
    t.swap(3, 2);
    assert_eq!(column(&t), 0, "recency follows the window");
    t.remove(4);
    assert_eq!(t.recent_child(t.root()), 1, "then the one focused before");
    t.remove(2);
    assert_eq!(t.recent, [3]);
    assert_eq!(t.windows(), [1, 3]);
}

const AREA: Frame = Frame { x: 0, y: 0, w: 1000, h: 600 };

fn frame(t: &Tree, w: u32) -> Frame {
    let all = layout::layout(t, AREA, Opts::default());
    all.into_iter().find(|(id, _)| *id == w).unwrap().1
}

#[test]
fn resize_moves_pixels_between_siblings() {
    let mut t = tree(Orientation::H, vec![win(1), v(vec![win(2), win(3)]), win(4)]);
    let o = Opts::default();
    assert!(t.resize(1, Orientation::H, 100, AREA, o));
    assert_eq!(frame(&t, 1).w, 433);
    assert!(close(&weights(t.root()), &[1300.0 / 3.0, 850.0 / 3.0, 850.0 / 3.0]));
    assert!(t.resize(3, Orientation::V, -100, AREA, o));
    assert_eq!((frame(&t, 2).h, frame(&t, 3).h), (400, 200));
    assert!(t.resize(2, Orientation::H, -10_000, AREA, o));
    assert_eq!(frame(&t, 2).w, MIN_TILE as i32);
    t.resize(2, Orientation::H, 10_000, AREA, o);
    assert_eq!(frame(&t, 2).w, 1000 - 2 * MIN_TILE as i32);
    check(&t);
}

#[test]
fn resize_needs_a_tiles_container_that_splits_that_way() {
    let mut t = tree(Orientation::H, vec![win(1), win(2)]);
    let o = Opts::default();
    assert!(!t.resize(1, Orientation::V, 10, AREA, o));
    assert!(!t.resize(9, Orientation::H, 10, AREA, o));
    assert!(!t.resize(1, Orientation::H, 10, Frame { w: 0, ..AREA }, o));
    assert!(t.resize(1, Orientation::H, -100, Frame { w: 30, ..AREA }, o));
    assert!(close(&weights(t.root()), &[15.0, 15.0]));
    t.set_layout(1, Layout::Accordion);
    assert!(!t.resize(1, Orientation::H, 10, AREA, o));
    assert!(!tree(Orientation::H, vec![win(1)]).resize(1, Orientation::H, 10, AREA, o));
}

/// xorshift64: a fixed, dependency-free stream of pseudo-random numbers.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }

    fn win(&mut self) -> WinId {
        self.below(8) as WinId + 1
    }

    fn orientation(&mut self) -> Orientation {
        if self.below(2) == 0 { Orientation::H } else { Orientation::V }
    }
}

/// One random op on `t`; accordions only when `accordion`.
fn random_op(t: &mut Tree, rng: &mut Rng, accordion: bool) {
    let (a, b) = (rng.win(), rng.win());
    let focused = if rng.below(4) == 0 { None } else { Some(b) };
    let layout = if accordion && rng.below(2) == 0 { Layout::Accordion } else { Layout::Tiles };
    let opts = Opts { stacked: rng.below(2) == 0 };
    if let Some(f) = focused {
        t.focus(f);
    }
    let delta = rng.below(400) as i32 - 200;
    let _ = match rng.below(12) {
        0..=3 => t.insert(a, focused),
        4 => t.remove(a),
        5 => t.swap(a, b),
        6 | 7 => t.join(a, rng.orientation(), rng.below(2) == 0),
        8 => t.set_layout(a, layout),
        9 => t.toggle_zoom(a),
        10 => t.resize(a, rng.orientation(), delta, AREA, opts),
        _ if rng.below(4) == 0 => {
            t.reset();
            true
        }
        _ => {
            t.balance();
            true
        }
    };
}

fn has_accordion(c: &Container) -> bool {
    c.layout == Layout::Accordion
        || c.children.iter().any(|ch| matches!(&ch.node, Node::Split(i) if has_accordion(i)))
}

fn inside(f: Frame, area: Frame) -> bool {
    f.x >= area.x && f.y >= area.y && f.x + f.w <= area.x + area.w && f.y + f.h <= area.y + area.h
}

fn overlap(a: Frame, b: Frame) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Every window gets a frame inside the workspace; in a tiles-only tree, unzoomed, the
/// frames cover it exactly, with no gaps or overlaps. True when it checked that.
fn check_layout(t: &Tree, area: Frame) -> bool {
    let frames = layout::layout(t, area, Opts::default());
    assert_eq!(frames.iter().map(|(w, _)| *w).collect::<Vec<_>>(), t.windows());
    assert!(frames.iter().all(|(_, f)| inside(*f, area) && f.w >= 0 && f.h >= 0));
    if has_accordion(t.root()) || t.zoomed().is_some() || frames.is_empty() {
        return false;
    }
    let covered: i32 = frames.iter().map(|(_, f)| f.w * f.h).sum();
    assert_eq!(covered, area.w * area.h, "{} {frames:?}", shape(t));
    for (i, (_, a)) in frames.iter().enumerate() {
        assert!(frames[i + 1..].iter().all(|(_, b)| !overlap(*a, *b)), "{frames:?}");
    }
    true
}

fn depth(c: &Container) -> usize {
    1 + c
        .children
        .iter()
        .map(|ch| if let Node::Split(i) = &ch.node { depth(i) } else { 0 })
        .max()
        .unwrap_or(0)
}

#[test]
fn every_op_sequence_stays_normalized_and_tiles_exactly() {
    let area = Frame { x: -1440, y: 25, w: 1441, h: 877 };
    let (mut exact, mut deep) = (0, 0);
    for seed in 1..=300u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut t = Tree::default();
        let accordion = seed % 3 == 0;
        for _ in 0..60 {
            random_op(&mut t, &mut rng, accordion);
            check(&t);
            exact += usize::from(check_layout(&t, area));
            deep += usize::from(depth(t.root()) >= 3);
        }
    }
    assert!(
        exact > 1000 && deep > 300,
        "the ops built too few trees worth checking: {exact} {deep}"
    );
}
