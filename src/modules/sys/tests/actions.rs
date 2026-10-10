//! Fleet actions (flick-4a4c): menus, Screen Sharing and Open Dash, Tail Log, the restart
//! confirm, and the `sys tail` and `sys restart` verbs. Every child is a testkit fake: no
//! test opens a URL, runs tail or launchctl, or ssh.

use super::*;
use crate::core::later;
use testkit::OPENED;

const ACTIONS: &str = r#"
[[sys.service]]
name = "up"
kind = "launchd"
target = "up"
log = "/logs/up.log"
restart = true
[[sys.service]]
name = "gone"
kind = "launchd"
target = "gone"
log = "/logs/missing.log"
restart = true
[[sys.service]]
name = "web"
kind = "http"
target = "http://ok/"
[[sys.machine]]
name = "laptop"
via = "local"
vnc = "vnc://laptop"
[[sys.machine]]
name = "server"
via = "flick"
dash = "https://server:8310/"
[[sys.machine]]
name = "pro"
via = "ssh"
ssh = "pro"
vnc = "vnc://pro"
[[sys.machine.service]]
name = "up"
kind = "launchd"
target = "up"
log = "~/logs/up.log"
restart = true
[[sys.machine.service]]
name = "down"
kind = "launchd"
target = "down"
restart = true
"#;

fn id(key: &str) -> ItemId {
    ItemId::new(ID, key)
}

fn menu(m: &mut Sys, key: &str) -> Vec<&'static str> {
    test_cx("", |cx| m.actions(&id(key), cx)).iter().map(|a| a.key).collect()
}

fn act(m: &mut Sys, key: &str, action: &str) -> Outcome {
    test_cx("", |cx| m.act(&id(key), action, cx))
}

fn stay(o: Outcome) -> String {
    match o {
        Outcome::Stay(s) => s.unwrap_or_default(),
        other => format!("not a Stay: {other:?}"),
    }
}

fn tailed(m: &Sys) -> Option<Result<String, String>> {
    assert!(m.shared.wait(Duration::from_secs(5), |s| s.tail.as_ref().is_some_and(|t| t.text.is_some())));
    m.shared.lock().tail.as_ref().and_then(|t| t.text.clone())
}

fn acted(m: &Sys) -> String {
    assert!(m.shared.wait(Duration::from_secs(5), |s| s.acted.is_some()));
    m.shared.lock().acted.take().map(|(text, _)| text).unwrap_or_default()
}

#[test]
fn menus_follow_the_config() {
    let mut m = sys(ACTIONS, HOOKS);
    assert_eq!(menu(&mut m, "machine/laptop"), ["vnc"]);
    assert_eq!(menu(&mut m, "machine/server"), ["dash"]);
    assert_eq!(menu(&mut m, "machine/pro"), ["vnc"]);
    assert_eq!(menu(&mut m, "service/laptop/up"), ["tail", "restart"]);
    assert_eq!(menu(&mut m, "service/laptop/web"), Vec::<&str>::new());
    assert_eq!(menu(&mut m, "service/pro/up"), ["tail", "restart"]);
    assert_eq!(menu(&mut m, "service/pro/down"), ["restart"]);
    // A service only the peer's snapshot names: nothing to run from here.
    assert!(menu(&mut m, "service/server/kota-dash").is_empty());
    for key in ["agents", "fact/0", "log", "head", "check/up", "machine/nope", "x"] {
        assert!(menu(&mut m, key).is_empty(), "{key}");
    }
    // In the machine view the head and checks are the picked machine's.
    m.detail = Some("pro".into());
    assert_eq!(menu(&mut m, "head"), ["vnc"]);
    assert_eq!(menu(&mut m, "check/up"), ["tail", "restart"]);
    m.detail = Some("laptop".into());
    assert_eq!(menu(&mut m, "check/gone"), ["tail", "restart"]);
}

#[test]
fn screen_sharing_and_dash_open_their_urls() {
    let mut m = sys(ACTIONS, HOOKS);
    OPENED.with(|o| o.borrow_mut().clear());
    assert!(matches!(act(&mut m, "machine/pro", "vnc"), Outcome::Hide));
    assert!(matches!(act(&mut m, "machine/server", "dash"), Outcome::Hide));
    let gone = "sys: that row has no such action now";
    assert_eq!(stay(act(&mut m, "machine/server", "vnc")), gone);
    assert_eq!(stay(act(&mut m, "service/pro/up", "dash")), gone);
    assert_eq!(stay(act(&mut m, "machine/pro", "fly")), "");
    assert_eq!(OPENED.with(|o| o.borrow().clone()), ["vnc://pro", "https://server:8310/"]);
}

