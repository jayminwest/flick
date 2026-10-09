//! Remote herdr servers through the herdr CLI: `herdr --machine <label> agent list|read|
//! focus`. herdr owns SSH (its saved machine profiles); Flick runs the CLI and never opens a
//! connection itself. Each call costs one SSH round trip (2-3 s), so callers run it on a
//! background thread, one thread per machine.
//!
//! The CLI prints a JSON reply `{"id","result"}` on stdout and exits 0, except `agent
//! read`, which prints the output text itself. A server error is JSON `{"id","error"}` on
//! stderr with exit 1; a usage error (an unknown machine) is text on stderr with exit 2.
//!
//! herdr 0.9.1 has no forwarded event stream: `--machine` forwards one request and its
//! reply, and the CLI has no `events` command (spike, flick-4115). Remote machines are
//! polled.
//!
//! Every `HERDR_*` variable is removed from the child's environment, so a Flick started
//! from a herdr pane does not retarget the CLI at that pane's session.

use super::model::{self, Agent};
use serde::Deserialize;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// How long one CLI call may take before it is killed.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Where Homebrew installs herdr; used when `PATH` (minimal for a Finder-launched app)
/// does not have it.
const HOMEBREW: &str = "/opt/homebrew/bin";

/// Variables herdr sets in its panes. `Remote::command` also removes any other `HERDR_*`
/// in Flick's environment.
const HERDR_ENV: [&str; 6] = [
    "HERDR_BIN_PATH",
    "HERDR_ENV",
    "HERDR_PANE_ID",
    "HERDR_SOCKET_PATH",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
];

/// The herdr executable for config value `herdr`: a path (with `~/`) as given; a bare
/// name looked up on `PATH`, then in `/opt/homebrew/bin`; else the name unchanged.
pub fn resolve(herdr: &str) -> PathBuf {
    if herdr.contains('/') {
        return super::local::expand(herdr);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([PathBuf::from(HOMEBREW)])
        .map(|dir| dir.join(herdr))
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(herdr))
}

/// Runs the herdr CLI. Cheap; holds no process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    herdr: PathBuf,
    timeout: Duration,
}

impl Remote {
    pub fn new(herdr: impl Into<PathBuf>) -> Remote {
        Remote { herdr: herdr.into(), timeout: TIMEOUT }
    }

    /// The same runner with calls killed after `timeout`.
    pub fn with_timeout(self, timeout: Duration) -> Remote {
        Remote { timeout, ..self }
    }

    pub fn herdr(&self) -> &Path {
        &self.herdr
    }

    /// `machine`'s agents.
    pub fn list(&self, machine: &str) -> Result<Vec<Agent>, String> {
        let out = self.on(machine, &["agent", "list"])?;
        model::parse_agents(machine, &json(machine, &out)?)
    }

    /// The last `lines` lines of `target`'s output on `machine`, escapes stripped by herdr.
    pub fn read(&self, machine: &str, target: &str, lines: u32) -> Result<String, String> {
        let lines = lines.to_string();
        let args = ["agent", "read", target, "--source", "recent-unwrapped", "--lines", &lines];
        self.on(machine, &args)
    }

    /// Focus `target`'s pane on `machine`'s server. Whether a local herdr client attached
    /// to that machine follows is not known yet (flick-4115 left it to the manual smoke).
    pub fn focus(&self, machine: &str, target: &str) -> Result<(), String> {
        self.on(machine, &["agent", "focus", target]).map(drop)
    }

    /// The labels of the enabled saved machines (`herdr machine list --json`), in herdr's
    /// order.
    pub fn machines(&self) -> Result<Vec<String>, String> {
        #[derive(Deserialize)]
        struct Profile {
            label: String,
            #[serde(default)]
            enabled: bool,
        }
        let out = self.run(&["machine", "list", "--json"]).map_err(|e| format!("herdr: {e}"))?;
        let profiles: Vec<Profile> =
            serde_json::from_str(&out).map_err(|e| format!("herdr machine list: {e}"))?;
        Ok(profiles.into_iter().filter(|p| p.enabled).map(|p| p.label).collect())
    }

    /// `herdr --machine <machine> <args>`; errors start with the machine.
    fn on(&self, machine: &str, args: &[&str]) -> Result<String, String> {
        let mut all = vec!["--machine", machine];
        all.extend_from_slice(args);
        self.run(&all).map_err(|e| format!("{machine}: {e}"))
    }

    /// The command for `args`, with herdr's pane variables removed.
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.herdr);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for name in HERDR_ENV {
            cmd.env_remove(name);
        }
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("HERDR_") {
                cmd.env_remove(name);
            }
        }
        cmd
    }

    /// Run the CLI and return its stdout, or the error it reported. Killed at the timeout.
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let mut child = self.command(args).spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("{} not found", self.herdr.display()),
            _ => format!("{}: {e}", self.herdr.display()),
        })?;
        // Read on threads so a full pipe never stalls herdr while we wait for it to exit.
        let drain = |pipe: Option<Box<dyn Read + Send>>| {
            thread::spawn(move || {
                let mut out = String::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_string(&mut out);
                }
                out
            })
        };
        let stdout = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let stderr = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let deadline = Instant::now() + self.timeout;
        let exit = loop {
            match child.try_wait() {
                Ok(Some(exit)) => break exit,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    // The pipes may stay open in a grandchild (ssh); leave the readers be.
                    return Err(format!("timed out after {} s", self.timeout.as_secs_f32()));
                }
                Err(e) => return Err(e.to_string()),
            }
        };
        let out = stdout.join().unwrap_or_default();
        if exit.success() {
            return Ok(out);
        }
        let err = stderr.join().unwrap_or_default();
        Err(cli_error(&err).unwrap_or_else(|| format!("herdr exited with {exit}")))
    }
}

/// The message in the CLI's stderr: a JSON error's message, else its first non-empty
/// line without a leading `error: `.
fn cli_error(stderr: &str) -> Option<String> {
    let line = stderr.lines().map(str::trim).find(|l| !l.is_empty())?;
    if let Ok(reply) = serde_json::from_str::<Value>(line)
        && let Err(msg) = model::reply_result(&reply)
    {
        return Some(msg);
    }
    Some(line.strip_prefix("error: ").unwrap_or(line).to_string())
}

/// `out` as a JSON reply.
fn json(machine: &str, out: &str) -> Result<Value, String> {
    serde_json::from_str(out.trim()).map_err(|e| format!("{machine}: bad herdr reply: {e}"))
}

#[cfg(test)]
mod tests;
