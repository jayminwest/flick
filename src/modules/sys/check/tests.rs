use super::*;
use crate::modules::sys::run::Exit;

// Real `launchctl print gui/501/<label>` output from mbp-server (macOS 27.0.1), with the
// argument and environment blocks emptied and the home directory renamed.
/// kota-life-ops: running, never exited.
const RUNNING: &str = include_str!("../fixtures/launchctl_running.txt");
/// pundit worker: running again after launchd's `SIGKILL`.
const SIGNAL: &str = include_str!("../fixtures/launchctl_signal.txt");
/// pundit board: running again after exit 143.
const EXITED: &str = include_str!("../fixtures/launchctl_exit.txt");
/// kota-heartbeat: a calendar job between runs, last exit 0.
const IDLE: &str = include_str!("../fixtures/launchctl_idle.txt");
/// A label launchd does not know (stderr, exit 113).
const MISSING: &str = include_str!("../fixtures/launchctl_missing.txt");

#[expect(clippy::unnecessary_wraps, reason = "the checks take what a hook returns")]
fn ok(stdout: &str) -> Result<Exit, String> {
    Ok(Exit { code: Some(0), stdout: stdout.into(), stderr: String::new() })
}

#[expect(clippy::unnecessary_wraps, reason = "the checks take what a hook returns")]
fn exit(code: i32, stderr: &str) -> Result<Exit, String> {
    Ok(Exit { code: Some(code), stdout: String::new(), stderr: stderr.into() })
}

#[expect(clippy::unnecessary_wraps, reason = "the checks take what a hook returns")]
fn killed() -> Result<Exit, String> {
    Ok(Exit { code: None, ..Exit::default() })
}

fn v(status: Status, reason: &str, value: Option<f64>) -> Verdict {
    Verdict { status, reason: reason.into(), value }
}

const NONE: Limits = Limits { warn: None, fail: None };

#[test]
fn status_words() {
    let all = [Status::Ok, Status::Warn, Status::Fail, Status::Unknown];
    assert_eq!(all.map(Status::as_str), ["ok", "warn", "fail", "unknown"]);
    assert_eq!(serde_json::to_value(all).unwrap(), serde_json::json!(["ok", "warn", "fail", "unknown"]));
}

#[test]
fn http_passes_2xx_and_3xx_and_grades_milliseconds() {
    assert_eq!(http(ok("200 0.012345"), NONE), v(Status::Ok, "HTTP 200 in 12 ms", Some(12.0)));
    assert_eq!(http(ok("302 0.5"), NONE).status, Status::Ok);
    let slow = Limits { warn: Some(100.0), fail: Some(1000.0) };
    assert_eq!(http(ok("200 0.25"), slow), v(Status::Warn, "HTTP 200 in 250 ms (warn >= 100)", Some(250.0)));
    assert_eq!(http(ok("200 1.5"), slow), v(Status::Fail, "HTTP 200 in 1500 ms (fail >= 1000)", Some(1500.0)));
    assert_eq!(http(ok("404 0.01"), NONE), Verdict::fail("HTTP 404"));
    assert_eq!(http(ok("503"), NONE), Verdict::fail("HTTP 503"));
    // Real curl output for a closed port (exit 7); its error text is the reason.
    let refused = exit(7, "curl: (7) Failed to connect to 127.0.0.1 port 1 after 0 ms: Couldn't connect to server\n");
    assert_eq!(http(refused, NONE), Verdict::fail("(7) Failed to connect to 127.0.0.1 port 1 after 0 ms: Couldn't connect to server"));
    assert_eq!(http(exit(28, ""), NONE), Verdict::fail("exit 28"));
    assert_eq!(http(killed(), NONE), Verdict::fail("killed by a signal"));
    assert_eq!(http(Err("timed out after 5 s".into()), NONE), Verdict::fail("timed out after 5 s"));
    assert_eq!(http(ok("000 0"), NONE), Verdict::unknown("unexpected curl output \"000 0\""));
    assert_eq!(http(ok(""), NONE).status, Status::Unknown);
}

