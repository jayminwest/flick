//! Module `remote`: Flick's control protocol over TCP for named peers in the user's tailnet.
//! `[remote]` sets `peers` (Tailscale names allowed to connect), `port` and `events` (may
//! peers subscribe to `["events"]`). The on/off switch is user state in `remote_state`, off
//! by default: `flick remote on|off` or the root item `remote:network`. The listener runs
//! iff the switch is on and `peers` is not empty.
//!
//! The module never imports the transport: its constructor takes `NetHooks` (the control
//! layer's in production, fakes in tests). It applies the wanted settings on `Started`, on a
//! reload that changes them, on each toggle, and on `Wake` and `LauncherOpened` while on but
//! not listening (Tailscale may come up after Flick). Remote callers may only read status.

mod store;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::config::Section;
use crate::core::control::{DEFAULT_PORT, LastConn, NetHooks, NetSettings, NetStatus};
use crate::core::{Cx, Event, Icon, Item, ItemId, Module, Outcome, unknown_verb};
use store::{MIGRATIONS, Switch};

const ID: &str = "remote";
const USAGE: &str = "remote: usage: remote status | on | off";
const REFUSED: &str = "remote: only `remote status` is allowed from a remote caller";
const NO_PEERS: &str = "no peers: add Tailscale names to [remote] peers";

/// The `[remote]` table.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default)]
struct Settings {
    peers: Vec<String>,
    port: u16,
    events: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { peers: vec![], port: DEFAULT_PORT, events: false }
    }
}

impl Settings {
    fn parse(table: &Section) -> Result<Settings, String> {
        let mut s: Settings = table.get()?;
        if s.port == 0 {
            return Err("[remote] port: must be 1-65535".into());
        }
        s.peers = s.peers.iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
        Ok(s)
    }
}

/// `remote status --json`.
#[derive(Serialize)]
struct Status<'a> {
    on: bool,
    port: u16,
    events: bool,
    peers: &'a [String],
    listening: Vec<std::net::SocketAddr>,
    last: Option<LastConn>,
    error: Option<String>,
}

pub struct Remote {
    hooks: NetHooks,
    now: fn() -> i64,
    settings: Settings,
    /// `Started` arrived: from then on `configure` applies (a reload configures a throwaway
    /// instance too, which must not touch the listener).
    started: bool,
    /// The switch as of `Started` or the last toggle, for `configure`, which has no store.
    on: bool,
    /// What the last apply asked for; `None`: stopped or never started.
    applied: Option<NetSettings>,
    /// The last apply's error, until one succeeds.
    error: Option<String>,
}

impl Remote {
    pub fn new(hooks: NetHooks) -> Remote {
        Remote {
            hooks,
            now: crate::core::store::now,
            settings: Settings::default(),
            started: false,
            on: false,
            applied: None,
            error: None,
        }
    }

    /// What the transport should serve: settings while on with peers, else nothing.
    fn want(&self, on: bool) -> Option<NetSettings> {
        (on && !self.settings.peers.is_empty()).then(|| NetSettings {
            peers: self.settings.peers.clone(),
            port: self.settings.port,
            events: self.settings.events,
        })
    }

