//! The root item and the `fleet` and `machine` views (flick-9eb1).

use super::*;

fn ids(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.id.as_str()).collect()
}

fn root(m: &mut Sys) -> Vec<Item> {
    test_cx("", |cx| m.items(cx))
}

fn open(m: &mut Sys, view: &str) -> Option<ListView> {
    test_cx("", |cx| m.open(view, cx))
}

fn shown(m: &mut Sys, view: &str, query: &str) -> ListView {
    let mut v = open(m, view).unwrap();
    test_cx(query, |cx| m.refresh(&mut v, cx));
    v
}

fn close(m: &mut Sys, view: &str) {
    test_cx("", |cx| m.closed(view, cx));
}

fn enter(m: &mut Sys, key: &str) -> Outcome {
    test_cx("", |cx| m.activate(&ItemId::new(ID, key), cx))
}

fn pushed(o: &Outcome) -> Option<(&str, &str)> {
    match o {
        Outcome::Push(v) => Some((v.module, v.name.as_str())),
        _ => None,
    }
}

#[test]
fn no_machines_no_root_item_and_nothing_polls() {
    let mut m = sys(SERVICES, Hooks { visible: || true, ..HOOKS });
    event(&mut m, Event::Started);
    assert!(root(&mut m).is_empty());
    // Pushed by name anyway: an empty view with the hint, and still no round.
    let v = shown(&mut m, "fleet", "");
    assert!(v.items.is_empty());
    assert_eq!(v.empty, "No machines (add [[sys.machine]] tables to config.toml)");
    event(&mut m, Event::ModuleChanged { module: ID });
    let st = m.shared.lock();
    assert!(!st.fleet.busy() && !st.probing && !st.checking && st.snapshot.is_none());
}

#[test]
fn root_item_opens_the_fleet() {
    let mut m = sys(FLEET, Hooks { visible: || true, ..HOOKS });
    event(&mut m, Event::Started);
    let items = root(&mut m);
    assert_eq!(ids(&items), ["sys:fleet"]);
    assert_eq!(items[0].subtitle, "3 machines");
    assert_eq!(tried(&m), [None, None, None], "listing the root item polls nothing");
    assert_eq!(pushed(&enter(&mut m, "fleet")), Some((ID, "fleet")));
    let v = shown(&mut m, "fleet", "");
    assert_eq!(m.fleet_view, Some("fleet"));
    assert_eq!(v.placeholder, "Search machines and services…");
    let want = ["sys:machine/laptop", "sys:machine/server", "sys:machine/pro", "sys:service/pro/ollama", "sys:agents"];
    settle(&m);
    assert_eq!(tried(&m), [None, Some(1_000), Some(1_000)], "opening the view polled");
    let v = shown(&mut m, "fleet", "");
    assert_eq!(ids(&v.items), want);
    assert!(v.footer.starts_with("3 machines") && v.footer.ends_with("  ·  esc to go back"), "{}", v.footer);
    assert_eq!(ids(&shown(&mut m, "fleet", "ollama").items)[0], "sys:service/pro/ollama");
    settle(&m);
}

/// Mark the server read at 980, tick, and say whether it was read again.
fn tick_polls(m: &mut Sys) -> bool {
    m.shared.lock().fleet.slots[1].tried_at = Some(980);
    event(m, Event::ModuleChanged { module: ID });
    settle(m);
    tried(m)[1] == Some(1_000)
}

#[test]
fn the_fleet_polls_only_while_its_view_shows() {
    let mut m = sys(FLEET, Hooks { visible: || true, ..HOOKS });
    event(&mut m, Event::Started);
    open(&mut m, "fleet").unwrap();
    settle(&m);
    assert!(tick_polls(&mut m), "15 s on, the open view polls again");
    // Root search, listing items, does not stop it; the launcher closing the view does.
    root(&mut m);
    assert!(tick_polls(&mut m));
    close(&mut m, "fleet");
    assert_eq!(m.fleet_view, None);
    assert!(!tick_polls(&mut m));
}

