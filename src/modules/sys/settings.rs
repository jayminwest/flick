//! `[sys]`, its `[[sys.service]]` checks and its `[[sys.machine]]` fleet. Only this Mac's
//! config defines a check, a machine or an ssh target; nothing from a request, a peer reply
//! or a card ever becomes one.

use serde::Deserialize;

use super::check::Limits;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Service checks, in display order; none by default.
    pub service: Vec<Service>,
    /// Machines of the fleet view, in display order; none by default (no fleet polling).
    pub machine: Vec<Machine>,
    /// Background fleet refresh in seconds: 0 (the default) polls only when the launcher or
    /// the fleet view opens, on wake and every 15 s while the view shows; else at least 30.
    pub refresh_secs: u64,
    /// Shows the fleet window, or hides it when it has the keyboard; unset by default.
    pub hotkey: Option<String>,
}

/// The least non-zero `refresh_secs`.
pub const MIN_REFRESH: u64 = 30;

/// How the fleet reads a machine.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    /// This Mac: its own cached snapshot and services.
    Local,
    /// The machine's Flick, `sys snapshot --json` over the network transport.
    Flick,
    /// The probe over `ssh <target> /bin/sh -s`; no Flick needed there.
    Ssh,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Local => "local",
            Via::Flick => "flick",
            Via::Ssh => "ssh",
        }
    }
}

/// One `[[sys.machine]]`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub name: String,
    pub via: Via,
    /// `name[:port]` of its Flick (via = flick); default: `name`.
    #[serde(default)]
    pub host: Option<String>,
    /// An ssh destination (`user@host`): via = ssh, and the fallback when its Flick is too
    /// old or unreachable.
    #[serde(default)]
    pub ssh: Option<String>,
    /// A `vnc://` URL for Screen Sharing (flick-4a4c).
    #[serde(default)]
    pub vnc: Option<String>,
    /// An http(s) dashboard URL (flick-4a4c).
    #[serde(default)]
    pub dash: Option<String>,
    /// Checks of a machine read over ssh: launchd and process run inside the ssh call,
    /// http and tcp from this Mac. Needs `ssh`. A Flick machine checks its own
    /// `[[sys.service]]` and reports them; listed here too, a service is checked when its
    /// Flick falls back to ssh, and its `log` and `restart` let this Mac tail and restart it
    /// over ssh (flick-4a4c).
    #[serde(default)]
    pub service: Vec<Service>,
}

impl Machine {
    /// Where its Flick listens: `host`, else its name.
    pub fn flick_host(&self) -> &str {
        self.host.as_deref().unwrap_or(&self.name)
    }

    fn check(&self) -> Result<(), String> {
        let name = &self.name;
        let bad = |why: &str| Err(format!("[sys]: machine \"{name}\": {why}"));
        let odd = |w: &str| w.is_empty() || w.starts_with('-') || w.contains(char::is_whitespace);
        if word(self.host.as_ref()).is_some_and(odd) {
            return bad("host is one word, name[:port]");
        }
        if word(self.ssh.as_ref()).is_some_and(odd) {
            return bad("ssh is one word, e.g. \"user@host\", not an option");
        }
        if self.via == Via::Ssh && self.ssh.is_none() {
            return bad("via = \"ssh\" needs ssh = \"user@host\"");
        }
        if word(self.vnc.as_ref()).is_some_and(|u| !u.starts_with("vnc://")) {
            return bad("vnc must start with vnc://");
        }
        if word(self.dash.as_ref()).is_some_and(|u| !(u.starts_with("http://") || u.starts_with("https://"))) {
            return bad("dash must start with http:// or https://");
        }
        if !self.service.is_empty() && (self.via == Via::Local || self.ssh.is_none()) {
            return bad("services belong to machines with an ssh target; this Mac's are [[sys.service]]");
        }
        for s in &self.service {
            if s.kind == Kind::Command {
                return bad("a command check runs on the machine's own Flick, not over ssh");
            }
            s.check().map_err(|e| format!("[sys]: machine \"{name}\": {}", e.trim_start_matches("[sys]: ")))?;
        }
        unique(self.service.iter().map(|s| s.name.as_str()), &format!("machine \"{name}\": service"))
    }
}

/// An optional setting, trimmed.
fn word(v: Option<&String>) -> Option<&str> {
    v.map(|w| w.trim())
}

/// Every name in `names` is not blank and appears once.
fn unique<'a>(names: impl Iterator<Item = &'a str>, what: &str) -> Result<(), String> {
    let mut seen: Vec<&str> = vec![];
    for (i, n) in names.enumerate() {
        if n.trim().is_empty() {
            return Err(format!("[sys]: {what} {} has no name", i + 1));
        }
        if seen.contains(&n) {
            return Err(format!("[sys]: {what} \"{n}\" is named twice"));
        }
        seen.push(n);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Http,
    Tcp,
    Launchd,
    Process,
    Command,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Http => "http",
            Kind::Tcp => "tcp",
            Kind::Launchd => "launchd",
            Kind::Process => "process",
            Kind::Command => "command",
        }
    }
}

