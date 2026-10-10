use super::*;
use crate::config::parse;
use crate::modules::sys::settings::Settings;
use crate::modules::sys::testkit::HOOKS;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

fn machines(text: &str) -> Vec<Machine> {
    parse(text).unwrap().section("sys").unwrap().unwrap().get::<Settings>().unwrap().check().unwrap().machine
}

fn one(fields: &str) -> Machine {
    machines(&format!("[[sys.machine]]\nname = \"m\"\n{fields}")).remove(0)
}

#[test]
fn a_flick_machine_answers_with_its_snapshot() {
    let f = fetch(&one("via = \"flick\"\nhost = \"server\""), HOOKS).unwrap();
    assert_eq!((f.source, f.note, f.snapshot["host"].as_str()), (Via::Flick, None, Some("server")));
    assert_eq!(fetch(&one("via = \"flick\"\nhost = \"odd\""), HOOKS).unwrap_err(), "flick: unexpected reply \"text\"");
    assert_eq!(fetch(&one("via = \"flick\"\nhost = \"busy\""), HOOKS).unwrap_err(), "sys: no snapshot yet (probe still running)");
    // Without ssh, too old and unreachable are errors.
    assert_eq!(fetch(&one("via = \"flick\"\nhost = \"old\""), HOOKS).unwrap_err(), "flick too old (no sys snapshot)");
    assert_eq!(fetch(&one("via = \"flick\""), HOOKS).unwrap_err(), "can't reach Flick at m:7419 (Connection refused)");
}

#[test]
fn a_flick_too_old_or_unreachable_falls_back_to_ssh() {
    let f = fetch(&one("via = \"flick\"\nhost = \"old\"\nssh = \"pro\""), HOOKS).unwrap();
    assert_eq!((f.source, f.note.as_deref()), (Via::Ssh, Some("flick too old (no sys snapshot)")));
    assert_eq!(f.snapshot["host"], "mac-pro");
    let f = fetch(&one("via = \"flick\"\nssh = \"pro\""), HOOKS).unwrap();
    assert_eq!(f.note.as_deref(), Some("can't reach Flick at m:7419 (Connection refused)"));
    let e = fetch(&one("via = \"flick\"\nhost = \"old\"\nssh = \"refused\""), HOOKS).unwrap_err();
    assert_eq!(e, "flick too old (no sys snapshot); ssh: ssh: connect to host pro port 22: Connection refused");
}

#[test]
fn an_ssh_machine_runs_the_probe_and_its_checks() {
    let text = r#"
[[sys.machine]]
name = "pro"
via = "ssh"
ssh = "pro"
[[sys.machine.service]]
name = "agent"
kind = "launchd"
target = "up"
[[sys.machine.service]]
name = "gone"
kind = "launchd"
target = "gone"
[[sys.machine.service]]
name = "ollama"
kind = "process"
target = "ollama"
[[sys.machine.service]]
name = "web"
kind = "http"
target = "http://ok/"
[[sys.machine.service]]
name = "port"
kind = "tcp"
target = "closed:1"
"#;
    let f = fetch(&machines(text)[0], HOOKS).unwrap();
    assert_eq!((f.source, f.note), (Via::Ssh, None));
    let snap = &f.snapshot;
    assert_eq!((snap["host"].as_str(), snap["battery"].is_null(), snap["schema"].as_u64()), (Some("mac-pro"), true, Some(1)));
    let field = |s: &Value, k: &str| s[k].as_str().unwrap_or_default().to_string();
    let services: Vec<[String; 3]> =
        snap["services"].as_array().unwrap().iter().map(|s| [field(s, "name"), field(s, "status"), field(s, "reason")]).collect();
    assert_eq!(services[0][..2], ["agent", "ok"]);
    assert_eq!(services[1], ["gone", "fail", "not loaded in gui/502"]);
    assert_eq!(services[2], ["ollama", "ok", "running (pid 812)"]);
    assert_eq!(services[3], ["web", "ok", "HTTP 200 in 12 ms"]);
    assert_eq!(services[4], ["port", "fail", "closed:1: Connection refused"]);
}

#[test]
fn ssh_failures_are_errors() {
    assert_eq!(fetch(&one("via = \"ssh\"\nssh = \"refused\""), HOOKS).unwrap_err(), "ssh: ssh: connect to host pro port 22: Connection refused");
    assert_eq!(fetch(&one("via = \"ssh\"\nssh = \"mute\""), HOOKS).unwrap_err(), "ssh: exit 0");
    assert_eq!(fetch(&one("via = \"ssh\"\nssh = \"nowhere\""), HOOKS).unwrap_err(), "ssh: timed out after 10 s");
    // Settings never allow it, but a flick machine without a target cannot fall back.
    let mut m = one("via = \"ssh\"\nssh = \"pro\"");
    m.ssh = None;
    assert_eq!(fetch(&m, HOOKS).unwrap_err(), "no ssh target");
}

fn shared(text: &str) -> Arc<Shared> {
    let sh = Arc::new(Shared::default());
    sh.lock().fleet.set_machines(&machines(text));
    sh
}

const TWO: &str = "[[sys.machine]]\nname = \"a\"\nvia = \"flick\"\nhost = \"server\"\n[[sys.machine]]\nname = \"b\"\nvia = \"ssh\"\nssh = \"refused\"\n";

/// Generous: a slow CI runner still finishes well inside it.
const DEADLINE: Duration = Duration::from_secs(15);