#[test]
fn a_fleet_of_only_this_mac_keeps_polling_while_its_view_shows() {
    let mut m = sys("[[sys.machine]]\nname = \"laptop\"\nvia = \"local\"\n", Hooks { visible: || true, ..HOOKS });
    m.tick = Duration::from_millis(20);
    event(&mut m, Event::Started);
    open(&mut m, "fleet").unwrap();
    settle(&m);
    let at = |m: &Sys| m.shared.lock().snapshot.as_ref().map(|s| s.at);
    assert_eq!(at(&m), Some(1_000));
    // No round runs (no remote machine), yet a tick comes due on its own (flick-1e00).
    assert!(m.shared.wait(Duration::from_secs(15), |s| !s.fleet.ticking), "a tick was pending");
    m.shared.lock().snapshot.as_mut().unwrap().at = 1_000 - VISIBLE_DUE;
    event(&mut m, Event::ModuleChanged { module: ID });
    settle(&m);
    assert_eq!(at(&m), Some(1_000), "the tick's poll refreshed this Mac's cache");
    // Back at root search the view is gone: the pending tick is the last.
    close(&mut m, "fleet");
    assert!(m.shared.wait(Duration::from_secs(15), |s| !s.fleet.ticking));
    event(&mut m, Event::ModuleChanged { module: ID });
    assert!(!m.shared.lock().fleet.ticking);
}

#[test]
fn only_closing_the_open_fleet_view_stops_its_cadence() {
    let mut m = sys(FLEET, HOOKS);
    open(&mut m, "fleet").unwrap();
    event(&mut m, Event::Started);
    event(&mut m, Event::LauncherOpened);
    assert_eq!(m.fleet_view, Some("fleet"), "the launcher closes the view, not the event");
    // Fleet to machine: `open` of the new view comes before `closed` of the old one.
    m.detail = Some("pro".into());
    open(&mut m, "machine").unwrap();
    close(&mut m, "fleet");
    close(&mut m, "log");
    assert_eq!(m.fleet_view, Some("machine"));
    close(&mut m, "machine");
    assert_eq!(m.fleet_view, None);
    settle(&m);
}

#[test]
fn enter_on_a_machine_or_service_opens_the_machine() {
    let mut m = sys(FLEET, HOOKS);
    assert!(open(&mut m, "machine").is_none(), "no machine picked yet");
    assert!(open(&mut m, "other").is_none());
    assert_eq!(pushed(&enter(&mut m, "service/pro/ollama")), Some((ID, "machine")));
    assert_eq!(m.detail.as_deref(), Some("pro"));
    assert_eq!(pushed(&enter(&mut m, "machine/server")), Some((ID, "machine")));
    assert_eq!(m.detail.as_deref(), Some("server"));
    let v = shown(&mut m, "machine", "");
    assert_eq!(v.placeholder, "Search this machine…");
    assert_eq!(ids(&v.items), ["sys:head", "sys:agents"]);
    assert_eq!(v.footer, "server · pending via flick  ·  esc to go back");
    // Head, facts and Fleet go back to the fleet.
    for key in ["head", "fact/0", "fleet"] {
        assert_eq!(pushed(&enter(&mut m, key)), Some((ID, "fleet")), "{key}");
    }
    assert!(matches!(enter(&mut m, "service/gone/x"), Outcome::Stay(None)));
}

fn status(o: Outcome) -> Option<String> {
    match o {
        Outcome::Stay(s) => s,
        _ => Some("not a Stay".into()),
    }
}

#[test]
fn enter_on_a_check_says_its_status() {
    let mut m = sys(FLEET, HOOKS);
    event(&mut m, Event::Started);
    m.detail = Some("pro".into());
    open(&mut m, "machine").unwrap();
    settle(&m);
    let v = shown(&mut m, "machine", "");
    assert!(ids(&v.items).contains(&"sys:check/ollama"), "{:?}", ids(&v.items));
    assert_eq!(status(enter(&mut m, "check/ollama")).as_deref(), Some("ollama: ok · running (pid 812)"));
    assert_eq!(status(enter(&mut m, "check/gone")), None);
    m.detail = Some("gone".into());
    assert_eq!(status(enter(&mut m, "check/ollama")), None);
}

#[test]
fn herdr_agents_is_pushed_by_name() {
    let mut m = sys(FLEET, HOOKS);
    open(&mut m, "fleet").unwrap();
    assert_eq!(pushed(&enter(&mut m, "agents")), Some(("herdr", "agents")));
    // With herdr disabled the push opens nothing and the fleet view stays (flick-7638).
    assert_eq!(m.fleet_view, Some("fleet"));
    close(&mut m, "fleet");
    assert_eq!(m.fleet_view, None, "herdr's view replaced the fleet view");
}
