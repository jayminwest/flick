//! The fleet window (flick-a2ed), on `testkit::WINDOW`: no test opens a window, and every
//! machine is a testkit fake (no peer, no ssh).

use super::*;
use crate::platform::surface::{Key, Keystroke, Status};
use testkit::SHOWN;
use crate::modules::sys::window::{self as win, Note};

fn fresh() {
    SHOWN.with(|s| *s.borrow_mut() = testkit::Shown::default());
}

fn rows() -> Vec<String> {
    SHOWN.with(|s| s.borrow().rows.clone())
}

fn keys() -> Vec<String> {
    rows().iter().map(|r| r.split('|').next().unwrap_or_default().to_string()).collect()
}

fn header() -> (String, Status) {
    SHOWN.with(|s| s.borrow().header.clone().map(|(t, sub, st)| (format!("{t}: {sub}"), st))).unwrap()
}

fn visible() -> bool {
    SHOWN.with(|s| s.borrow().visible)
}

fn notice() -> Option<String> {
    SHOWN.with(|s| s.borrow().notice.clone())
}

/// Queue `note` as a handler would, then deliver the `ModuleChanged` it posts.
fn send(m: &mut Sys, note: Note) {
    SHOWN.with(|s| s.borrow_mut().notes.push(note));
    event(m, Event::ModuleChanged { module: ID });
    settle(m);
    event(m, Event::ModuleChanged { module: ID });
}

/// Every machine read 14 s ago: one short of `VISIBLE_EVERY`, so only a forced poll reads.
fn age_all(m: &Sys, at: u64) {
    for slot in &mut m.shared.lock().fleet.slots {
        slot.tried_at = Some(at);
    }
}

fn started() -> Sys {
    fresh();
    let mut m = sys(FLEET, HOOKS);
    event(&mut m, Event::Started);
    m
}

fn stroke(key: Key, cmd: bool, shift: bool) -> Keystroke {
    Keystroke { key, cmd, shift, opt: false }
}

#[test]
fn keys_map_to_notes() {
    let input = || "pro".to_string();
    assert_eq!(win::note(stroke(Key::Return, false, false), input), Some(Note::Filter("pro".into())));
    assert_eq!(win::note(stroke(Key::Char('r'), true, false), || unreachable!()), Some(Note::Refresh));
    assert_eq!(win::note(stroke(Key::Char('w'), true, false), || unreachable!()), Some(Note::Close));
    for k in [
        stroke(Key::Char('r'), true, true),
        stroke(Key::Char('r'), false, false),
        stroke(Key::Char('n'), true, false),
        stroke(Key::Escape, false, false),
        stroke(Key::Up, false, false),
    ] {
        assert_eq!(win::note(k, || unreachable!()), None, "{k:?}");
    }
}

#[test]
fn sys_window_shows_one_bubble_per_machine_and_polls() {
    let mut m = started();
    assert_eq!(ask(&mut m, &["window"], false).unwrap(), "Fleet window shown (polls every 15 s while it shows)");
    assert!(visible());
    // Drawn before the first round lands: every machine pending.
    assert_eq!(keys(), ["machine/laptop", "machine/server", "machine/pro"]);
    assert!(rows()[1].starts_with("machine/server|Theirs|server · pending · via flick||Pending|loading…"), "{:?}", rows());
    assert_eq!(header(), ("Fleet: 3 machines  ·  ⌘R refresh  ·  Esc hides".into(), Status::Busy));
    settle(&m);
    assert_eq!(tried(&m), [None, Some(1_000), Some(1_000)], "showing it polled");
    event(&mut m, Event::ModuleChanged { module: ID });
    let got = rows();
    assert!(got[0].starts_with("machine/laptop|Theirs|laptop · via local|0s ago|Done|load 1.42"), "{got:?}");
    assert!(got[1].starts_with("machine/server|Theirs|server · via flick|0s ago|Done|load 0.50"), "{got:?}");
    assert!(got[2].ends_with("\n- ok ollama · running (pid 812)"), "{got:?}");
    assert_eq!(header().1, Status::Idle);
    // A second `sys window` builds nothing new.
    ask(&mut m, &["window"], false).unwrap();
    assert_eq!(SHOWN.with(|s| s.borrow().opened), 1);
}