/// A URL, `host:port`, launchd label or process name; an argv for `command`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Target {
    One(String),
    Argv(Vec<String>),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub name: String,
    pub kind: Kind,
    pub target: Target,
    #[serde(default)]
    pub warn: Option<f64>,
    #[serde(default)]
    pub fail: Option<f64>,
    /// A log file the dashboard can tail (flick-4a4c).
    #[serde(default)]
    pub log: Option<String>,
    /// The dashboard may restart it after a confirm (launchd only; flick-4a4c).
    #[serde(default)]
    pub restart: bool,
    /// The launchd domain: `gui` (the user's agents, the default) or `system` (daemons;
    /// restart, and a tail of an unreadable log, go through `sudo -n`, flick-1356).
    #[serde(default)]
    pub domain: Domain,
}

/// A launchd service's domain.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Domain {
    /// `gui/<uid>`: launch agents of the logged-in user.
    #[default]
    Gui,
    /// `system`: launch daemons, run as root.
    System,
}

impl Service {
    pub fn limits(&self) -> Limits {
        Limits { warn: self.warn, fail: self.fail }
    }

    /// The launchd domain of the label for `launchctl`: `gui/<uid>` or `system`. `uid`
    /// is this user's here; over ssh, a shell expression for the remote user's.
    pub fn launchd_domain(&self, uid: &str) -> String {
        match self.domain {
            Domain::Gui => format!("gui/{uid}"),
            Domain::System => "system".into(),
        }
    }

    /// The target's one word (`""` for an argv).
    pub fn word(&self) -> &str {
        match &self.target {
            Target::One(s) => s,
            Target::Argv(_) => "",
        }
    }

    /// The target as an argv: a command's own, else the one word.
    pub fn argv(&self) -> &[String] {
        match &self.target {
            Target::One(s) => std::slice::from_ref(s),
            Target::Argv(argv) => argv,
        }
    }

    /// What reports show as the target. A command shows only its program, so arguments
    /// (which may hold a token) never leave this Mac in a reply.
    pub fn shown_target(&self) -> String {
        match &self.target {
            Target::One(s) => s.clone(),
            Target::Argv(argv) => argv.first().cloned().unwrap_or_default(),
        }
    }

    fn check(&self) -> Result<(), String> {
        let name = &self.name;
        let bad = |why: &str| Err(format!("[sys]: service \"{name}\": {why}"));
        let word = self.word().trim();
        match (self.kind, &self.target) {
            (Kind::Command, Target::Argv(argv)) if argv.first().is_some_and(|p| !p.trim().is_empty()) => {}
            (Kind::Command, _) => return bad("command target is an argv, e.g. [\"/bin/echo\", \"1\"]"),
            (_, Target::Argv(_)) => return bad("target is a string for this kind"),
            (_, Target::One(_)) if word.is_empty() => return bad("target is empty"),
            (Kind::Http, _) if !(word.starts_with("http://") || word.starts_with("https://")) => {
                return bad("http target must start with http:// or https://");
            }
            (Kind::Tcp, _) if !word.rsplit_once(':').is_some_and(|(h, p)| !h.is_empty() && p.parse::<u16>().is_ok()) => {
                return bad("tcp target is host:port");
            }
            (Kind::Launchd | Kind::Process, _) if word.contains('/') || word.contains(char::is_whitespace) => {
                return bad("target is one word without / or spaces");
            }
            _ => {}
        }
        if matches!(self.kind, Kind::Launchd | Kind::Process) && (self.warn.is_some() || self.fail.is_some()) {
            return bad("warn and fail apply to http, tcp and command");
        }
        if self.restart && self.kind != Kind::Launchd {
            return bad("restart = true needs kind = \"launchd\"");
        }
        if self.domain == Domain::System && self.kind != Kind::Launchd {
            return bad("domain applies to kind = \"launchd\"");
        }
        if [self.warn, self.fail].into_iter().flatten().any(|n| !n.is_finite()) {
            return bad("warn and fail are numbers");
        }
        Ok(())
    }
}

impl Settings {
    /// Settings with every service and machine valid and named once.
    pub fn check(self) -> Result<Settings, String> {
        unique(self.service.iter().map(|s| s.name.as_str()), "service")?;
        for s in &self.service {
            s.check()?;
        }
        unique(self.machine.iter().map(|m| m.name.as_str()), "machine")?;
        for m in &self.machine {
            m.check()?;
        }
        if self.refresh_secs != 0 && self.refresh_secs < MIN_REFRESH {
            return Err(format!("[sys]: refresh_secs is 0 (off) or at least {MIN_REFRESH}"));
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests;