#[test]
fn limits_grade_both_ways() {
    let up = Limits { warn: Some(10.0), fail: Some(50.0) };
    assert_eq!(command(ok("9\n"), up), v(Status::Ok, "9", Some(9.0)));
    assert_eq!(command(ok("10"), up), v(Status::Warn, "10 (warn >= 10)", Some(10.0)));
    assert_eq!(command(ok("50 items"), up), v(Status::Fail, "50 (fail >= 50)", Some(50.0)));
    // fail below warn: lower is worse.
    let down = Limits { warn: Some(20.0), fail: Some(5.0) };
    assert_eq!(command(ok("21"), down).status, Status::Ok);
    assert_eq!(command(ok("12.5"), down), v(Status::Warn, "12.5 (warn <= 20)", Some(12.5)));
    assert_eq!(command(ok("5"), down), v(Status::Fail, "5 (fail <= 5)", Some(5.0)));
    // One limit alone: higher is worse.
    let fail_only = Limits { warn: None, fail: Some(3.0) };
    assert_eq!(command(ok("2"), fail_only).status, Status::Ok);
    assert_eq!(command(ok("3"), fail_only).status, Status::Fail);
}

#[test]
fn command_needs_exit_zero_and_a_number_when_limited() {
    let up = Limits { warn: Some(1.0), fail: None };
    assert_eq!(command(ok("backlog: 3 files"), up).value, Some(3.0));
    assert_eq!(command(ok("none"), up), Verdict::unknown("no number in the output"));
    assert_eq!(command(ok("inf NaN"), up), Verdict::unknown("no number in the output"));
    assert_eq!(command(ok("\nall good\n"), NONE), v(Status::Ok, "all good", None));
    assert_eq!(command(ok(""), NONE), v(Status::Ok, "exit 0", None));
    assert_eq!(command(exit(2, "wc: x: open: No such file\n"), NONE), Verdict::fail("exit 2: wc: x: open: No such file"));
    assert_eq!(command(exit(1, ""), NONE), Verdict::fail("exit 1"));
    assert_eq!(command(killed(), NONE), Verdict::fail("killed by a signal"));
    assert_eq!(command(Err("no such file".into()), NONE), Verdict::fail("no such file"));
}

#[test]
fn tcp_reports_connect_time() {
    assert_eq!(tcp(Ok(Duration::from_micros(3_400)), NONE), v(Status::Ok, "open in 3 ms", Some(3.0)));
    let limit = Limits { warn: Some(2.0), fail: None };
    assert_eq!(tcp(Ok(Duration::from_millis(2)), limit).status, Status::Warn);
    assert_eq!(tcp(Err("connection refused".into()), NONE), Verdict::fail("connection refused"));
}

#[test]
fn launchd_reads_real_launchctl_output() {
    let d = "gui/501";
    assert_eq!(launchd(ok(RUNNING), d), v(Status::Ok, "running (pid 919)", None));
    assert_eq!(launchd(ok(SIGNAL), d), v(Status::Warn, "running (pid 95368); last killed: Killed: 9", None));
    assert_eq!(launchd(ok(EXITED), d), v(Status::Warn, "running (pid 95361); last exit 143", None));
    assert_eq!(launchd(ok(IDLE), d), v(Status::Ok, "not running; last exit 0", None));
    assert_eq!(launchd(exit(113, MISSING), d), Verdict::fail("not loaded in gui/501"));
    assert_eq!(launchd(exit(5, "Bad request.\n"), d), Verdict::fail("Bad request."));
    assert_eq!(launchd(Err("timed out".into()), d), Verdict::fail("timed out"));
}

#[test]
fn launchd_states_beyond_the_fixtures() {
    let job = |lines: &str| launchd(ok(&format!("gui/501/x = {{\n{lines}}}\n")), "gui/501");
    assert_eq!(job("\tstate = not running\n\tlast exit code = 1\n"), Verdict::fail("not running; last exit 1"));
    assert_eq!(job("\tstate = not running\n\tlast terminating signal = Terminated: 15\n"), Verdict::fail("not running; last killed: Terminated: 15"));
    assert_eq!(job("\tstate = not running\n"), v(Status::Ok, "not running", None));
    assert_eq!(job("\tstate = spawn scheduled\n"), v(Status::Warn, "state spawn scheduled", None));
    assert_eq!(job("\tstate = running\n"), v(Status::Ok, "running", None));
    // Only top-level lines count: a coalition's `state = active` is not the job's.
    assert_eq!(job("\tcoalition = {\n\t\tstate = active\n\t}\n\tnot a pair\n"), Verdict::unknown("no state in launchctl output"));
}

#[test]
fn process_reads_pgrep() {
    assert_eq!(process(ok("917\n958\n")), v(Status::Ok, "running (pid 917, 958)", None));
    assert_eq!(process(exit(1, "")), Verdict::fail("not running"));
    assert_eq!(process(exit(2, "pgrep: illegal option\n")), Verdict::unknown("illegal option"));
    assert_eq!(process(Err("spawn failed".into())), Verdict::fail("spawn failed"));
}