#[test]
fn it_polls_while_it_shows_and_stops_once_hidden() {
    let mut m = started();
    ask(&mut m, &["window"], false).unwrap();
    settle(&m);
    // Shown, launcher closed: due machines are read on the next tick.
    age_all(&m, 985);
    event(&mut m, Event::ModuleChanged { module: ID });
    settle(&m);
    assert_eq!(tried(&m), [Some(985), Some(1_000), Some(1_000)]);
    // cmd+W hides it: ticks read nothing more.
    age_all(&m, 985);
    send(&mut m, Note::Close);
    assert!(!visible());
    assert_eq!(tried(&m), [Some(985), Some(985), Some(985)]);
    // cmd+R reads every machine at once, even recently read ones.
    ask(&mut m, &["window"], false).unwrap();
    settle(&m);
    age_all(&m, 999);
    send(&mut m, Note::Refresh);
    assert_eq!(tried(&m), [Some(999), Some(1_000), Some(1_000)]);
}

#[test]
fn the_filter_keeps_matching_machines_or_services() {
    let mut m = started();
    ask(&mut m, &["window"], false).unwrap();
    settle(&m);
    send(&mut m, Note::Filter("  PRO ".into()));
    assert_eq!(keys(), ["machine/pro"]);
    assert_eq!(notice().as_deref(), Some("Showing “PRO”  ·  Return on an empty field shows all"));
    // A service name keeps its machine with only that service.
    send(&mut m, Note::Filter("ollama".into()));
    assert_eq!(keys(), ["machine/pro"]);
    assert!(rows()[0].ends_with("- ok ollama · running (pid 812)"));
    send(&mut m, Note::Filter("flick".into()));
    assert_eq!(keys(), ["machine/server"]);
    send(&mut m, Note::Filter("zzz".into()));
    assert_eq!(rows(), ["none|System|||Done|Nothing matches “zzz”"]);
    send(&mut m, Note::Filter(String::new()));
    assert_eq!(keys(), ["machine/laptop", "machine/server", "machine/pro"]);
    assert_eq!(notice(), None);
}

#[test]
fn trouble_turns_the_dot_red_and_marks_the_machine() {
    fresh();
    let config = FLEET.replace("ssh = \"pro\"", "ssh = \"refused\"").replace("target = \"ollama\"", "target = \"nope\"");
    let mut m = sys(&config, HOOKS);
    event(&mut m, Event::Started);
    ask(&mut m, &["window"], false).unwrap();
    settle(&m);
    event(&mut m, Event::ModuleChanged { module: ID });
    let pro = &rows()[2];
    assert!(pro.starts_with("machine/pro|Theirs|pro · down · via ssh||Failed|ssh: "), "{pro}");
    assert_eq!(header(), ("Fleet: 3 machines · 1 down  ·  ⌘R refresh  ·  Esc hides".into(), Status::Error));
    // A failing service alone turns it red too, and is bold in its bullet.
    fresh();
    let config = FLEET.replace("target = \"ollama\"", "target = \"nope\"");
    let mut m = sys(&config, HOOKS);
    event(&mut m, Event::Started);
    ask(&mut m, &["window"], false).unwrap();
    settle(&m);
    event(&mut m, Event::ModuleChanged { module: ID });
    assert!(rows()[2].contains("\n- **fail** ollama · "), "{:?}", rows());
    assert_eq!(header().1, Status::Error);
}