#[test]
fn a_round_reads_every_due_machine_once() {
    let sh = shared(TWO);
    assert!(round(&sh, 15, HOOKS));
    // A machine being read is not read twice at once.
    assert!(!round(&sh, 15, HOOKS) || !sh.lock().fleet.busy());
    assert!(sh.wait(DEADLINE, |s| !s.fleet.busy()));
    {
        let st = sh.lock();
        assert_eq!(st.fleet.slots[0].fetched_at, Some(1_000));
        assert_eq!(st.fleet.slots[1].error.as_deref(), Some("ssh: ssh: connect to host pro port 22: Connection refused"));
    }
    // Nothing is due again within min_age.
    assert!(!round(&sh, 15, HOOKS));
    assert!(round(&sh, 0, HOOKS));
    assert!(sh.wait(DEADLINE, |s| !s.fleet.busy()));
}

/// Set by the test once the round is forgotten: `gated_ask` answers only then.
static GATE: AtomicBool = AtomicBool::new(false);

#[expect(clippy::unnecessary_wraps, reason = "stands in for PeerHooks::ask")]
fn gated_ask(host: &str, _words: &[String], _flags: Flags) -> Result<Reply, String> {
    let deadline = Instant::now() + DEADLINE;
    while !GATE.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(Reply::Ok(serde_json::json!({ "host": host })))
}

/// Wait until every round thread let go of `sh`, so its result was applied or dropped.
fn threads_done(sh: &Arc<Shared>) -> bool {
    let deadline = Instant::now() + DEADLINE;
    while Arc::strong_count(sh) > 1 {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

#[test]
fn a_round_dropped_by_sleep_stores_nothing() {
    let hooks = Hooks { ask: gated_ask, ..HOOKS };
    let sh = shared("[[sys.machine]]\nname = \"a\"\nvia = \"flick\"\nhost = \"gated\"\n");
    assert!(round(&sh, 0, hooks));
    sh.lock().fleet.forget_round();
    // The answer comes in only after the round was dropped, and the thread has ended.
    GATE.store(true, Ordering::SeqCst);
    assert!(threads_done(&sh), "the read thread ended");
    let st = sh.lock();
    assert_eq!((st.fleet.slots[0].tried_at, st.fleet.busy()), (None, false));
}

/// Held until the test lets it go: the peer `hang` answers only then.
static HELD: AtomicBool = AtomicBool::new(true);
/// How often the fast peer was asked.
static FAST: AtomicUsize = AtomicUsize::new(0);

#[expect(clippy::unnecessary_wraps, reason = "stands in for PeerHooks::ask")]
fn held_ask(host: &str, _words: &[String], _flags: Flags) -> Result<Reply, String> {
    if host == "fast" {
        FAST.fetch_add(1, Ordering::SeqCst);
    }
    let deadline = Instant::now() + DEADLINE;
    while host == "hang" && HELD.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(Reply::Ok(serde_json::json!({ "host": host })))
}

#[test]
fn a_slow_machine_holds_only_itself() {
    let hooks = Hooks { ask: held_ask, ..HOOKS };
    let sh = shared("[[sys.machine]]\nname = \"a\"\nvia = \"flick\"\nhost = \"fast\"\n[[sys.machine]]\nname = \"b\"\nvia = \"flick\"\nhost = \"hang\"\n");
    assert!(round(&sh, 0, hooks));
    assert!(sh.wait(DEADLINE, |s| !s.fleet.slots[0].busy && s.fleet.slots[0].fetched_at.is_some()));
    assert!(sh.lock().fleet.slots[1].busy);
    // The next round reads `a` again without waiting for `b`, and leaves `b` alone.
    assert_eq!(FAST.load(Ordering::SeqCst), 1);
    assert!(round(&sh, 0, hooks));
    assert!(sh.wait(DEADLINE, |s| !s.fleet.slots[0].busy));
    assert_eq!(FAST.load(Ordering::SeqCst), 2);
    // A waiter on the whole fleet is woken when `b` ends.
    HELD.store(false, Ordering::SeqCst);
    assert!(sh.wait(DEADLINE, |s| !s.fleet.busy()));
    assert!(sh.lock().fleet.slots[1].fetched_at.is_some());
}

/// Every post (`ModuleChanged`) the tick test's hooks sent.
static TICKS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn a_tick_posts_once_and_only_one_is_pending() {
    let hooks = Hooks { post: || {
            TICKS.fetch_add(1, Ordering::SeqCst);
        }, ..HOOKS };
    let sh = shared(TWO);
    tick_after(&sh, Duration::from_millis(50), hooks);
    assert!(sh.lock().fleet.ticking);
    // A second ask while one is pending starts none.
    tick_after(&sh, Duration::ZERO, hooks);
    assert!(sh.wait(DEADLINE, |s| !s.fleet.ticking));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(TICKS.load(Ordering::SeqCst), 1);
}

static POSTS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn the_timer_posts_until_stopped() {
    let hooks = Hooks { post: || {
            POSTS.fetch_add(1, Ordering::SeqCst);
        }, ..HOOKS };
    let sh = shared(TWO);
    start_timer(&sh, Duration::from_millis(20), hooks);
    // Slow CI runners: wait for two timer posts rather than a fixed sleep.
    let deadline = Instant::now() + DEADLINE;
    while POSTS.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    stop_timer(&sh);
    std::thread::sleep(Duration::from_millis(50));
    let after = POSTS.load(Ordering::SeqCst);
    assert!(after >= 2, "{after}");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(POSTS.load(Ordering::SeqCst), after);
}

#[test]
fn old_flicks_are_told_apart_from_other_errors() {
    assert!(too_old("unknown module \"sys\" (modules: app)"));
    assert!(too_old("sys: unknown command \"snapshot\""));
    assert!(!too_old("sys: no snapshot yet"));
    assert!(!too_old("unknown module \"system\""));
}
