//! Fleet actions (flick-4a4c), pure: which actions a row offers, what a tail or a restart
//! runs and where, the restart's confirm, and what their output means. No locks, no I/O.
//!
//! A tail or a restart acts on a service that this Mac's own config defines: its
//! `[[sys.service]]` (run here) or a `[[sys.machine.service]]` of a machine with an `ssh`
//! target (run over ssh). A service that only a peer's snapshot names offers neither: a
//! label, a path or a command never comes from a peer reply. Restart needs `restart = true`
//! (launchd only, `settings.rs`) and a destructive confirm showing the exact command; tail
//! needs `log`. The only commands are `launchctl kickstart -k gui/<uid>/<label>` and
//! `tail -n TAIL_LINES -- <log>`, with the label and path quoted as one shell word over ssh.
//! A `domain = "system"` daemon (flick-1356) restarts with
//! `sudo -n launchctl kickstart -k system/<label>`, and its tail runs again through
//! `sudo -n` when the plain one is denied. `sudo -n` never prompts: when it would need a
//! password the result says `SUDO_NEEDS_PASSWORD`.

use super::ID;
use super::fleet::Fleet;
use super::io::Entry;
use super::run::Exit;
use super::settings::{Domain, Machine, Service, Via};
use super::ssh;
use crate::core::{Action, Confirm, ConfirmRow, Icon};

/// The prefix of a `system` daemon's commands here: never prompts (`-n`).
const SUDO: [&str; 2] = ["/usr/bin/sudo", "-n"];

/// Lines a tail reads.
pub const TAIL_LINES: u32 = 100;

/// What `sys tail` and `sys restart` call this Mac when no machine is named.
pub const HERE: &str = "this Mac";

/// Where a service's commands run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Local,
    /// An ssh destination from config.
    Ssh(String),
}

/// A service that this Mac's config defines, and where it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// The machine's name, or `HERE`.
    pub machine: String,
    pub service: Service,
    pub place: Place,
}

/// A command to run: its argv, its stdin (the script, over ssh) and how a confirm or the
/// CLI shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub argv: Vec<String>,
    pub input: Option<String>,
    pub shown: String,
}

/// The service `service` of `machine` (none: this Mac's `[[sys.service]]`). A via = local
/// machine means this Mac; another needs the service in its `[[sys.machine.service]]` and
/// an ssh target.
pub fn resolve(fleet: &Fleet, mine: &[Entry], machine: Option<&str>, service: &str) -> Result<Target, String> {
    let slot = match machine {
        None => None,
        Some(m) => Some(&fleet.slots.iter().find(|s| s.machine.name == m).ok_or(format!("sys: no machine \"{m}\""))?.machine),
    };
    let name = slot.map_or(HERE, |m| m.name.as_str()).to_string();
    let missing = || format!("sys: no service \"{service}\" on {name} in this Mac's config");
    match slot {
        None | Some(Machine { via: Via::Local, .. }) => {
            let entry = mine.iter().find(|e| e.service.name == service).ok_or_else(missing)?;
            Ok(Target { machine: name.clone(), service: entry.service.clone(), place: Place::Local })
        }
        Some(m) => {
            let svc = m.service.iter().find(|s| s.name == service).ok_or_else(missing)?;
            let target = m.ssh.clone().ok_or(format!("sys: {name} has no ssh target"))?;
            Ok(Target { machine: name.clone(), service: svc.clone(), place: Place::Ssh(target) })
        }
    }
}

/// `word` as one single-quoted shell word.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

fn words(w: &[&str]) -> Vec<String> {
    w.iter().map(|w| (*w).to_string()).collect()
}

impl Target {
    /// `ollama on mac-pro`.
    pub fn what(&self) -> String {
        format!("{} on {}", self.service.name, self.machine)
    }

