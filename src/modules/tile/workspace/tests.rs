use super::*;

fn ws(n: usize) -> Ws {
    Ws::new(n).unwrap()
}

fn screen(id: u32, name: &str, built_in: bool, x: f64) -> Screen {
    Screen { id, name: name.into(), built_in, area: Rect { x, y: 0.0, w: 1000.0, h: 800.0 } }
}

fn built_in() -> Screen {
    screen(1, "Built-in Retina Display", true, 0.0)
}

fn dell() -> Screen {
    screen(2, "DELL U2723QE", false, 1000.0)
}

fn tablet() -> Screen {
    screen(3, "MD130FH", false, -1000.0)
}

fn lg() -> Screen {
    screen(4, "LG HDR 4K", false, 2000.0)
}

/// The user's pins: 1-2 on the laptop, 3-4 on the DELL (or the only external), 7 on the
/// tablet; 5, 6 and 8 follow focus.
fn pins() -> [Vec<Match>; COUNT] {
    let p = |names: &[&str]| names.iter().map(|n| Match::parse(n)).collect::<Vec<_>>();
    let mut pins: [Vec<Match>; COUNT] = Default::default();
    pins[0] = p(&["built-in"]);
    pins[1] = p(&["built-in"]);
    pins[2] = p(&["DELL", "secondary"]);
    pins[3] = p(&["DELL", "secondary"]);
    pins[6] = p(&["MD130FH"]);
    pins
}

fn with(screens: Vec<Screen>) -> Workspaces {
    let mut w = Workspaces::new(pins());
    w.set_screens(screens);
    w
}

fn desk() -> Workspaces {
    with(vec![built_in(), dell(), tablet()])
}

fn homes(w: &Workspaces) -> Vec<Option<usize>> {
    Ws::all().map(|ws| w.home(ws)).collect()
}

fn shown(w: &Workspaces) -> Vec<usize> {
    (0..w.screens().len()).filter_map(|i| w.shown_on(i)).map(Ws::number).collect()
}

fn plan(show: &[WinId], hide: &[WinId]) -> Plan {
    Plan { show: show.to_vec(), hide: hide.to_vec() }
}

