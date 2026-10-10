//! The fleet window's cmd+K action cards and its hotkey (flick-1e00), on `testkit::WINDOW`
//! and the testkit fakes: no window opens, nothing restarts, no ssh runs.

use super::*;
use crate::modules::sys::window::Note;
use crate::platform::hud::CANCEL;
use testkit::{OPENED, SHOWN};

const ACTIONS: &str = r#"
[[sys.service]]
name = "agent"
kind = "launchd"
target = "up"
log = "/var/log/agent.log"
restart = true
[[sys.machine]]
name = "laptop"
via = "local"
vnc = "vnc://laptop"
[[sys.machine]]
name = "pro"
via = "ssh"
ssh = "pro"
dash = "http://pro/"
[[sys.machine.service]]
name = "up"
kind = "launchd"
target = "up"
log = "~/up.log"
restart = true
[[sys.machine]]
name = "bare"
via = "flick"
"#;

fn rows() -> Vec<String> {
    SHOWN.with(|s| s.borrow().rows.clone())
}

fn keys() -> Vec<String> {
    rows().iter().map(|r| r.split('|').next().unwrap_or_default().to_string()).collect()
}

fn notice() -> Option<String> {
    SHOWN.with(|s| s.borrow().notice.clone())
}

fn visible() -> bool {
    SHOWN.with(|s| s.borrow().visible)
}

/// Queue `note` as a handler would, then deliver the `ModuleChanged` it posts.
fn send(m: &mut Sys, note: Note) {
    SHOWN.with(|s| s.borrow_mut().notes.push(note));
    event(m, Event::ModuleChanged { module: ID });
}

fn press(m: &mut Sys, card: &str, action: &str) {
    send(m, Note::Press { card: card.into(), action: action.into() });
}

/// The window shown on `config`, every machine read once.
fn shown(config: &str) -> Sys {
    SHOWN.with(|s| *s.borrow_mut() = testkit::Shown::default());
    let mut m = sys(config, HOOKS);
    event(&mut m, Event::Started);
    ask(&mut m, &["window"], false).unwrap();
    settle(&m);
    event(&mut m, Event::ModuleChanged { module: ID });
    m
}

#[test]
fn cmd_k_shows_a_card_under_each_machine_with_actions() {
    let mut m = shown(ACTIONS);
    assert_eq!(keys(), ["machine/laptop", "machine/pro", "machine/bare"]);
    send(&mut m, Note::Actions);
    assert_eq!(keys(), ["machine/laptop", "actions/laptop", "machine/pro", "actions/pro", "machine/bare"]);
    let got = rows();
    assert_eq!(got[1], "actions/laptop|card|laptop · actions|Screen Sharing, Tail agent, Restart agent…|confirm -");
    assert_eq!(got[3], "actions/pro|card|pro · actions|Open Dash, Tail up, Restart up…|confirm -");
    // The filter keeps a machine's card with it.
    send(&mut m, Note::Filter("pro".into()));
    assert_eq!(keys(), ["machine/pro", "actions/pro"]);
    // cmd+K again hides them.
    send(&mut m, Note::Actions);
    assert_eq!(keys(), ["machine/pro"]);
}

#[test]
fn a_fleet_without_actions_says_how_to_add_them() {
    let mut m = shown(FLEET);
    send(&mut m, Note::Actions);
    assert_eq!(keys(), ["machine/laptop", "machine/server", "machine/pro"]);
    assert_eq!(notice().as_deref(), Some("No actions: set vnc or dash on a machine, or log or restart on a service"));
}

#[test]
fn open_and_tail_from_a_card() {
    let mut m = shown(ACTIONS);
    send(&mut m, Note::Actions);
    OPENED.with(|o| o.borrow_mut().clear());
    press(&mut m, "actions/laptop", "vnc");
    press(&mut m, "actions/pro", "dash");
    assert_eq!(OPENED.with(|o| o.borrow().clone()), ["vnc://laptop", "http://pro/"]);
    // A machine without that URL: the notice says so.
    press(&mut m, "actions/laptop", "dash");
    assert_eq!(notice().as_deref(), Some("sys: laptop has no such action now"));
    // A tail shows under its machine's card, over ssh with the remote $HOME.
    press(&mut m, "actions/pro", "tail/up");
    assert_eq!(notice(), None, "a press that went out clears the last complaint");
    assert!(m.shared.wait(Duration::from_secs(5), |s| s.tail.as_ref().is_some_and(|t| t.text.is_some())));
    event(&mut m, Event::ModuleChanged { module: ID });
    assert_eq!(keys(), ["machine/laptop", "actions/laptop", "machine/pro", "actions/pro", "tail", "machine/bare"]);
    let want = "tail|Theirs|Tail · up on pro|0s ago|Done|```\ntail via pro: exec /usr/bin/tail -n 100 -- \"$HOME\"/'up.log'\n```";
    assert_eq!(rows()[4], want);
    // A service this Mac's config does not define: nothing runs, the notice says why.
    press(&mut m, "actions/pro", "tail/nope");
    assert_eq!(notice().as_deref(), Some("sys: no service \"nope\" on pro in this Mac's config"));
    assert!(!keys().contains(&"tail".to_string()));
    // Unknown cards and actions do nothing.
    press(&mut m, "deploy-42", "vnc");
    press(&mut m, "actions/pro", "reboot");
    assert_eq!(notice().as_deref(), Some("sys: no service \"nope\" on pro in this Mac's config"));
}

