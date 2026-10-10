//! Fakes for the module's tests. `run` answers from fixtures by argv, so no test runs
//! herdr, curl or Flick; `feed` stands in for ssh; `sleep` stands 1 s for 20 ms, so timers
//! fire within a test.

use std::cell::{Cell, RefCell};
use std::thread;
use std::time::Duration;

use super::io::Hooks;
use super::run::Exit;

/// `herdr agent list` on mbp-server (2026-10-10): the KOTA pane, working.
pub const AGENT_LIST: &str = include_str!("fixtures/agent_list_mbp_server.json");
/// kota-dash `/ok` on mbp-server (2026-10-10): every check true.
pub const OK: &str = include_str!("fixtures/ok_mbp_server.json");
/// herdr's stderr for an unknown `--machine` (exit 2).
pub const UNKNOWN_MACHINE: &str = include_str!("fixtures/herdr_unknown_machine.txt");

pub const HOOKS: Hooks = Hooks {
    post: || {},
    now: || 1_000,
    run,
    feed,
    exe: || Ok(EXE.into()),
    host: || Some("this-mac".into()),
    sleep: |d| thread::sleep(Duration::from_millis(d.as_secs() * 20)),
    utc_offset: |_| 0,
};

#[expect(clippy::unnecessary_wraps, reason = "fake hooks answer as the real ones do")]
pub fn exit(code: i32, stdout: &str, stderr: &str) -> Result<Exit, String> {
    Ok(Exit { code: Some(code), stdout: stdout.into(), stderr: stderr.into() })
}

/// The fake path of this binary.
pub const EXE: &str = "/fake/Flick";

thread_local! {
    /// Child calls on this thread, in order: `run <argv>` or `feed <argv> <<stdin`.
    static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Self-calls that answer busy before the next one works.
    static BUSY: Cell<u32> = const { Cell::new(0) };
}

/// The calls this thread made since the last take.
pub fn take_calls() -> Vec<String> {
    CALLS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// The next `n` self-calls on this thread find Flick busy.
pub fn set_busy(n: u32) {
    BUSY.set(n);
}

fn call(line: String) {
    CALLS.with(|c| c.borrow_mut().push(line));
}

/// A self-call: busy while `BUSY` lasts; a body `flick-down` cannot reach Flick; else the
/// id after `--id` (or `m1`).
fn flick(words: &[&str]) -> Result<Exit, String> {
    if BUSY.get() > 0 {
        BUSY.set(BUSY.get() - 1);
        return exit(1, "", "flick: Flick is busy; try again\n");
    }
    if words.last() == Some(&"flick-down") {
        return exit(3, "", "flick: cannot reach Flick: Connection refused\n");
    }
    let id = words.iter().position(|w| *w == "--id").and_then(|i| words.get(i + 1)).unwrap_or(&"m1");
    exit(0, &format!("{id}\n"), "")
}

/// ssh by target: `ok` queues; `down` refuses; `slow` times out; `silent` exits 1 quietly;
/// anything else does not resolve.
fn feed(argv: &[String], input: &str, _budget: Duration) -> Result<Exit, String> {
    call(format!("feed {} <<{input}", argv.join(" ")));
    match argv.get(5).map(String::as_str) {
        Some("ok") => exit(0, "queued for KOTA (t1)\n", ""),
        Some("down") => exit(255, "", "ssh: connect to host down port 22: Connection refused\n"),
        Some("slow") => Err("timed out after 15 s".into()),
        Some("silent") => exit(1, "", ""),
        Some("killed") => Ok(Exit { code: None, ..Exit::default() }),
        _ => exit(255, "", "ssh: Could not resolve hostname x: nodename nor servname provided\n"),
    }
}

/// Canned answers by argv:
/// - herdr `agent list` (this Mac) and `--machine server`: `AGENT_LIST`; `--machine
///   empty`: no agents; `--machine slow`: `AGENT_LIST` after 300 ms; `--machine gone`:
///   the spawn fails; any other machine: `UNKNOWN_MACHINE`, exit 2;
/// - curl `http://ok/ok`: `OK`; `http://queue/ok`: the queue check false; anything else
///   refused (exit 7);
/// - this binary (`EXE`): `flick`.
fn run(argv: &[String], _budget: Duration) -> Result<Exit, String> {
    let words: Vec<&str> = argv.iter().map(String::as_str).collect();
    if words.first() == Some(&EXE) {
        call(format!("run {}", argv.join(" ")));
        return flick(&words);
    }
    match words.as_slice() {
        [_, "agent", "list"] | [_, "--machine", "server", "agent", "list"] => exit(0, AGENT_LIST, ""),
        [_, "--machine", "empty", "agent", "list"] => {
            exit(0, r#"{"id":"cli:agent:list","result":{"agents":[],"type":"agent_list"}}"#, "")
        }
        [_, "--machine", "slow", "agent", "list"] => {
            thread::sleep(Duration::from_millis(300));
            exit(0, AGENT_LIST, "")
        }
        [_, "--machine", "gone", ..] => Err("herdr not found".into()),
        [_, "--machine", ..] => exit(2, "", UNKNOWN_MACHINE),
        ["/usr/bin/curl", .., "http://ok/ok"] => exit(0, OK, ""),
        ["/usr/bin/curl", .., "http://queue/ok"] => exit(0, &OK.replace("\"queue\": true", "\"queue\": false"), ""),
        _ => exit(7, "", "curl: (7) Failed to connect to ok port 80 after 0 ms: Couldn't connect to server\n"),
    }
}
