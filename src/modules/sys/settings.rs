//! `[sys]` and its `[[sys.service]]` checks. Only this Mac's config defines a check; nothing
//! from a request, a peer reply or a card ever becomes one.

use serde::Deserialize;

use super::check::Limits;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Service checks, in display order; none by default.
    pub service: Vec<Service>,
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
}

impl Service {
    pub fn limits(&self) -> Limits {
        Limits { warn: self.warn, fail: self.fail }
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
        if [self.warn, self.fail].into_iter().flatten().any(|n| !n.is_finite()) {
            return bad("warn and fail are numbers");
        }
        Ok(())
    }
}

impl Settings {
    /// Settings with every service valid and named once.
    pub fn check(self) -> Result<Settings, String> {
        for (i, s) in self.service.iter().enumerate() {
            if s.name.trim().is_empty() {
                return Err(format!("[sys]: service {} has no name", i + 1));
            }
            if self.service[..i].iter().any(|o| o.name == s.name) {
                return Err(format!("[sys]: service \"{}\" is named twice", s.name));
            }
            s.check()?;
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests;
