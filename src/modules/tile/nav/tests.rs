use super::*;
use crate::modules::tile::tree::{self as split, Container};
use crate::modules::tile::workspace::{COUNT, Screen};

/// A tree spec: a window, or a container with its orientation, most recent child and
/// children.
enum Spec {
    W(WinId),
    C(Orientation, usize, Vec<Spec>),
}

fn h(kids: Vec<Spec>) -> Spec {
    Spec::C(Orientation::H, 0, kids)
}

fn v(recent: usize, kids: Vec<Spec>) -> Spec {
    Spec::C(Orientation::V, recent, kids)
}

struct Node {
    parent: Option<(usize, usize)>,
    kids: Vec<usize>,
    orientation: Orientation,
    recent: usize,
    window: Option<WinId>,
}

/// A plain arena tree; node 0 is the root.
#[derive(Default)]
struct Arena(Vec<Node>);

impl Arena {
    fn of(spec: Spec) -> Arena {
        let mut a = Arena::default();
        a.add(spec, None);
        a
    }

    fn add(&mut self, spec: Spec, parent: Option<(usize, usize)>) -> usize {
        let id = self.0.len();
        let (orientation, recent, kids, window) = match spec {
            Spec::W(w) => (Orientation::H, 0, vec![], Some(w)),
            Spec::C(orientation, recent, kids) => (orientation, recent, kids, None),
        };
        self.0.push(Node { parent, kids: vec![], orientation, recent, window });
        for (i, kid) in kids.into_iter().enumerate() {
            let k = self.add(kid, Some((id, i)));
            self.0[id].kids.push(k);
        }
        id
    }
}

impl Tree for Arena {
    type Node = usize;
    fn root(&self) -> usize {
        0
    }
    fn leaf(&self, w: WinId) -> Option<usize> {
        self.0.iter().position(|n| n.window == Some(w))
    }
    fn parent(&self, n: usize) -> Option<(usize, usize)> {
        self.0[n].parent
    }
    fn child(&self, n: usize, i: usize) -> Option<usize> {
        self.0[n].kids.get(i).copied()
    }
    fn len(&self, n: usize) -> usize {
        self.0[n].kids.len()
    }
    fn orientation(&self, n: usize) -> Orientation {
        self.0[n].orientation
    }
    fn recent(&self, n: usize) -> Option<usize> {
        self.child(n, self.0[n].recent)
    }
    fn window(&self, n: usize) -> Option<WinId> {
        self.0[n].window
    }
}

/// `1 | (2 over 3, 3 focused last) | 4`.
fn tree() -> Arena {
    Arena::of(h(vec![Spec::W(1), v(1, vec![Spec::W(2), Spec::W(3)]), Spec::W(4)]))
}

fn rect(x: f64, y: f64) -> Rect {
    Rect { x, y, w: 1000.0, h: 800.0 }
}

#[test]
fn keys_name_directions() {
    let keys: Vec<_> = "hjkl".chars().map(Dir::from_key).collect();
    assert_eq!(keys, Dir::ALL.map(Some));
    assert_eq!(Dir::from_key('x'), None);
    let back: Vec<_> = Dir::ALL.map(Dir::opposite).to_vec();
    assert_eq!(back, [Dir::Right, Dir::Up, Dir::Down, Dir::Left]);
    let (h, v) = (Orientation::H, Orientation::V);
    assert_eq!(Dir::ALL.map(Dir::orientation), [h, v, v, h]);
}

#[test]
fn a_neighbour_is_found_by_walking_up_to_a_container_along_the_axis() {
    let t = tree();
    assert_eq!(neighbour(&t, 1, Dir::Right), Some(3), "into a split: its most recent child");
    assert_eq!(neighbour(&t, 4, Dir::Left), Some(3));
    assert_eq!(neighbour(&t, 2, Dir::Right), Some(4), "out of a split, up a level");
    assert_eq!(neighbour(&t, 2, Dir::Left), Some(1));
    assert_eq!(neighbour(&t, 3, Dir::Up), Some(2));
    assert_eq!(neighbour(&t, 2, Dir::Down), Some(3));
}

#[test]
fn the_tree_edge_has_no_neighbour() {
    let t = tree();
    assert_eq!(neighbour(&t, 1, Dir::Left), None);
    assert_eq!(neighbour(&t, 4, Dir::Right), None);
    assert_eq!(neighbour(&t, 2, Dir::Up), None);
    assert_eq!(neighbour(&t, 1, Dir::Down), None);
    assert_eq!(neighbour(&t, 9, Dir::Right), None, "not in this tree");
}

