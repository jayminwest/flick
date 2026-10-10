//! The real I/O behind `io::Hooks`: a child process with a time budget, the herdr
//! executable, and this Mac's short host name. Only background threads call these.
//!
//! Copied in small from `herdr/remote.rs` and `sys/run.rs` (modules never import each
//! other): every `HERDR_*` variable is removed from the child's environment, so a Flick
//! started from a herdr pane does not retarget the CLI at that pane's session.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

/// A finished child process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exit {
    /// `None` when a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Where Homebrew installs herdr; used when `PATH` (minimal for a Finder-launched app)
/// does not have it.
const HOMEBREW: &str = "/opt/homebrew/bin";

/// The herdr executable for config value `herdr`: a path (with `~/`) as given; a bare
/// name looked up on `PATH`, then in `/opt/homebrew/bin`; else the name unchanged.
pub fn resolve(herdr: &str) -> String {
    if let Some(rest) = herdr.strip_prefix("~/") {
        return dirs::home_dir().map_or_else(|| herdr.to_string(), |h| h.join(rest).display().to_string());
    }
    if herdr.contains('/') {
        return herdr.to_string();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([PathBuf::from(HOMEBREW)])
        .map(|dir| dir.join(herdr))
        .find(|p| p.is_file())
        .map_or_else(|| herdr.to_string(), |p| p.display().to_string())
}

/// Removes every `HERDR_*` name in `env` (Flick's environment) from `cmd`.
fn strip_herdr_env(cmd: &mut Command, env: impl IntoIterator<Item = OsString>) {
    for name in env {
        if name.to_string_lossy().starts_with("HERDR_") {
            cmd.env_remove(name);
        }
    }
}

/// Run `argv` (no shell) with stdin closed and no `HERDR_*` variables; kill it when
/// `budget` runs out.
pub fn run(argv: &[String], budget: Duration) -> Result<Exit, String> {
    let (program, rest) = argv.split_first().ok_or("empty command")?;
    let mut cmd = Command::new(program);
    cmd.args(rest).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    strip_herdr_env(&mut cmd, std::env::vars_os().map(|(name, _)| name));
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{program} not found"),
        _ => format!("{program}: {e}"),
    })?;
    // Read on threads so a full pipe never stalls the child while we wait for it.
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
    let deadline = Instant::now() + budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // A grandchild (ssh) may hold the pipes open; leave the readers be.
                return Err(format!("timed out after {} s", budget.as_secs_f32()));
            }
            Err(e) => return Err(format!("{program}: {e}")),
        }
    };
    let (stdout, stderr) = (stdout.join().unwrap_or_default(), stderr.join().unwrap_or_default());
    Ok(Exit { code: status.code(), stdout, stderr })
}

/// This Mac's short host name, lowercased (`hostname -s`, once per process).
pub fn host() -> Option<String> {
    static HOST: OnceLock<Option<String>> = OnceLock::new();
    HOST.get_or_init(|| {
        let argv = ["/bin/hostname".to_string(), "-s".to_string()];
        let out = run(&argv, Duration::from_secs(2)).ok()?;
        let name = out.stdout.trim().to_ascii_lowercase();
        (out.code == Some(0) && !name.is_empty()).then_some(name)
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn herdr_variables_are_removed_from_the_child() {
        let mut cmd = Command::new("/usr/bin/true");
        cmd.env("HERDR_PANE_ID", "x").env("KEEP", "1");
        let env = ["HERDR_PANE_ID", "HERDR_SOCKET_PATH", "PATH"].map(OsString::from);
        strip_herdr_env(&mut cmd, env);
        let removed: Vec<_> = cmd.get_envs().filter(|(_, v)| v.is_none()).map(|(k, _)| k.to_owned()).collect();
        assert_eq!(removed, ["HERDR_PANE_ID", "HERDR_SOCKET_PATH"].map(OsString::from));
        assert!(cmd.get_envs().any(|(k, v)| k == "KEEP" && v.is_some()));
    }

    #[test]
    fn resolve_keeps_paths_and_falls_back_to_the_name() {
        assert_eq!(resolve("/opt/x/herdr"), "/opt/x/herdr");
        assert!(resolve("~/bin/herdr").ends_with("/bin/herdr"));
        assert!(!resolve("~/bin/herdr").starts_with('~'));
        assert_eq!(resolve("no-such-herdr-binary-xyz"), "no-such-herdr-binary-xyz");
    }
}
