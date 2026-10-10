//! Fakes for the module's tests. `run` answers from fixtures by argv, so no test runs the
//! probe.

use std::time::Duration;

use super::io::Hooks;
use super::probe::PROBE;
use super::run::Exit;

/// The real probe output from mbp-server.
pub const SERVER: &str = include_str!("fixtures/probe_mbp_server.txt");

pub const HOOKS: Hooks = Hooks { post: || {}, now: || 1_000, run };

/// The probe answers `SERVER`; anything else is missing.
fn run(argv: &[String], _budget: Duration) -> Result<Exit, String> {
    match argv {
        [sh, c, script] if sh == "/bin/sh" && c == "-c" && script == PROBE => {
            Ok(Exit { code: Some(0), stdout: SERVER.into(), stderr: String::new() })
        }
        _ => Err(format!("{}: No such file or directory", argv.first().map_or("", String::as_str))),
    }
}
