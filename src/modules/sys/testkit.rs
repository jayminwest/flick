//! Fakes for the module's tests. `run` answers from fixtures by argv, so no test runs the
//! probe, curl, launchctl, pgrep or a configured command; `connect` never opens a socket.

use std::time::Duration;

use super::run::Exit;
use super::io::Hooks;
use super::probe::PROBE;

/// The real probe output from mbp-server.
pub const SERVER: &str = include_str!("fixtures/probe_mbp_server.txt");

pub const HOOKS: Hooks = Hooks { post: || {}, now: || 1_000, run, connect, uid: || Some(501) };

#[expect(clippy::unnecessary_wraps, reason = "fake hooks answer as the real ones do")]
fn exit(code: i32, stdout: &str, stderr: &str) -> Result<Exit, String> {
    Ok(Exit { code: Some(code), stdout: stdout.into(), stderr: stderr.into() })
}

/// Canned answers by argv:
/// - the probe: `SERVER`;
/// - curl: `http://ok/` 200 in 12 ms, `http://slow/` 200 in 1.5 s, `http://gone/` 404,
///   anything else refused;
/// - launchctl print: `gui/501/up` running, `gui/501/killed` running after a kill, else not
///   loaded;
/// - pgrep -x: `syncthing` runs;
/// - `/fake/print <text>` prints its text; `/fake/sleep <ms>` sleeps, then prints 1.
fn run(argv: &[String], _budget: Duration) -> Result<Exit, String> {
    let words: Vec<&str> = argv.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["/bin/sh", "-c", script] if *script == PROBE => exit(0, SERVER, ""),
        ["/usr/bin/curl", .., "http://ok/"] => exit(0, "200 0.012", ""),
        ["/usr/bin/curl", .., "http://slow/"] => exit(0, "200 1.5", ""),
        ["/usr/bin/curl", .., "http://gone/"] => exit(0, "404 0.002", ""),
        ["/usr/bin/curl", ..] => exit(7, "000", "curl: (7) Failed to connect\n"),
        ["/bin/launchctl", "print", "gui/501/up"] => {
            exit(0, include_str!("fixtures/launchctl_running.txt"), "")
        }
        ["/bin/launchctl", "print", "gui/501/killed"] => {
            exit(0, include_str!("fixtures/launchctl_signal.txt"), "")
        }
        ["/bin/launchctl", ..] => exit(113, "", include_str!("fixtures/launchctl_missing.txt")),
        ["/usr/bin/pgrep", "-x", "syncthing"] => exit(0, "917\n958\n", ""),
        ["/usr/bin/pgrep", ..] => exit(1, "", ""),
        ["/fake/print", text] => exit(0, text, ""),
        ["/fake/sleep", ms] => {
            std::thread::sleep(Duration::from_millis(ms.parse().unwrap_or(0)));
            exit(0, "1", "")
        }
        _ => Err(format!("{}: No such file or directory", words.first().unwrap_or(&""))),
    }
}

/// `open:<n>` connects in n ms; anything else is refused.
fn connect(target: &str, _budget: Duration) -> Result<Duration, String> {
    match target.strip_prefix("open:").and_then(|ms| ms.parse().ok()) {
        Some(ms) => Ok(Duration::from_millis(ms)),
        None => Err(format!("{target}: Connection refused")),
    }
}
