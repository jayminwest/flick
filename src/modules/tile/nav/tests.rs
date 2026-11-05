use super::*;
use crate::modules::tile::workspace::{COUNT, Screen};

/// A tree spec: a window, or a container with its axis, most recent child and children.
enum Spec {
    W(WinId),
    C(Axis, usize, Vec<Spec>),
}

fn h(kids: Vec<Spec>) -> Spec {
    Spec::C(Axis::H, 0, kids)
}

fn v(recent: usize, kids: Vec<Spec>) -> Spec {
    Spec::C(Axis::V, recent, kids)
}

struct Node {
    parent: Option<(usize, usize)>,
    kids: Vec<usize>,
    axis: Axis,
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
        let (axis, recent, kids, window) = match spec {
            Spec::W(w) => (Axis::H, 0, vec![], Some(w)),
            Spec::C(axis, recent, kids) => (axis, recent, kids, None),
        };
        self.0.push(Node { parent, kids: vec![], axis, recent, window });
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
    fn axis(&self, n: usize) -> Axis {
        self.0[n].axis
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
    assert_eq!(Dir::ALL.map(Dir::axis), [Axis::H, Axis::V, Axis::V, Axis::H]);
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
