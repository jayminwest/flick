//! This Mac's names. Runs on any thread.

use std::process::Command;

/// This Mac's Bonjour name as `scutil --get LocalHostName` prints it (`mbp-server`,
/// `Jaymins-MacBook-Pro`), else `hostname -s`. `None` when neither answers.
pub fn local_host_name() -> Option<String> {
    let run = |program: &str, args: &[&str]| {
        let out = Command::new(program).args(args).output().ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (out.status.success() && !name.is_empty()).then_some(name)
    };
    run("/usr/sbin/scutil", &["--get", "LocalHostName"]).or_else(|| run("/bin/hostname", &["-s"]))
}