#[test]
fn entering_a_split_snaps_to_its_near_edge() {
    let t = Arena::of(h(vec![Spec::W(1), h(vec![Spec::W(2), Spec::W(3)])]));
    assert_eq!(neighbour(&t, 1, Dir::Right), Some(2));
    let t = Arena::of(h(vec![h(vec![Spec::W(1), Spec::W(2)]), Spec::W(3)]));
    assert_eq!(neighbour(&t, 3, Dir::Left), Some(2));
}

#[test]
fn join_targets_the_neighbouring_branch() {
    let t = tree();
    let split = t.parent(t.leaf(2).unwrap()).unwrap().0;
    assert_eq!(join_target(&t, 1, Dir::Right), Some(split));
    assert_eq!(join_target(&t, 3, Dir::Right), t.leaf(4));
    assert_eq!(join_target(&t, 1, Dir::Left), None);
    assert_eq!(join_target(&t, 9, Dir::Left), None);
}

#[test]
fn focus_enters_a_tree_at_the_edge_it_comes_through() {
    let t = tree();
    assert_eq!(entry(&t, Dir::Right), Some(1));
    assert_eq!(entry(&t, Dir::Left), Some(4));
    assert_eq!(entry(&t, Dir::Down), Some(1), "across the axis: the most recent child");
    let t = Arena::of(v(0, vec![h(vec![Spec::W(5), Spec::W(6)]), Spec::W(7)]));
    assert_eq!(entry(&t, Dir::Up), Some(7));
    assert_eq!(entry(&t, Dir::Down), Some(5));
    assert_eq!(entry(&Arena::of(Spec::W(8)), Dir::Left), Some(8), "a lone window");
}

#[test]
fn an_empty_tree_has_no_entry() {
    let t = Arena::of(h(vec![]));
    for dir in Dir::ALL {
        assert_eq!(entry(&t, dir), None, "{dir:?}");
    }
}

#[test]
fn the_screen_that_way_is_the_nearest_within_45_degrees() {
    let areas = [rect(0.0, 0.0), rect(1000.0, 300.0), rect(-1000.0, 0.0), rect(0.0, -800.0)];
    assert_eq!(screen_toward(&areas, 0, Dir::Right), Some(1), "staggered still counts");
    assert_eq!(screen_toward(&areas, 0, Dir::Left), Some(2));
    assert_eq!(screen_toward(&areas, 0, Dir::Up), Some(3));
    assert_eq!(screen_toward(&areas, 0, Dir::Down), None);
    assert_eq!(screen_toward(&areas, 2, Dir::Right), Some(0), "the nearer of two");
    assert_eq!(screen_toward(&areas, 7, Dir::Right), None);
}

/// Laptop in the middle, DELL right, tablet left; no pins, so they show 1, 2 and 3.
fn desk() -> Workspaces {
    let screen =
        |id, name: &str, x| Screen { id, name: name.into(), built_in: id == 1, area: rect(x, 0.0) };
    let mut ws = Workspaces::new(std::array::from_fn(|_| vec![]));
    ws.set_screens(vec![
        screen(1, "Built-in", 0.0),
        screen(2, "DELL", 1000.0),
        screen(3, "MD130FH", -1000.0),
    ]);
    ws
}

#[test]
fn focus_crosses_to_the_next_screen_at_the_tree_edge() {
    let ws = desk();
    let trees: Vec<Arena> = (1..=COUNT)
        .map(|n| match n {
            1 => tree(),
            2 => Arena::of(h(vec![Spec::W(8), Spec::W(9)])),
            _ => Arena::of(h(vec![])),
        })
        .collect();
    let of = |w: Ws| trees.get(w.number() - 1);
    assert_eq!(focus_target(&ws, Some(1), Dir::Right, of), Some(Target::Window(3)));
    assert_eq!(focus_target(&ws, Some(4), Dir::Right, of), Some(Target::Window(8)));
    assert_eq!(focus_target(&ws, None, Dir::Right, of), Some(Target::Window(8)));
    assert_eq!(focus_target(&ws, Some(1), Dir::Left, of), Some(Target::Screen(2)), "empty there");
    assert_eq!(focus_target(&ws, Some(1), Dir::Up, of), None);
}

#[test]
fn focus_crosses_from_a_workspace_without_a_tree_and_stops_without_screens() {
    let ws = desk();
    let none = |_: Ws| None::<&Arena>;
    assert_eq!(focus_target(&ws, Some(1), Dir::Right, none), Some(Target::Screen(1)));
    let bare = Workspaces::new(std::array::from_fn(|_| vec![]));
    assert_eq!(focus_target(&bare, Some(1), Dir::Right, none), None);
}

