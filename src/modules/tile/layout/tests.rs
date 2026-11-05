use super::*;
use crate::modules::tile::tree::Child;

const AREA: Frame = Frame { x: 0, y: 25, w: 1000, h: 600 };
const P: i32 = ACCORDION_PADDING;

fn win(n: u32) -> Child {
    Child { weight: 1.0, node: Node::Window(n) }
}

fn split(orientation: Orientation, layout: Layout, children: Vec<Child>) -> Child {
    Child { weight: 1.0, node: Node::Split(Container { orientation, layout, children }) }
}

fn tree(orientation: Orientation, layout: Layout, children: Vec<Child>) -> Tree {
    Tree::from_root(Container { orientation, layout, children })
}

fn frame(x: i32, y: i32, w: i32, h: i32) -> Frame {
    Frame { x, y, w, h }
}

/// The frames alone, in window order.
fn frames_of(t: &Tree, area: Frame, opts: Opts) -> Vec<Frame> {
    layout(t, area, opts).into_iter().map(|(_, f)| f).collect()
}

/// `t` with window `n` focused.
fn focus(mut t: Tree, n: u32) -> Tree {
    assert!(t.focus(n));
    t
}

#[test]
fn an_empty_workspace_has_no_frames() {
    assert!(layout(&Tree::default(), AREA, Opts::default()).is_empty());
}

#[test]
fn tiles_split_by_weight_and_round_each_edge() {
    let t = tree(Orientation::H, Layout::Tiles, vec![win(1), win(2), win(3)]);
    let got = layout(&t, AREA, Opts::default());
    assert_eq!(
        got,
        [(1, frame(0, 25, 333, 600)), (2, frame(333, 25, 334, 600)), (3, frame(667, 25, 333, 600)),]
    );
    let t = tree(Orientation::V, Layout::Tiles, vec![Child { weight: 3.0, ..win(1) }, win(2)]);
    assert_eq!(
        frames_of(&t, AREA, Opts::default()),
        [frame(0, 25, 1000, 450), frame(0, 475, 1000, 150)]
    );
}

#[test]
fn nested_tiles_split_their_own_frame() {
    let inner = split(Orientation::V, Layout::Tiles, vec![win(2), win(3)]);
    let t = tree(Orientation::H, Layout::Tiles, vec![win(1), inner]);
    assert_eq!(
        frames_of(&t, AREA, Opts::default()),
        [frame(0, 25, 500, 600), frame(500, 25, 500, 300), frame(500, 325, 500, 300)]
    );
}

#[test]
fn stacked_screens_lose_the_bottom_pixel() {
    let t = tree(Orientation::V, Layout::Tiles, vec![win(1), win(2)]);
    let opts = Opts { stacked: true };
    assert_eq!(frames_of(&t, AREA, opts), [frame(0, 25, 1000, 300), frame(0, 325, 1000, 299)]);
}

#[test]
fn an_accordion_pads_around_the_focused_child() {
    let five = (1..=5).map(win).collect();
    let t = tree(Orientation::H, Layout::Accordion, five);
    let spans: Vec<(i32, i32)> =
        frames_of(&focus(t.clone(), 3), AREA, Opts::default()).iter().map(|f| (f.x, f.w)).collect();
    assert_eq!(
        spans,
        [(0, 1000 - P), (0, 1000 - 2 * P), (P, 1000 - 2 * P), (2 * P, 1000 - 2 * P), (P, 1000 - P)]
    );
    let spans: Vec<(i32, i32)> =
        frames_of(&t, AREA, Opts::default()).iter().map(|f| (f.x, f.w)).collect();
    assert_eq!(spans[1], (2 * P, 1000 - 2 * P));
    assert_eq!(spans[2], (P, 1000 - 2 * P));
}

#[test]
fn an_accordion_of_one_fills_and_a_vertical_one_pads_heights() {
    let t = tree(Orientation::V, Layout::Accordion, vec![win(1)]);
    assert_eq!(frames_of(&t, AREA, Opts::default()), [AREA]);
    let t = tree(Orientation::V, Layout::Accordion, vec![win(1), win(2)]);
    assert_eq!(
        frames_of(&focus(t, 2), AREA, Opts::default()),
        [frame(0, 25, 1000, 600 - P), frame(0, 25 + P, 1000, 600 - P)]
    );
}

#[test]
fn an_accordion_tops_the_child_that_holds_focus() {
    let inner = split(Orientation::H, Layout::Tiles, vec![win(3), win(4)]);
    let t = tree(Orientation::V, Layout::Accordion, vec![win(1), win(2), inner, win(5)]);
    let ys: Vec<i32> =
        frames_of(&focus(t, 4), AREA, Opts::default()).iter().map(|f| f.y - 25).collect();
    assert_eq!(ys, [0, 0, P, P, P]);
}

#[test]
fn an_accordion_off_the_focus_keeps_its_most_recent_child_on_top() {
    let inner = split(Orientation::V, Layout::Accordion, vec![win(2), win(3), win(5), win(6)]);
    let t = tree(Orientation::H, Layout::Tiles, vec![win(1), inner]);
    let mut t = focus(focus(focus(t, 5), 3), 1);
    let ys = |t: &Tree| -> Vec<i32> {
        frames_of(t, AREA, Opts::default()).iter().map(|f| f.y - 25).collect()
    };
    assert_eq!(ys(&t), [0, 0, P, 2 * P, P], "3 stays on top after focus moves to 1");
    t.remove(3);
    assert_eq!(ys(&t), [0, 0, P, P], "then the one focused before it");
    t.remove(5);
    t.insert(7, Some(6));
    assert_eq!(ys(&t), [0, 0, 2 * P, P], "none focused under it: the first");
}

#[test]
fn an_accordion_narrower_than_its_padding_stays_inside() {
    let t = tree(Orientation::H, Layout::Accordion, vec![win(1), win(2), win(3)]);
    let area = frame(0, 0, 20, 100);
    assert_eq!(
        frames_of(&t, area, Opts::default()),
        [frame(0, 0, 0, 100), frame(20, 0, 0, 100), frame(20, 0, 0, 100)]
    );
}

#[test]
fn a_zoomed_window_takes_the_workspace() {
    let mut t = tree(Orientation::H, Layout::Tiles, vec![win(1), win(2)]);
    t.toggle_zoom(2);
    let opts = Opts { stacked: true };
    assert_eq!(frames_of(&t, AREA, opts), [frame(0, 25, 500, 599), frame(0, 25, 1000, 599)]);
}

#[test]
fn rect_at_follows_a_path_of_containers() {
    let inner = split(Orientation::V, Layout::Tiles, vec![win(2), win(3)]);
    let t = tree(Orientation::H, Layout::Tiles, vec![win(1), inner]);
    let o = Opts::default();
    assert_eq!(rect_at(&t, &[], AREA, o), Some(AREA));
    assert_eq!(rect_at(&t, &[1], AREA, o), Some(frame(500, 25, 500, 600)));
    assert_eq!(rect_at(&t, &[0], AREA, o), None);
    assert_eq!(rect_at(&t, &[2], AREA, o), None);
}
