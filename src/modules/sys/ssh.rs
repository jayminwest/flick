//! Reading a machine over ssh: the script sent to `/bin/sh -s` on stdin and the pure parser
//! of what it prints. The script is `probe::PROBE` plus, for each launchd or process check of
//! the machine, one sectioned command; targets are single-quoted, so nothing in config
//! becomes shell syntax there. http and tcp checks run from this Mac instead (`poll.rs`).

use std::fmt::Write as _;

use super::check::{self, Verdict};
use super::probe::PROBE;
use super::run::Exit;
use super::settings::{Kind, Service};

/// The ssh argv for `target`: no prompts (`BatchMode`), a 5 s connect, the script on stdin.
pub fn argv(target: &str) -> Vec<String> {
    let words = ["/usr/bin/ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", target, "/bin/sh", "-s"];
    words.iter().map(|w| (*w).to_string()).collect()
}

/// `word` as one single-quoted shell word.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// Whether the check runs inside the ssh call (else from this Mac).
pub fn remote(service: &Service) -> bool {
    matches!(service.kind, Kind::Launchd | Kind::Process)
}

/// The probe, the remote uid, then each remote check `i` as `@@ svc <i>` (its output, stderr
/// merged) and `@@ rc <i> <exit code>`.
pub fn script(services: &[Service]) -> String {
    let mut s = format!("{PROBE}echo '@@ uid'\n/usr/bin/id -u\n");
    for (i, service) in services.iter().enumerate().filter(|(_, s)| remote(s)) {
        let target = quote(service.word());
        let cmd = match service.kind {
            Kind::Launchd => format!("/bin/launchctl print \"gui/$(/usr/bin/id -u)/\"{target}"),
            _ => format!("/usr/bin/pgrep -x {target}"),
        };
        let _ = writeln!(
            s,
            "out=$({cmd} 2>&1); rc=$?; printf '@@ svc {i}\\n%s\\n@@ rc {i} %s\\n' \"$out\" \"$rc\""
        );
    }
    s
}

/// The verdict of each remote check in `out` (the script's stdout), by service index; `None`
/// for checks that run from this Mac.
pub fn verdicts(out: &str, services: &[Service]) -> Vec<Option<Verdict>> {
    let uid = section(out, "uid").trim().to_string();
    let domain = if uid.is_empty() { "gui/?".to_string() } else { format!("gui/{uid}") };
    services
        .iter()
        .enumerate()
        .map(|(i, service)| {
            if !remote(service) {
                return None;
            }
            let rc = out.lines().find_map(|l| l.strip_prefix(&format!("@@ rc {i} "))?.trim().parse().ok());
            let Some(code) = rc else { return Some(Verdict::unknown("no answer over ssh")) };
            let exit = Ok(Exit { code: Some(code), stdout: section(out, &format!("svc {i}")), stderr: String::new() });
            Some(if service.kind == Kind::Launchd { check::launchd(exit, &domain) } else { check::process(exit) })
        })
        .collect()
}

/// The lines after `@@ <name>` up to the next `@@ ` line, each with its newline.
fn section(out: &str, name: &str) -> String {
    let mut lines = out.lines().skip_while(|l| l.strip_prefix("@@ ").is_none_or(|n| n.trim() != name));
    lines.next();
    lines.take_while(|l| !l.starts_with("@@ ")).fold(String::new(), |mut s, l| {
        s.push_str(l);
        s.push('\n');
        s
    })
}

/// Why an ssh run gave no snapshot, or `None` when it printed one. ssh itself exits 255.
pub fn failure(exit: &Exit) -> Option<String> {
    if exit.code == Some(255) || exit.stdout.trim().is_empty() {
        let why = exit.stderr.lines().map(str::trim).find(|l| !l.is_empty());
        let code = exit.code.map_or("killed by a signal".to_string(), |c| format!("exit {c}"));
        return Some(format!("ssh: {}", why.map_or(code, str::to_string)));
    }
    None
}

#[cfg(test)]
mod tests;
