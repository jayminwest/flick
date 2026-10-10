//! The menu bar item over `testkit::UI`: never the real menu bar or notifications.

use super::*;
use crate::config::parse;
use crate::core::{Event, Module, test_cx};
use crate::modules::kota::presence::Transition;
use crate::modules::kota::testkit::{queue, take_ui};
use crate::modules::kota::tests::{configured, event, wait};
use crate::modules::kota::{ID, Kota};

const SERVER: &str = "[kota]\nmachine = \"server\"\ndash = \"http://ok\"\n";
const THINKING: &str = "show K… | KOTA: thinking · <1m\nFlick KOTA cards design brainstorm | 9 rows";

fn reconfigure(k: &mut Kota, text: &str) {
    k.configure(&parse(text).unwrap().section(ID).unwrap().unwrap()).unwrap();
}

/// Started, then the first round ended and its `ModuleChanged` arrived.
fn started(text: &str) -> Kota {
    let mut k = configured(text).unwrap();
    event(&mut k, Event::Started);
    wait("first round", || {
        let p = k.shared.lock();
        !p.running && p.ended_at.is_some()
    });
    event(&mut k, Event::ModuleChanged { module: ID });
    k
}

#[test]
fn no_kota_key_means_no_item() {
    let mut k = configured("").unwrap();
    for e in [Event::Started, Event::CardsPending { count: 3, unread: 0 }, Event::ModuleChanged { module: ID }] {
        event(&mut k, e);
    }
    drop(k);
    assert!(take_ui().is_empty());
}

#[test]
fn the_item_follows_the_presence_and_the_badge() {
    take_ui();
    let mut k = started(SERVER);
    let calls = take_ui();
    // `Started` shows it; the round may or may not have ended by then.
    assert_eq!(calls.first().map(String::as_str), Some("listen"));
    assert_eq!(calls.last().map(String::as_str), Some(THINKING));
    assert!(calls.len() == 2 || calls[1] == "show K? | KOTA: unknown | 8 rows", "{calls:?}");
    // Nothing changed: no redraw.
    event(&mut k, Event::ModuleChanged { module: ID });
    event(&mut k, Event::LauncherOpened);
    assert!(take_ui().is_empty());
    event(&mut k, Event::CardsPending { count: 2, unread: 0 });
    assert_eq!(take_ui(), [THINKING.replace("show K… ", "show K… 2 ")]);
    // An unread post adds to the badge.
    event(&mut k, Event::CardsPending { count: 1, unread: 1 });
    assert_eq!(take_ui(), [THINKING.replace("show K… ", "show K… 2 ")], "the Inbox row changed");
    event(&mut k, Event::Locked);
    assert_eq!(take_ui(), [THINKING.replace("show K… ", "show K? 2 ").replace("<1m\n", "<1m (stale)\n")]);
    // Disabled or dropped: hidden.
    drop(k);
    assert_eq!(take_ui(), ["hide"]);
}

#[test]
fn status_item_false_hides_it_and_true_brings_it_back() {
    take_ui();
    let mut k = started(&format!("{SERVER}status_item = false"));
    assert!(take_ui().is_empty());
    reconfigure(&mut k, SERVER);
    assert_eq!(take_ui(), ["listen", THINKING]);
    reconfigure(&mut k, &format!("{SERVER}status_item = false"));
    assert_eq!(take_ui(), ["hide"]);
    drop(k);
    assert!(take_ui().is_empty());
}

#[test]
fn emptying_the_table_hides_it() {
    take_ui();
    let mut k = started(SERVER);
    take_ui();
    k.configure(&parse("").unwrap().section(ID).unwrap().unwrap()).unwrap();
    assert_eq!(take_ui(), ["hide"]);
    event(&mut k, Event::CardsPending { count: 1, unread: 0 });
    assert!(take_ui().is_empty());
}

#[test]
fn picks_open_the_dashboard_and_refresh() {
    take_ui();
    let mut k = started(SERVER);
    take_ui();
    queue(DASH);
    queue("unknown");
    event(&mut k, Event::ModuleChanged { module: ID });
    assert_eq!(take_ui(), ["open http://ok"]);
    // Refresh and menu opens: rate-limited like `kota refresh` (the fake clock stands
    // still, so the first round is 0 s ago).
    let started_at = k.shared.lock().started_at;
    queue(REFRESH);
    queue(MENU_OPENED);
    event(&mut k, Event::ModuleChanged { module: ID });
    assert_eq!(k.shared.lock().started_at, started_at);
    k.shared.lock().started_at = Some(0);
    queue(MENU_OPENED);
    event(&mut k, Event::ModuleChanged { module: ID });
    assert_eq!(k.shared.lock().started_at, Some(1_000));
}

fn down(k: &mut Kota) {
    k.shared.lock().transitions.push(Transition { from: State::Idle, to: State::Down });
    event(k, Event::ModuleChanged { module: ID });
}

#[test]
fn a_change_to_down_notifies_once() {
    take_ui();
    let mut k = started(SERVER);
    take_ui();
    down(&mut k);
    assert_eq!(take_ui(), ["notify KOTA is down: herdr on server has no KOTA pane"]);
    // Other changes and drained ones do not.
    k.shared.lock().transitions.push(Transition { from: State::Down, to: State::Idle });
    event(&mut k, Event::ModuleChanged { module: ID });
    assert!(take_ui().is_empty());
    reconfigure(&mut k, &format!("{SERVER}notify_down = false"));
    take_ui();
    down(&mut k);
    assert!(take_ui().is_empty());
    assert!(k.shared.lock().transitions.is_empty());
}

#[test]
fn without_a_kota_key_down_does_not_notify() {
    let mut k = configured("").unwrap();
    event(&mut k, Event::Started);
    down(&mut k);
    assert!(take_ui().is_empty());
    assert!(k.shared.lock().transitions.is_empty());
}

#[test]
fn the_menu_rows_route_through_hotkeys() {
    let mut k = configured("").unwrap();
    test_cx("", |cx| {
        let inbox = k.hotkey(INBOX, cx).unwrap();
        assert!(inbox.is(INBOX_VIEW.0, INBOX_VIEW.1));
        assert!(k.hotkey("ask", cx).unwrap().is(ID, "ask"));
        assert!(k.hotkey("nope", cx).is_none());
    });
}