#[test]
fn no_machines_says_how_to_add_them() {
    fresh();
    let mut m = sys("", HOOKS);
    ask(&mut m, &["window"], false).unwrap();
    assert_eq!(rows(), ["empty|System|||Done|No machines (add `[[sys.machine]]` tables to config.toml)"]);
    assert_eq!(header(), ("Fleet: 0 machines  ·  ⌘R refresh  ·  Esc hides".into(), Status::Idle));
    // Not started: showing it polls nothing.
    assert!(!m.shared.lock().fleet.busy());
}

#[test]
fn snapshot_draws_without_showing_and_bad_words_get_usage() {
    let mut m = started();
    assert_eq!(ask(&mut m, &["window", "--snapshot", "/x/fleet.png"], false).unwrap(), "Wrote /x/fleet.png");
    SHOWN.with(|s| {
        let s = s.borrow();
        assert_eq!((s.opened, s.visible, s.snapshots.clone()), (1, false, vec!["/x/fleet.png".to_string()]));
    });
    assert_eq!(keys(), ["machine/laptop", "machine/server", "machine/pro"]);
    assert_eq!(ask(&mut m, &["window", "--snapshot", "/nope/fleet.png"], false).unwrap_err(), "can't write /nope/fleet.png");
    let usage = "sys: usage: sys window [--snapshot <png>]";
    assert_eq!(ask(&mut m, &["window", "now"], false).unwrap_err(), usage);
    assert_eq!(ask(&mut m, &["window", "--snapshot"], false).unwrap_err(), usage);
    assert_eq!(tried(&m), [None, None, None], "a snapshot polls nothing");
}

#[test]
fn the_root_item_offers_open_fleet_window() {
    let mut m = started();
    let menu = test_cx("", |cx| m.actions(&ItemId::new(ID, "fleet"), cx));
    assert_eq!(menu.iter().map(|a| (a.key, a.title.as_str())).collect::<Vec<_>>(), [("window", "Open Fleet Window")]);
    assert!(matches!(test_cx("", |cx| m.act(&ItemId::new(ID, "fleet"), "window", cx)), Outcome::Hide));
    assert!(visible());
    settle(&m);
    assert_eq!(tried(&m), [None, Some(1_000), Some(1_000)]);
}

#[test]
fn cmd_r_says_refreshing_then_refreshed_ahead_of_the_filter() {
    fresh();
    // `slow` answers after 300 ms, so the round still runs when the window redraws.
    let mut m = sys(&FLEET.replace("ssh = \"pro\"", "ssh = \"slow\""), HOOKS);
    event(&mut m, Event::Started);
    ask(&mut m, &["window"], false).unwrap();
    settle(&m);
    event(&mut m, Event::ModuleChanged { module: ID });
    assert_eq!(notice(), None, "showing it is not a cmd+R");
    send(&mut m, Note::Filter("pro".into()));
    let showing = "Showing “pro”  ·  Return on an empty field shows all";
    SHOWN.with(|s| s.borrow_mut().notes.push(Note::Refresh));
    event(&mut m, Event::ModuleChanged { module: ID });
    assert_eq!(notice(), Some(format!("Refreshing…  ·  {showing}")));
    settle(&m);
    event(&mut m, Event::ModuleChanged { module: ID });
    assert_eq!(notice(), Some(format!("Refreshed  ·  {showing}")));
    // Still there within `REFRESHED_FOR` s, gone at the first redraw after.
    m.hooks.now = || 1_002;
    event(&mut m, Event::ModuleChanged { module: ID });
    assert_eq!(notice(), Some(format!("Refreshed  ·  {showing}")));
    m.hooks.now = || 1_003;
    event(&mut m, Event::ModuleChanged { module: ID });
    assert_eq!(notice().as_deref(), Some(showing));
    // Without a filter it stands alone; cmd+W drops it.
    send(&mut m, Note::Filter(String::new()));
    send(&mut m, Note::Refresh);
    assert_eq!(notice().as_deref(), Some("Refreshed"));
    send(&mut m, Note::Close);
    ask(&mut m, &["window"], false).unwrap();
    assert_eq!(notice(), None);
}