/// The arena with `t`'s shape and most recent children.
fn arena_of(t: &split::Tree) -> Arena {
    fn spec(t: &split::Tree, c: &Container) -> Spec {
        let kids = c.children.iter().map(|ch| match &ch.node {
            split::Node::Window(w) => Spec::W(*w),
            split::Node::Split(inner) => spec(t, inner),
        });
        Spec::C(c.orientation, t.recent_child(c), kids.collect())
    }
    Arena::of(spec(t, t.root()))
}

/// The first window under node `n` and how many children `n` has, to compare nodes of two
/// kinds of tree.
fn sketch<T: Tree>(t: &T, n: T::Node) -> (Option<WinId>, usize) {
    fn first<T: Tree>(t: &T, n: T::Node) -> Option<WinId> {
        t.window(n).or_else(|| first(t, t.child(n, 0)?))
    }
    (first(t, n), t.len(n))
}

/// Every walk over the real tree `t` gives what it gives over the plain arena of it.
fn agrees(t: &split::Tree) {
    let a = arena_of(t);
    for dir in Dir::ALL {
        assert_eq!(entry(t, dir), entry(&a, dir), "entry {dir:?}");
        for w in 0..=9 {
            assert_eq!(neighbour(t, w, dir), neighbour(&a, w, dir), "{w} {dir:?}");
            let (real, plain) = (join_target(t, w, dir), join_target(&a, w, dir));
            assert_eq!(real.map(|n| sketch(t, n)), plain.map(|n| sketch(&a, n)), "{w} {dir:?}");
        }
    }
}

/// `1 | (2 over 3) | 4`, as a real tree.
fn real() -> split::Tree {
    let mut t = split::Tree::default();
    for w in 1..=4 {
        t.insert(w);
    }
    t.join(2, Orientation::H, true);
    t
}

#[test]
fn the_walks_read_the_real_tree_and_its_focus_history() {
    let mut t = real();
    agrees(&t);
    assert_eq!(neighbour(&t, 1, Dir::Right), Some(2), "nothing focused: the first child");
    t.focus(3);
    t.focus(4);
    agrees(&t);
    assert_eq!(neighbour(&t, 1, Dir::Right), Some(3), "into a split: its most recent child");
    assert_eq!(neighbour(&t, 3, Dir::Up), Some(2));
    assert_eq!(entry(&t, Dir::Down), Some(4), "across the axis: the most recent child");
    t.remove(3);
    assert_eq!(neighbour(&t, 1, Dir::Right), Some(2));
    agrees(&split::Tree::default());
}

#[test]
fn the_walks_agree_on_deep_trees_and_accordions() {
    let mut t = real();
    for w in 5..=8 {
        t.focus(w - 2);
        t.insert(w);
    }
    t.join(5, Orientation::V, true);
    t.join(2, Orientation::V, true);
    t.join(8, Orientation::H, false);
    t.set_layout(5, split::Layout::Accordion);
    assert_eq!(t.path(5), Some(vec![1, 1, 0]), "three levels deep");
    for w in [7, 2, 5, 8] {
        t.focus(w);
        agrees(&t);
    }
}

#[test]
fn real_tree_nodes_have_one_handle_each() {
    let t = real();
    let root = Tree::root(&t);
    let col = t.child(root, 1).unwrap();
    assert_eq!(t.parent(t.leaf(3).unwrap()), Some((col, 1)), "found from below or above");
    assert_eq!(t.parent(col), Some((root, 1)));
    assert_eq!(t.parent(root), None);
    assert_eq!((t.len(col), t.orientation(col), t.window(col)), (2, Orientation::V, None));
    let leaf = t.leaf(2).unwrap();
    assert_eq!((t.len(leaf), t.window(leaf), t.recent(leaf)), (0, Some(2), None));
    assert_eq!(t.orientation(leaf), Orientation::H, "a leaf has none: the default");
    assert_eq!((t.child(root, 3), t.child(leaf, 0), t.leaf(9)), (None, None, None));
    let empty = split::Tree::default();
    assert_eq!((empty.len(root), empty.recent(root), empty.window(root)), (0, None, None));
}

#[test]
fn a_handle_from_before_a_change_finds_nothing_or_the_node_now_there() {
    let mut t = real();
    let deep = t.leaf(3).unwrap();
    t.remove(2);
    assert_eq!((t.window(deep), t.parent(deep), t.len(deep)), (None, None, 0));
    assert_eq!(t.leaf(3).and_then(|n| t.window(n)), Some(3));
}