#[test]
fn tail_log_shows_the_lines_in_view_log() {
    let mut m = sys(ACTIONS, HOOKS);
    m.fleet_view = true;
    let Outcome::Push(v) = act(&mut m, "service/pro/up", "tail") else { panic!("no push") };
    assert_eq!((v.module, v.name.as_str()), (ID, "log"));
    assert!(!m.fleet_view, "the log view does not poll the fleet");
    let want = "tail via pro: exec /usr/bin/tail -n 100 -- \"$HOME\"/'logs/up.log'\n";
    assert_eq!(tailed(&m), Some(Ok(want.into())));
    let mut view = test_cx("", |cx| m.open("log", cx)).unwrap();
    test_cx("", |cx| m.refresh(&mut view, cx));
    assert_eq!(view.text, want);
    assert_eq!(view.items[0].title, "up · pro");
    assert_eq!(view.footer, "up log · pro  ·  esc to go back");
    // Enter tails again; a missing log says why.
    m.detail = Some("laptop".into());
    assert!(matches!(act(&mut m, "check/gone", "tail"), Outcome::Push(_)));
    assert_eq!(tailed(&m), Some(Err("tail: /logs/missing.log: No such file or directory".into())));
    m.shared.lock().tail.as_mut().unwrap().text = None;
    assert_eq!(stay(test_cx("", |cx| m.activate(&id("log"), cx))), "");
    assert_eq!(tailed(&m).unwrap().unwrap_err(), "tail: /logs/missing.log: No such file or directory");
    assert_eq!(stay(act(&mut m, "machine/pro", "tail")), "sys: that row has no such action now");
}

#[test]
fn tail_again_without_a_tail_or_config_does_nothing_or_says_why() {
    let mut m = sys(ACTIONS, HOOKS);
    assert_eq!(stay(test_cx("", |cx| m.activate(&id("log"), cx))), "");
    let mut view = test_cx("", |cx| m.open("log", cx)).unwrap();
    test_cx("", |cx| m.refresh(&mut view, cx));
    assert!(view.items.is_empty());
    // The config lost the service since.
    act(&mut m, "service/laptop/up", "tail");
    tailed(&m);
    m.configure(&parse("").unwrap().section(ID).unwrap().unwrap()).unwrap();
    assert_eq!(stay(test_cx("", |cx| m.activate(&id("log"), cx))), "sys: no machine \"laptop\"");
    // A service without a log can't be tailed from a stale menu either.
    let mut m = sys(ACTIONS, HOOKS);
    m.detail = Some("pro".into());
    assert_eq!(stay(act(&mut m, "check/down", "tail")), "sys: down on pro has no log (set log = \"<path>\")");
}

#[test]
fn restart_asks_first_then_runs_the_shown_command() {
    let mut m = sys(ACTIONS, HOOKS);
    let Outcome::Confirm(c) = act(&mut m, "service/laptop/up", "restart") else { panic!("no confirm") };
    assert!(c.destructive);
    assert_eq!(c.rows[0].title, "launchctl kickstart -k gui/501/up");
    assert!(m.shared.lock().acted.is_none(), "nothing ran before the confirm");
    assert_eq!(stay(test_cx("", |cx| m.confirmed(&c.token, cx))), "Restarting up on laptop…");
    assert_eq!(acted(&m), "Restarted up on laptop");
    // Over ssh: the fake Mac knows `up` but not `down`.
    let Outcome::Confirm(c) = act(&mut m, "service/pro/down", "restart") else { panic!("no confirm") };
    assert_eq!(c.rows[0].title, "ssh pro launchctl kickstart -k gui/$(id -u)/down");
    stay(test_cx("", |cx| m.confirmed(&c.token, cx)));
    assert_eq!(acted(&m), "Restart down on pro failed: Could not find service in domain for user gui: 502");
    let Outcome::Confirm(c) = act(&mut m, "service/pro/up", "restart") else { panic!("no confirm") };
    stay(test_cx("", |cx| m.confirmed(&c.token, cx)));
    assert_eq!(acted(&m), "Restarted up on pro");
    // The footer of the fleet views shows the result for a minute.
    m.shared.lock().acted = Some(("Restarted up on pro".into(), 990));
    let mut view = test_cx("", |cx| m.open("fleet", cx)).unwrap();
    test_cx("", |cx| m.refresh(&mut view, cx));
    assert!(view.footer.starts_with("Restarted up on pro  ·  3 machines"), "{}", view.footer);
    m.shared.lock().acted = Some(("old".into(), 900));
    test_cx("", |cx| m.refresh(&mut view, cx));
    assert!(view.footer.starts_with("3 machines"), "{}", view.footer);
    settle(&m);
}