#[test]
fn restart_asks_on_the_card_then_runs_on_run() {
    let mut m = shown(ACTIONS);
    send(&mut m, Note::Actions);
    press(&mut m, "actions/laptop", "restart/agent");
    assert!(rows()[1].ends_with("|confirm restart/agent"), "{:?}", rows());
    assert!(m.shared.lock().acted.is_none(), "the first press only asks");
    // Cancel: back to the buttons, nothing ran.
    press(&mut m, "actions/laptop", CANCEL);
    assert!(rows()[1].ends_with("|confirm -"));
    assert!(m.shared.lock().acted.is_none());
    // Asking on one card and pressing on another asks again.
    press(&mut m, "actions/laptop", "restart/agent");
    press(&mut m, "actions/pro", "restart/up");
    assert!(rows()[1].ends_with("|confirm -") && rows()[3].ends_with("|confirm restart/up"), "{:?}", rows());
    // Run (the same action again) restarts, re-resolved from config.
    press(&mut m, "actions/pro", "restart/up");
    assert!(rows()[3].ends_with("|confirm -"));
    assert!(m.shared.wait(Duration::from_secs(5), |s| s.acted.as_ref().is_some_and(|(t, _)| !t.ends_with('…'))));
    event(&mut m, Event::ModuleChanged { module: ID });
    assert_eq!(notice().as_deref(), Some("Restarted up on pro"));
    // Locally too.
    press(&mut m, "actions/laptop", "restart/agent");
    press(&mut m, "actions/laptop", "restart/agent");
    assert!(m.shared.wait(Duration::from_secs(5), |s| s.acted.as_ref().is_some_and(|(t, _)| t == "Restarted agent on laptop")));
    // A restart whose service is gone from config by the time Run is pressed runs nothing.
    press(&mut m, "actions/pro", "restart/gone");
    press(&mut m, "actions/pro", "restart/gone");
    assert_eq!(notice().as_deref(), Some("sys: no service \"gone\" on pro in this Mac's config"));
}

#[test]
fn hiding_and_showing_again_drops_the_cards() {
    let mut m = shown(ACTIONS);
    send(&mut m, Note::Actions);
    press(&mut m, "actions/laptop", "restart/agent");
    send(&mut m, Note::Close);
    ask(&mut m, &["window"], false).unwrap();
    assert_eq!(keys(), ["machine/laptop", "machine/pro", "machine/bare"]);
    // Shown again while it shows: the cards stay.
    send(&mut m, Note::Actions);
    ask(&mut m, &["window"], false).unwrap();
    assert!(keys().contains(&"actions/pro".to_string()));
}

#[test]
fn the_hotkey_shows_the_window_or_hides_it_when_it_has_the_keyboard() {
    let mut m = shown(&format!("[sys]\nhotkey = \"cmd+KeyY\"\n{FLEET}"));
    let bound: Vec<(String, Result<String, String>)> = m.hotkeys().into_iter().map(|b| (b.spec, b.key)).collect();
    assert_eq!(bound, [("cmd+KeyY".to_string(), Ok("window".to_string()))]);
    // Shown with the keyboard: it hides.
    assert!(test_cx("", |cx| m.hotkey("window", cx)).is_none());
    assert!(!visible());
    // Hidden: it shows.
    assert!(test_cx("", |cx| m.hotkey("window", cx)).is_none());
    assert!(visible());
    // Shown without the keyboard: it takes it, it does not hide.
    SHOWN.with(|s| s.borrow_mut().key = false);
    test_cx("", |cx| m.hotkey("window", cx));
    assert!(visible() && SHOWN.with(|s| s.borrow().key));
    // Another key does nothing; unset or blank binds nothing.
    test_cx("", |cx| m.hotkey("other", cx));
    assert!(visible());
    assert!(sys(FLEET, HOOKS).hotkeys().is_empty());
    assert!(sys("[sys]\nhotkey = \" \"", HOOKS).hotkeys().is_empty());
}

#[test]
fn a_tail_bubble_shows_its_lines_or_why_not() {
    use crate::modules::sys::jobs::Tail;
    use crate::modules::sys::window::tail_bubble;
    use crate::platform::surface::rows::BubbleState;
    let t = |text: Option<Result<&str, &str>>, at| Tail {
        id: 1,
        machine: "pro".into(),
        service: "up".into(),
        shown: "ssh pro tail -n 100 ~/up.log".into(),
        text: text.map(|r| r.map(str::to_string).map_err(str::to_string)),
        at,
    };
    let b = tail_bubble(&t(None, None), 1_000);
    assert_eq!((b.md.as_str(), b.time.as_str(), b.state), ("Running `ssh pro tail -n 100 ~/up.log`…", "", BubbleState::Pending));
    let b = tail_bubble(&t(Some(Ok(" \n")), Some(990)), 1_000);
    assert_eq!((b.md.as_str(), b.time.as_str(), b.state), ("The log is empty.", "10s ago", BubbleState::Done));
    let b = tail_bubble(&t(Some(Err("tail: no such file")), Some(1_000)), 1_000);
    assert_eq!((b.header.as_str(), b.md.as_str(), b.state), ("Tail · up on pro", "tail: no such file", BubbleState::Failed));
}
