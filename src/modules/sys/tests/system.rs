//! System-domain launchd daemons through `sudo -n` and the restart's HUD toast
//! (flick-1356). Every child is a testkit fake: no test runs sudo, launchctl, tail or ssh,
//! and no toast reaches the HUD.

use super::*;
use crate::core::later;
use crate::modules::sys::act::SUDO_NEEDS_PASSWORD;
use testkit::TOASTS;

const SYSTEM: &str = r#"
[[sys.service]]
name = "up"
kind = "launchd"
target = "up"
domain = "system"
log = "/var/log/root.log"
restart = true
[[sys.service]]
name = "locked"
kind = "launchd"
target = "locked"
domain = "system"
log = "/var/log/locked.log"
restart = true
[[sys.machine]]
name = "laptop"
via = "local"
[[sys.machine]]
name = "pro"
via = "ssh"
ssh = "pro"
[[sys.machine.service]]
name = "up"
kind = "launchd"
target = "up"
domain = "system"
log = "/var/log/root.log"
restart = true
[[sys.machine.service]]
name = "down"
kind = "launchd"
target = "down"
domain = "system"
restart = true
"#;

fn act(m: &mut Sys, key: &str, action: &str) -> Outcome {
    test_cx("", |cx| m.act(&ItemId::new(ID, key), action, cx))
}

fn confirm(m: &mut Sys, key: &str) -> crate::core::Confirm {
    match act(m, key, "restart") {
        Outcome::Confirm(c) => c,
        other => panic!("no confirm: {other:?}"),
    }
}

/// Confirm the restart of `key`, wait for its result and deliver the `ModuleChanged`.
fn restarted(m: &mut Sys, key: &str) -> String {
    let c = confirm(m, key);
    test_cx("", |cx| m.confirmed(&c.token, cx));
    assert!(m.shared.wait(Duration::from_secs(5), |s| s.acted.as_ref().is_some_and(|(t, _)| !t.ends_with('…'))));
    event(m, Event::ModuleChanged { module: ID });
    m.shared.lock().acted.take().map(|(t, _)| t).unwrap_or_default()
}

fn toasts() -> Vec<String> {
    TOASTS.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

fn later_answer(now: Result<String, String>) -> Result<String, String> {
    later::settle(now, later::take(), Duration::from_secs(5))
}

#[test]
fn a_system_daemon_restarts_through_sudo_n_after_a_confirm_that_says_so() {
    let mut m = sys(SYSTEM, HOOKS);
    event(&mut m, Event::Started);
    toasts();
    let c = confirm(&mut m, "service/laptop/up");
    assert_eq!(c.rows[0].title, "sudo -n launchctl kickstart -k system/up");
    assert_eq!(c.rows[0].accessory, "launchd system · sudo -n");
    assert_eq!(restarted(&mut m, "service/laptop/up"), "Restarted up on laptop");
    assert_eq!(toasts(), ["Restarted up on laptop"]);
    // sudo would ask for a password: the result says so, and Flick typed nothing.
    let failed = format!("Restart locked on laptop failed: {SUDO_NEEDS_PASSWORD}");
    assert_eq!(restarted(&mut m, "service/laptop/locked"), failed);
    assert_eq!(toasts(), [failed]);
    // Over ssh the same, with the remote sudo.
    let c = confirm(&mut m, "service/pro/up");
    assert_eq!(c.rows[0].title, "ssh pro sudo -n launchctl kickstart -k system/up");
    assert_eq!(restarted(&mut m, "service/pro/up"), "Restarted up on pro");
    let failed = format!("Restart down on pro failed: {SUDO_NEEDS_PASSWORD}");
    assert_eq!(restarted(&mut m, "service/pro/down"), failed);
    assert_eq!(toasts(), ["Restarted up on pro".to_string(), failed]);
    // A later change posts nothing new.
    event(&mut m, Event::ModuleChanged { module: ID });
    assert!(toasts().is_empty());
    settle(&m);
}

#[test]
fn the_cli_restart_answers_its_caller_and_shows_no_toast() {
    let mut m = sys(SYSTEM, HOOKS);
    event(&mut m, Event::Started);
    toasts();
    let dry = ask(&mut m, &["restart", "up"], false).unwrap();
    assert_eq!(dry, "would run: sudo -n launchctl kickstart -k system/up\nadd --yes to restart up on this Mac");
    let now = ask(&mut m, &["restart", "locked", "--yes"], false);
    assert_eq!(later_answer(now).unwrap_err(), format!("Restart locked on this Mac failed: {SUDO_NEEDS_PASSWORD}"));
    event(&mut m, Event::ModuleChanged { module: ID });
    assert!(toasts().is_empty());
    assert!(m.shared.lock().toast.is_none());
    settle(&m);
}

#[test]
fn a_root_only_log_is_tailed_again_through_sudo_n() {
    let mut m = sys(SYSTEM, HOOKS);
    let now = ask(&mut m, &["tail", "up"], false);
    assert_eq!(later_answer(now), Ok("root's last of /var/log/root.log\n".into()));
    assert_eq!(m.shared.lock().tail.as_ref().map(|t| t.shown.clone()).unwrap_or_default(), "sudo -n tail -n 100 /var/log/root.log");
    // A readable log needs no sudo; a denied one sudo would prompt for says so.
    let now = ask(&mut m, &["tail", "locked"], false);
    assert_eq!(later_answer(now), Ok("last of /var/log/locked.log\n".into()));
    assert_eq!(m.shared.lock().tail.as_ref().map(|t| t.shown.clone()).unwrap_or_default(), "tail -n 100 /var/log/locked.log");
    // Over ssh.
    let now = ask(&mut m, &["tail", "pro", "up"], false);
    let want = "tail via pro: exec /usr/bin/sudo -n /usr/bin/tail -n 100 -- '/var/log/root.log'\n";
    assert_eq!(later_answer(now), Ok(want.into()));
    assert_eq!(m.shared.lock().tail.as_ref().map(|t| t.shown.clone()).unwrap_or_default(), "ssh pro sudo -n tail -n 100 /var/log/root.log");
}

#[test]
fn a_system_daemon_is_checked_in_the_system_domain() {
    let mut m = sys(SYSTEM, Hooks { uid: || None, ..HOOKS });
    let out = ask(&mut m, &["services"], false).unwrap();
    assert!(out.contains("up") && out.contains("running"), "{out}");
}

#[test]
fn an_older_tail_still_answers_but_leaves_a_newer_one_alone() {
    let slow = Hooks {
        run: |argv, budget| {
            std::thread::sleep(Duration::from_millis(200));
            (HOOKS.run)(argv, budget)
        },
        ..HOOKS
    };
    let mut m = sys(SYSTEM, slow);
    let now = ask(&mut m, &["tail", "locked"], false);
    // A newer tail took its place while it ran.
    if let Some(t) = m.shared.lock().tail.as_mut() {
        t.id += 1;
    }
    assert_eq!(later_answer(now), Ok("last of /var/log/locked.log\n".into()));
    assert_eq!(m.shared.lock().tail.as_ref().map(|t| t.text.clone()), Some(None));
}