    /// The restart: `launchctl kickstart -k gui/<uid>/<label>`, here with `uid` (this
    /// user's), over ssh with the remote user's. A `system` daemon's is
    /// `sudo -n launchctl kickstart -k system/<label>`: it never asks for a password.
    pub fn restart(&self, uid: Option<u32>) -> Result<Run, String> {
        if !self.service.restart {
            return Err(format!("sys: {} may not be restarted (set restart = true on a launchd service)", self.what()));
        }
        let label = self.service.word();
        let system = self.service.domain == Domain::System;
        let sudo = if system { "sudo -n " } else { "" };
        match &self.place {
            Place::Local => {
                let domain = match uid {
                    _ if system => self.service.launchd_domain(""),
                    Some(uid) => self.service.launchd_domain(&uid.to_string()),
                    None => return Err("sys: no uid for the launchd domain".into()),
                };
                let target = format!("{domain}/{label}");
                let shown = format!("{sudo}launchctl kickstart -k {target}");
                let kick = ["/bin/launchctl", "kickstart", "-k", &target];
                let argv = if system { words(&[&SUDO[..], &kick].concat()) } else { words(&kick) };
                Ok(Run { argv, input: None, shown })
            }
            Place::Ssh(dest) => {
                let domain = self.service.launchd_domain("$(/usr/bin/id -u)");
                let prefix = if system { "/usr/bin/sudo -n " } else { "" };
                let script = format!("exec {prefix}/bin/launchctl kickstart -k \"{domain}/\"{}\n", quote(label));
                let domain = self.service.launchd_domain("$(id -u)");
                let shown = format!("ssh {dest} {sudo}launchctl kickstart -k {domain}/{label}");
                Ok(Run { argv: ssh::argv(dest), input: Some(script), shown })
            }
        }
    }

    /// The tail: `tail -n TAIL_LINES -- <log>`; a `~/` path is under `home` here, under the
    /// remote `$HOME` over ssh.
    pub fn tail(&self, home: &str) -> Result<Run, String> {
        self.tail_as(home, false)
    }

    /// A `system` daemon's tail again through `sudo -n`, for a log only root may read
    /// (`jobs::tail` runs it after the plain tail is denied). `None` for other services.
    pub fn sudo_tail(&self, home: &str) -> Option<Run> {
        (self.service.domain == Domain::System).then(|| self.tail_as(home, true).ok()).flatten()
    }

    fn tail_as(&self, home: &str, sudo: bool) -> Result<Run, String> {
        let (shown_sudo, sudo_path) = if sudo { ("sudo -n ", "/usr/bin/sudo -n ") } else { ("", "") };
        let log = self.service.log.as_deref().map(str::trim).unwrap_or_default();
        if log.is_empty() {
            return Err(format!("sys: {} has no log (set log = \"<path>\")", self.what()));
        }
        let rest = log.strip_prefix("~/");
        if rest.is_none() && !log.starts_with('/') {
            return Err(format!("sys: the log of {} is a path from / or ~/", self.what()));
        }
        let n = TAIL_LINES.to_string();
        match &self.place {
            Place::Local => {
                let path = rest.map_or_else(|| log.to_string(), |r| format!("{}/{r}", home.trim_end_matches('/')));
                let shown = format!("{shown_sudo}tail -n {n} {path}");
                let tail = ["/usr/bin/tail", "-n", &n, "--", &path];
                let argv = if sudo { words(&[&SUDO[..], &tail].concat()) } else { words(&tail) };
                Ok(Run { argv, input: None, shown })
            }
            Place::Ssh(dest) => {
                let path = rest.map_or_else(|| quote(log), |r| format!("\"$HOME\"/{}", quote(r)));
                let script = format!("exec {sudo_path}/usr/bin/tail -n {n} -- {path}\n");
                let shown = format!("ssh {dest} {shown_sudo}tail -n {n} {log}");
                Ok(Run { argv: ssh::argv(dest), input: Some(script), shown })
            }
        }
    }