#[test]
fn a_restart_is_checked_again_on_confirm() {
    let mut m = sys(ACTIONS, HOOKS);
    let Outcome::Confirm(c) = act(&mut m, "service/laptop/up", "restart") else { panic!("no confirm") };
    // The config changed while the confirm showed.
    let off = ACTIONS.replacen("restart = true", "restart = false", 1);
    m.configure(&parse(&off).unwrap().section(ID).unwrap().unwrap()).unwrap();
    let refused = "sys: up on laptop may not be restarted (set restart = true on a launchd service)";
    assert_eq!(stay(test_cx("", |cx| m.confirmed(&c.token, cx))), refused);
    assert_eq!(stay(act(&mut m, "service/laptop/up", "restart")), refused);
    assert_eq!(stay(test_cx("", |cx| m.confirmed("other", cx))), "");
    assert_eq!(stay(act(&mut m, "machine/laptop", "restart")), "sys: that row has no such action now");
    // No uid, no local restart.
    let mut m = sys(ACTIONS, Hooks { uid: || None, ..HOOKS });
    assert_eq!(stay(act(&mut m, "service/laptop/up", "restart")), "sys: no uid for the launchd domain");
    assert!(m.shared.lock().acted.is_none());
}

/// A verb's later answer, as the control socket settles it.
fn later_answer(now: Result<String, String>) -> Result<String, String> {
    later::settle(now, later::take(), Duration::from_secs(5))
}

#[test]
fn tail_verb_answers_with_the_lines() {
    let mut m = sys(ACTIONS, HOOKS);
    let now = ask(&mut m, &["tail", "up"], false);
    assert_eq!(now.as_deref(), Ok("Tailing up on this Mac"));
    assert_eq!(later_answer(now), Ok("last of /logs/up.log\n".into()));
    // Enter on the log row tails this Mac's service again.
    assert_eq!(stay(test_cx("", |cx| m.activate(&id("log"), cx))), "");
    assert_eq!(tailed(&m), Some(Ok("last of /logs/up.log\n".into())));
    let now = ask(&mut m, &["tail", "pro", "up"], false);
    assert!(later_answer(now).unwrap().starts_with("tail via pro: "));
    assert_eq!(ask(&mut m, &["tail", "web"], false).unwrap_err(), "sys: web on this Mac has no log (set log = \"<path>\")");
    assert!(later::take().is_none(), "a refused verb leaves no later answer");
    assert_eq!(ask(&mut m, &["tail"], false).unwrap_err(), "sys: usage: sys tail [<machine>] <service>");
}

#[test]
fn restart_verb_runs_only_with_yes() {
    let mut m = sys(ACTIONS, HOOKS);
    let dry = ask(&mut m, &["restart", "up"], false).unwrap();
    assert_eq!(dry, "would run: launchctl kickstart -k gui/501/up\nadd --yes to restart up on this Mac");
    assert!(m.shared.lock().acted.is_none(), "a dry run runs nothing");
    let now = ask(&mut m, &["restart", "pro", "up", "--yes"], false);
    assert_eq!(now.as_deref(), Ok("Restarting up on pro"));
    assert_eq!(later_answer(now), Ok("Restarted up on pro".into()));
    let now = ask(&mut m, &["restart", "gone", "--yes"], false);
    assert_eq!(later_answer(now).unwrap_err(), "Restart gone on this Mac failed: Could not find service \"no.such.label\" in domain for user gui: 501");
    assert_eq!(ask(&mut m, &["restart", "a", "b", "c"], false).unwrap_err(), "sys: usage: sys restart [<machine>] <service>");
    assert_eq!(ask(&mut m, &["restart", "nope", "up"], false).unwrap_err(), "sys: no machine \"nope\"");
}
