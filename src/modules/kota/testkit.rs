//! Fixtures for the module's tests: real output captured on mbp-server.

use super::run::Exit;

/// `herdr agent list` on mbp-server (2026-10-10): the KOTA pane, working.
pub const AGENT_LIST: &str = include_str!("fixtures/agent_list_mbp_server.json");
/// kota-dash `/ok` on mbp-server (2026-10-10): every check true.
pub const OK: &str = include_str!("fixtures/ok_mbp_server.json");
/// herdr's stderr for an unknown `--machine` (exit 2).
pub const UNKNOWN_MACHINE: &str = include_str!("fixtures/herdr_unknown_machine.txt");

#[expect(clippy::unnecessary_wraps, reason = "answers as a child-running hook does")]
pub fn exit(code: i32, stdout: &str, stderr: &str) -> Result<Exit, String> {
    Ok(Exit { code: Some(code), stdout: stdout.into(), stderr: stderr.into() })
}