    /// The restart's destructive confirm: the exact command and where it runs.
    pub fn confirm(&self, run: &Run) -> Confirm {
        let place = match &self.place {
            Place::Local => "runs on this Mac".to_string(),
            Place::Ssh(dest) => format!("runs over ssh as {dest}"),
        };
        let accessory = match self.service.domain {
            Domain::Gui => "launchd",
            Domain::System => "launchd system · sudo -n",
        };
        let row = ConfirmRow { subtitle: place, accessory: accessory.into(), ..ConfirmRow::new(&run.shown) };
        Confirm {
            rows: vec![row],
            label: format!("Restart {}", self.service.name),
            destructive: true,
            ..Confirm::new(ID, token(&self.machine, &self.service.name), format!("Restart {}?", self.what()))
        }
    }
}

/// The confirm's token: machine and service, re-resolved from config on confirm.
fn token(machine: &str, service: &str) -> String {
    format!("restart\t{machine}\t{service}")
}

/// The machine and service of a restart token.
pub fn parse_token(token: &str) -> Option<(&str, &str)> {
    let mut parts = token.strip_prefix("restart\t")?.splitn(2, '\t');
    Some((parts.next()?, parts.next()?))
}

/// What `sudo -n` says when it would have to ask for a password.
const SUDO_ASKS: [&str; 2] = ["a password is required", "a terminal is required"];

/// Why a `sudo -n` command did not run.
pub const SUDO_NEEDS_PASSWORD: &str = "sudo needs a password here; Flick never types one. Allow this command without a password (a NOPASSWD rule in sudoers), or run it in Terminal";

/// The last non-empty stderr line (launchctl says "Bad request." before the reason), else
/// the exit code. `sudo -n` asking for a password says so plainly.
fn why(exit: &Exit) -> String {
    if exit.stderr.lines().any(|l| l.starts_with("sudo:") && SUDO_ASKS.iter().any(|a| l.contains(a))) {
        return SUDO_NEEDS_PASSWORD.into();
    }
    let line = exit.stderr.lines().map(str::trim).rfind(|l| !l.is_empty());
    line.map_or_else(|| exit.code.map_or("killed by a signal".into(), |c| format!("exit {c}")), str::to_string)
}

/// What a restart's run means.
pub fn restarted(got: Result<Exit, String>, what: &str) -> Result<String, String> {
    match got {
        Ok(exit) if exit.code == Some(0) => Ok(format!("Restarted {what}")),
        Ok(exit) => Err(format!("Restart {what} failed: {}", why(&exit))),
        Err(e) => Err(format!("Restart {what} failed: {e}")),
    }
}

/// What a tail's run means: the lines, or why there are none.
pub fn tailed(got: Result<Exit, String>) -> Result<String, String> {
    match got {
        Ok(exit) if exit.code == Some(0) => Ok(exit.stdout),
        Ok(exit) => Err(why(&exit)),
        Err(e) => Err(e),
    }
}

/// The action menu of a machine row: Screen Sharing (`vnc`), Open Dash (`dash`).
pub fn machine_menu(m: &Machine) -> Vec<Action> {
    let vnc = m.vnc.as_ref().map(|_| Action::new("vnc", "Screen Sharing", Icon::Symbol("display")));
    let dash = m.dash.as_ref().map(|_| Action::new("dash", "Open Dash", Icon::Symbol("safari")));
    vnc.into_iter().chain(dash).collect()
}

/// The action menu of a service row: Tail Log with a `log`, Restart with `restart = true`.
pub fn service_menu(t: &Target) -> Vec<Action> {
    let tail = t.service.log.as_ref().map(|_| Action::new("tail", "Tail Log", Icon::Symbol("doc.text.magnifyingglass")));
    let restart = t.service.restart.then(|| Action::new("restart", "Restart…", Icon::Symbol("arrow.clockwise")));
    tail.into_iter().chain(restart).collect()
}

#[cfg(test)]
mod tests;
