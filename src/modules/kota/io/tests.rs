use super::*;
use crate::modules::kota::presence::State;
use crate::modules::kota::testkit::{HOOKS, exit};
use std::time::Instant;

fn settings(machine: &str, dash: &str) -> Settings {
    Settings { machine: machine.into(), dash: dash.into(), ..Settings::default() }
}

/// Wait up to 5 s for `cond`.
fn wait(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Run one round and wait for it to land.
fn one_round(sh: &Arc<Shared>, s: &Settings) {
    assert!(round(sh, s, HOOKS));
    wait("round", || !sh.lock().running);
}

#[test]
fn argvs_name_the_machine_unless_it_is_this_mac() {
    let s = settings("mbp-server", "https://h:8310/");
    let (list, curl) = argvs(&s, "/opt/homebrew/bin/herdr", Some("laptop"));
    assert_eq!(list, ["/opt/homebrew/bin/herdr", "--machine", "mbp-server", "agent", "list"]);
    assert_eq!(curl, ["/usr/bin/curl", "-sS", "--fail", "--max-time", "5", "https://h:8310/ok"]);
    let (list, _) = argvs(&s, "herdr", Some("mbp-server"));
    assert_eq!(list, ["herdr", "agent", "list"]);
    let (list, _) = argvs(&settings("local", "http://x"), "herdr", None);
    assert_eq!(list, ["herdr", "agent", "list"]);
}

#[test]
fn a_round_runs_both_children_and_debounces() {
    let sh = Arc::new(Shared::default());
    one_round(&sh, &settings("server", "http://ok"));
    {
        let p = sh.lock();
        assert_eq!(p.presence.state, State::Thinking);
        assert_eq!(p.presence.seen.pane.as_ref().map(|p| p.title.as_str()), Some("Flick KOTA cards design brainstorm"));
        assert_eq!((p.started_at, p.ended_at, p.presence.checked_at), (Some(1_000), Some(1_000), Some(1_000)));
        assert_eq!(p.transitions.len(), 1);
    }
    one_round(&sh, &settings("server", "http://queue"));
    assert_eq!(sh.lock().presence.state, State::Degraded);
    assert_eq!(sh.lock().presence.seen.failing, ["queue"]);
    // KOTA's pane gone: down only on the second round.
    one_round(&sh, &settings("empty", "http://ok"));
    assert_eq!(sh.lock().presence.state, State::Degraded);
    one_round(&sh, &settings("empty", "http://ok"));
    assert_eq!(sh.lock().presence.state, State::Down);
    // Nothing answers: offline (two failing rounds in a row already).
    one_round(&sh, &settings("nope", "http://refused"));
    let p = sh.lock();
    assert_eq!(p.presence.state, State::Offline);
    assert_eq!(p.presence.errors[0], "herdr: unknown machine 'nope'; use `herdr machine list`");
    assert!(p.presence.errors[1].starts_with("dash: curl: (7) Failed to connect"), "{:?}", p.presence.errors);
}

#[test]
fn a_spawn_failure_is_an_error_not_a_hang() {
    let sh = Arc::new(Shared::default());
    let s = settings("gone", "http://ok");
    one_round(&sh, &s);
    one_round(&sh, &s);
    assert_eq!(sh.lock().presence.state, State::Degraded);
    assert_eq!(sh.lock().presence.errors, ["herdr: herdr not found"]);
    assert_eq!(exit(0, "a", "b").unwrap().stdout, "a");
}

#[test]
fn one_round_at_a_time_and_sleep_drops_the_round_in_flight() {
    let sh = Arc::new(Shared::default());
    let s = settings("slow", "http://ok");
    assert!(round(&sh, &s, HOOKS));
    assert!(!round(&sh, &s, HOOKS));
    assert_eq!(refresh(&sh, &s, HOOKS), "kota: a check is running");
    suspend(&sh);
    {
        let p = sh.lock();
        assert!(p.asleep && p.presence.stale && !p.running);
    }
    thread::sleep(Duration::from_millis(400));
    // The slow round finished after the sleep: its result is dropped.
    let p = sh.lock();
    assert_eq!((p.presence.state, p.ended_at), (State::Unknown, None));
}

#[test]
fn refresh_is_rate_limited() {
    let sh = Arc::new(Shared::default());
    let s = settings("server", "http://ok");
    assert_eq!(refresh(&sh, &s, HOOKS), "kota: checking");
    wait("round", || !sh.lock().running);
    assert_eq!(refresh(&sh, &s, HOOKS), "kota: checked 0 s ago; refresh again in 10 s");
    sh.lock().started_at = Some(1_000 - REFRESH_EVERY);
    assert_eq!(refresh(&sh, &s, HOOKS), "kota: checking");
    wait("round", || !sh.lock().running);
}

#[test]
fn tick_starts_a_due_round_else_arms_the_timer() {
    let sh = Arc::new(Shared::default());
    let s = settings("server", "http://ok");
    // Never checked: due at once.
    tick(&sh, &s, HOOKS);
    wait("round", || sh.lock().ended_at.is_some());
    wait("round end", || !sh.lock().running);
    // Thinking: the next round is due in 15 s, and the timer is armed for it.
    tick(&sh, &s, HOOKS);
    assert_eq!(sh.lock().timer_due, Some(1_015));
    let armed = sh.lock().timer;
    // Ticking again keeps that timer.
    tick(&sh, &s, HOOKS);
    assert_eq!(sh.lock().timer, armed);
    // The timer fires (15 test seconds are 300 ms) and clears itself.
    wait("timer", || sh.lock().timer_due.is_none());
    // poll_secs 0: no timer, no round.
    let on_demand = Settings { poll_secs: 0, ..s.clone() };
    sh.lock().ended_at = None;
    tick(&sh, &on_demand, HOOKS);
    let p = sh.lock();
    assert!(!p.running && p.timer_due.is_none());
    drop(p);
    // Asleep or running: nothing.
    sh.lock().asleep = true;
    tick(&sh, &s, HOOKS);
    assert!(!sh.lock().running);
}

#[test]
fn wake_waits_then_runs_a_round_with_grace() {
    let sh = Arc::new(Shared::default());
    let s = settings("nope", "http://refused");
    {
        let mut p = sh.lock();
        p.ended_at = Some(990);
        p.fast_until = Some(2_000);
    }
    suspend(&sh);
    resume(&sh, &s, HOOKS);
    {
        let p = sh.lock();
        assert!(!p.asleep && !p.running);
        assert_eq!((p.woke_at, p.not_before, p.timer_due), (Some(1_000), Some(1_005), Some(1_005)));
    }
    // A round in the grace period that fails keeps the last state, stale.
    one_round(&sh, &s);
    let p = sh.lock();
    assert!(p.presence.stale);
    assert_eq!((p.presence.state, p.presence.checked_at), (State::Unknown, None));
}

#[test]
fn reset_and_stop_retire_everything() {
    let sh = Arc::new(Shared::default());
    one_round(&sh, &settings("server", "http://ok"));
    arm(&sh, 60, HOOKS);
    let (epoch, timer) = {
        let p = sh.lock();
        (p.epoch, p.timer)
    };
    sh.reset();
    {
        let p = sh.lock();
        assert_eq!((p.epoch, p.timer, p.presence.state, p.timer_due), (epoch + 1, timer + 1, State::Unknown, None));
    }
    arm(&sh, 60, HOOKS);
    sh.stop();
    let p = sh.lock();
    assert_eq!((p.epoch, p.timer_due, p.running), (epoch + 2, None, false));
}