#[test]
fn workspaces_are_numbered_one_to_eight_and_wrap() {
    assert_eq!(Ws::new(0), None);
    assert_eq!(Ws::new(COUNT + 1), None);
    assert_eq!(Ws::all().map(Ws::number).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(ws(1).next(), ws(2));
    assert_eq!(ws(8).next(), ws(1));
    assert_eq!(ws(1).prev(), ws(8));
    assert_eq!(ws(5).prev(), ws(4));
}

#[test]
fn pin_entries_parse_keywords_and_names() {
    assert_eq!(Match::parse("built-in"), Match::BuiltIn);
    assert_eq!(Match::parse("main"), Match::Main);
    assert_eq!(Match::parse("secondary"), Match::Secondary);
    assert_eq!(Match::parse("DELL"), Match::Name("dell".into()));
}

#[test]
fn the_desk_pins_each_workspace_to_its_screen() {
    let w = desk();
    assert_eq!(homes(&w), [Some(0), Some(0), Some(1), Some(1), None, None, Some(2), None]);
    assert_eq!(shown(&w), [1, 3, 7]);
    assert_eq!(w.focused(), ws(1));
    assert_eq!(w.focused_screen(), Some(0));
}

#[test]
fn the_laptop_alone_takes_every_pinned_workspace() {
    let w = with(vec![built_in()]);
    assert_eq!(homes(&w), [Some(0), Some(0), Some(0), Some(0), None, None, Some(0), None]);
    assert_eq!(shown(&w), [1]);
}

#[test]
fn another_single_external_is_secondary() {
    let w = with(vec![built_in(), lg()]);
    assert_eq!(homes(&w), [Some(0), Some(0), Some(1), Some(1), None, None, Some(0), None]);
    assert_eq!(shown(&w), [1, 3]);
}

#[test]
fn without_a_built_in_screen_pins_fall_back_to_main() {
    let w = with(vec![lg(), dell()]);
    assert_eq!(homes(&w), [Some(0), Some(0), Some(1), Some(1), None, None, Some(0), None]);
    let mut pins: [Vec<Match>; COUNT] = Default::default();
    pins[0] = vec![Match::Main];
    let mut w = Workspaces::new(pins);
    assert_eq!(w.home(ws(1)), None, "no screens yet");
    w.set_screens(vec![built_in(), dell()]);
    assert_eq!(w.home(ws(1)), Some(0));
    assert_eq!(w.home(ws(2)), None);
}

#[test]
fn a_screen_with_no_pinned_workspace_gets_an_unpinned_one() {
    let w = with(vec![built_in(), lg(), screen(5, "Sidecar", false, 3000.0)]);
    assert_eq!(shown(&w), [1, 5, 6]);
}

#[test]
fn a_screen_takes_any_workspace_when_all_are_pinned_elsewhere() {
    let mut w = Workspaces::new(std::array::from_fn(|_| vec![Match::BuiltIn]));
    w.set_screens(vec![built_in(), dell()]);
    assert_eq!(shown(&w), [1, 2]);
}

#[test]
fn an_empty_screen_list_is_ignored_and_more_than_eight_are_cut() {
    let mut w = desk();
    assert_eq!(w.set_screens(vec![]), Plan::default());
    assert_eq!(shown(&w), [1, 3, 7]);
    let many: Vec<Screen> = (0..10).map(|i| screen(10 + i, "Wall", false, f64::from(i))).collect();
    w.set_screens(many);
    assert_eq!(w.screens().len(), COUNT);
}

#[test]
fn switching_on_the_same_screen_shows_the_target_then_hides_the_old() {
    let mut w = desk();
    w.put(10, ws(1));
    w.put(20, ws(2));
    w.put(21, ws(2));
    assert_eq!(w.switch(ws(2)), plan(&[20, 21], &[10]));
    assert_eq!(shown(&w), [2, 3, 7]);
    assert_eq!((w.focused(), w.last()), (ws(2), Some(ws(1))));
}

#[test]
fn switching_to_a_workspace_pinned_elsewhere_focuses_that_screen() {
    let mut w = desk();
    w.put(30, ws(3));
    w.put(40, ws(4));
    assert_eq!(w.switch(ws(4)), plan(&[40], &[30]));
    assert_eq!(shown(&w), [1, 4, 7]);
    assert_eq!(w.focused_screen(), Some(1));
}

#[test]
fn switching_to_a_visible_workspace_only_moves_focus() {
    let mut w = desk();
    assert_eq!(w.switch(ws(7)), Plan::default());
    assert_eq!(shown(&w), [1, 3, 7]);
    assert_eq!(w.focused_screen(), Some(2));
}

#[test]
fn an_unpinned_workspace_opens_on_the_focused_screen() {
    let mut w = desk();
    w.switch(ws(3));
    w.switch(ws(5));
    assert_eq!(shown(&w), [1, 5, 7]);
    assert_eq!(w.focused_screen(), Some(1));
}

#[test]
fn switching_to_the_focused_workspace_or_without_screens_does_nothing() {
    let mut w = desk();
    assert_eq!(w.switch(ws(1)), Plan::default());
    assert_eq!(w.last(), None);
    let mut none = Workspaces::new(pins());
    none.put(20, ws(2));
    assert_eq!(none.switch(ws(2)), Plan::default());
    assert_eq!(none.focused(), ws(1));
}

#[test]
fn next_and_prev_wrap_around_all_eight() {
    let mut w = with(vec![built_in()]);
    w.prev();
    assert_eq!(w.focused(), ws(8));
    w.next();
    assert_eq!(w.focused(), ws(1));
    w.next();
    assert_eq!(w.focused(), ws(2));
}

#[test]
fn back_and_forth_toggles_the_last_two() {
    let mut w = desk();
    w.put(10, ws(1));
    assert_eq!(w.back_and_forth(), Plan::default());
    w.switch(ws(2));
    assert_eq!(w.back_and_forth(), plan(&[10], &[]));
    assert_eq!(w.focused(), ws(1));
    w.back_and_forth();
    assert_eq!(w.focused(), ws(2));
}

#[test]
fn focusing_a_screen_focuses_its_workspace() {
    let mut w = desk();
    w.focus_screen(1);
    assert_eq!((w.focused(), w.last()), (ws(3), Some(ws(1))));
    w.focus_screen(1);
    w.focus_screen(9);
    assert_eq!((w.focused(), w.last()), (ws(3), Some(ws(1))));
}

#[test]
fn putting_a_window_hides_or_shows_it_as_it_crosses_the_screen_edge() {
    let mut w = desk();
    assert_eq!(w.put(1, ws(1)), Plan::default(), "new, visible");
    assert_eq!(w.put(5, ws(5)), plan(&[], &[5]), "new, hidden (an app pin)");
    assert_eq!(w.put(1, ws(2)), plan(&[], &[1]), "sent to a hidden workspace");
    assert_eq!(w.put(1, ws(6)), Plan::default(), "hidden to hidden");
    assert_eq!(w.put(1, ws(3)), plan(&[1], &[]), "back on screen");
    assert_eq!(w.put(5, ws(7)), plan(&[5], &[]));
    assert_eq!(w.workspace_of(1), Some(ws(3)));
    w.put(2, ws(3));
    assert_eq!(w.windows_on(ws(3)), [1, 2]);
    w.remove(1);
    assert_eq!((w.workspace_of(1), w.windows_on(ws(3))), (None, vec![2]));
}

#[test]
fn unplugging_the_dell_hides_its_workspace_and_focus_moves_to_main() {
    let mut w = desk();
    w.put(30, ws(3));
    w.put(70, ws(7));
    w.switch(ws(3));
    assert_eq!(w.set_screens(vec![built_in(), tablet()]), plan(&[], &[30]));
    assert_eq!(shown(&w), [1, 7]);
    assert_eq!(w.focused(), ws(1));
}

#[test]
fn plugging_the_dell_in_moves_its_workspace_there() {
    let mut w = with(vec![built_in()]);
    w.put(10, ws(1));
    w.put(40, ws(4));
    w.put(70, ws(7));
    w.switch(ws(4));
    let p = w.set_screens(vec![built_in(), dell(), tablet()]);
    assert_eq!(p, plan(&[10, 70], &[]));
    assert_eq!(shown(&w), [1, 4, 7]);
    assert_eq!(w.focused(), ws(4), "the focused workspace stays focused on its new screen");
}