    fn apply(&mut self, want: Option<NetSettings>) -> Option<NetStatus> {
        let result = (self.hooks.apply)(want.clone());
        self.applied = want;
        match result {
            Ok(status) => {
                self.error = None;
                Some(status)
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    /// Turn network access on or off and apply it; the status line.
    fn set_on(&mut self, on: bool, cx: &Cx) -> String {
        cx.store.set_network_on(on);
        self.on = on;
        let status = self.apply(self.want(on));
        if !on {
            return "Network access off".into();
        }
        let listening = status.map(|s| s.listening).unwrap_or_default();
        match (&self.error, listening.is_empty()) {
            _ if self.settings.peers.is_empty() => format!("Network access on · {NO_PEERS}"),
            (Some(e), _) => format!("Network access on · not listening: {e}"),
            (None, true) => "Network access on · not listening".into(),
            (None, false) => format!("Network access on · listening on {}", addrs(&listening)),
        }
    }

    fn status(&self, cx: &Cx) -> Result<String, String> {
        let on = cx.store.network_on();
        let net = (self.hooks.status)();
        let error = match &self.error {
            Some(e) => Some(e.clone()),
            None if on && self.settings.peers.is_empty() => Some(NO_PEERS.into()),
            None => net.error,
        };
        let status = Status {
            on,
            port: self.settings.port,
            events: self.settings.events,
            peers: &self.settings.peers,
            listening: net.listening,
            last: net.last,
            error,
        };
        if cx.json {
            return serde_json::to_string(&status).map_err(|e| format!("remote: {e}"));
        }
        Ok(self.status_text(&status))
    }

    fn status_text(&self, s: &Status) -> String {
        let listening =
            if s.listening.is_empty() { "not listening".into() } else { addrs(&s.listening) };
        let peers = if s.peers.is_empty() { "none".into() } else { s.peers.join(", ") };
        let last = s.last.as_ref().map_or_else(|| "none".into(), |c| self.last_text(c));
        let mut text = format!(
            "network access: {}\nlistening: {listening}\npeers: {peers}\nlast connection: {last}",
            if s.on { "on" } else { "off" }
        );
        if let Some(e) = &s.error {
            text = format!("{text}\nerror: {e}");
        }
        text
    }

    fn last_text(&self, c: &LastConn) -> String {
        let name = c.name.as_deref().unwrap_or("unknown");
        let ago = ago((self.now)() - c.at);
        match (&c.reason, c.allowed) {
            (_, true) => format!("{name} ({}) allowed {ago}", c.ip),
            (Some(r), false) => format!("{name} ({}) refused {ago}: {r}", c.ip),
            (None, false) => format!("{name} ({}) refused {ago}", c.ip),
        }
    }
}

/// Stop the listener when the module goes away (a reload with `enabled = false`).
impl Drop for Remote {
    fn drop(&mut self) {
        if self.applied.is_some() {
            let _ = (self.hooks.apply)(None);
        }
    }
}

fn addrs(list: &[std::net::SocketAddr]) -> String {
    list.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
}

fn ago(secs: i64) -> String {
    match secs.max(0) {
        s @ 0..60 => format!("{s}s ago"),
        s @ 60..3600 => format!("{}m ago", s / 60),
        s @ 3600..86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

impl Module for Remote {
    fn id(&self) -> &'static str {
        ID
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    /// A reload that changes what the transport should serve applies it; the throwaway
    /// instance a reload builds never started, so it applies nothing.
    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.settings = Settings::parse(table)?;
        if self.started {
            let want = self.want(self.on);
            if want != self.applied {
                self.apply(want);
            }
        }
        Ok(())
    }

    fn items(&mut self, cx: &mut Cx) -> Vec<Item> {
        let (title, symbol) = if cx.store.network_on() {
            ("Turn Off Network Access", "network.slash")
        } else {
            ("Turn On Network Access", "network")
        };
        vec![Item {
            subtitle: "Remote".into(),
            accessory: "Command".into(),
            keywords: vec!["tailscale remote control peers tailnet".into()],
            ..Item::new(ItemId::new(ID, "network"), title, "Run Command", Icon::Symbol(symbol))
        }]
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        match id.key() {
            "network" => Outcome::Stay(Some(self.set_on(!cx.store.network_on(), cx))),
            _ => Outcome::Stay(None),
        }
    }

    fn on_event(&mut self, event: Event, cx: &mut Cx) -> bool {
        match event {
            Event::Started => {
                self.started = true;
                self.on = cx.store.network_on();
                let want = self.want(self.on);
                self.apply(want);
            }
            Event::Wake | Event::LauncherOpened if self.started => {
                let want = self.want(cx.store.network_on());
                if want.is_some() && (self.hooks.status)().listening.is_empty() {
                    self.apply(want);
                }
            }
            _ => {}
        }
        false
    }

    fn verbs(&self) -> &'static str {
        "remote status|on|off"
    }

    /// `status` answers with JSON under `--json`. Remote callers reach only `status`.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v] if v == "status" => self.status(cx),
            [v] if (v == "on" || v == "off") && cx.remote => Err(REFUSED.into()),
            [v] if v == "on" => Ok(self.set_on(true, cx)),
            [v] if v == "off" => Ok(self.set_on(false, cx)),
            [v, ..] if ["status", "on", "off"].contains(&v.as_str()) => Err(USAGE.into()),
            _ => Err(unknown_verb(ID, args)),
        }
    }
}
