//! Fakes for the module's tests. `run` answers from fixtures by argv, so no test runs
//! herdr or curl; `sleep` stands 1 s for 20 ms, so timers fire within a test.

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
    host: || Some("this-mac".into()),
    sleep: |d| thread::sleep(Duration::from_millis(d.as_secs() * 20)),
    utc_offset: |_| 0,
};

#[expect(clippy::unnecessary_wraps, reason = "fake hooks answer as the real ones do")]
pub fn exit(code: i32, stdout: &str, stderr: &str) -> Result<Exit, String> {
    Ok(Exit { code: Some(code), stdout: stdout.into(), stderr: stderr.into() })
}

/// Canned answers by argv:
/// - herdr `agent list` (this Mac) and `--machine server`: `AGENT_LIST`; `--machine
///   empty`: no agents; `--machine slow`: `AGENT_LIST` after 300 ms; `--machine gone`:
///   the spawn fails; any other machine: `UNKNOWN_MACHINE`, exit 2;
/// - curl `http://ok/ok`: `OK`; `http://queue/ok`: the queue check false; anything else
///   refused (exit 7).
fn run(argv: &[String], _budget: Duration) -> Result<Exit, String> {
    let words: Vec<&str> = argv.iter().map(String::as_str).collect();
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
