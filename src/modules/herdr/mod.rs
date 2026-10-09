//! Module `herdr`: coding agents in herdr on this Mac and on herdr's saved SSH machines,
//! the ones waiting on me first, and a jump to an agent's pane (flick-3cc9). Read-only:
//! Flick lists, reads and focuses; it never sends input to an agent.
//!
//! Ids are `herdr:<key>` (`views.rs`): the root item `herdr:agents` (the only id that
//! reaches the usage table); in views `herdr:agent/<machine>/<pane id>`,
//! `herdr:machine/<machine>` and `herdr:line/<n>`. Views: `agents` (every agent, Enter
//! jumps, ⌘K Show Output) and `agent` (Jump plus the agent's last output lines, held in
//! memory only).
//!
//! Table `[herdr]`: `herdr` (the CLI; default `herdr` on `PATH`, then
//! /opt/homebrew/bin), `socket` (default `~/.config/herdr/herdr.sock`), `machines`
//! (default empty: `local` plus every enabled `herdr machine list` profile; `"local"` names
//! this Mac's server), `remote_refresh_secs` (default 0: remote machines refresh only
//! while the launcher is open; at least 15 to poll in the background too), `terminal`
//! (the app brought to the front on a jump; default `WezTerm`), `hotkey` (opens the agents
//! view) and `preview_lines` (default 6).
//!
//! All herdr I/O runs on the threads in `io.rs`; they post `ModuleChanged`. The local
//! server streams events; remote machines are polled every `VISIBLE_EVERY` while the
//! launcher is open, on `LauncherOpened`, and on `remote_refresh_secs` when set.

mod cli;
mod io;
pub mod local;
pub mod model;
pub mod remote;
#[cfg(test)]
mod testkit;
mod views;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::config::Section;
use crate::core::{Action, Binding, Cx, Event, Icon, ItemId, ListView, Module, Outcome, unknown_verb};
use crate::platform::{events, panel, workspace};
use io::{Hooks, LOCAL, Shared, Transport, lock};
use local::Local;
use remote::Remote;
use views::{ID, Key};

/// While the launcher is open, a remote machine is read again after this long.
const VISIBLE_EVERY: u64 = 15;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    herdr: String,
    socket: String,
    machines: Vec<String>,
    remote_refresh_secs: u64,
    terminal: String,
    hotkey: Option<String>,
    preview_lines: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            herdr: "herdr".into(),
            socket: "~/.config/herdr/herdr.sock".into(),
            machines: vec![],
            remote_refresh_secs: 0,
            terminal: "WezTerm".into(),
            hotkey: None,
            preview_lines: 6,
        }
    }
}

impl Settings {
    fn check(self) -> Result<Settings, String> {
        if !(1..=io::READ_LINES as usize).contains(&self.preview_lines) {
            return Err(format!("[herdr]: preview_lines must be 1 to {}", io::READ_LINES));
        }
        if (1..VISIBLE_EVERY).contains(&self.remote_refresh_secs) {
            return Err(format!("[herdr]: remote_refresh_secs must be 0 or at least {VISIBLE_EVERY}"));
        }
        if let Some(bad) = self.machines.iter().find(|m| m.trim().is_empty() || m.contains('/')) {
            return Err(format!("[herdr]: bad machine name \"{bad}\""));
        }
        Ok(self)
    }

    fn transport(&self) -> Transport {
        Transport { local: Local::new(local::expand(&self.socket)), remote: Remote::new(remote::resolve(&self.herdr)) }
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

const HOOKS: Hooks = Hooks {
    post: || events::post(Event::ModuleChanged { module: ID }),
    now: unix_now,
    visible: panel::is_visible,
    front: |name| {
        events::on_main(move || match workspace::find_app(&name) {
            Some(app) => workspace::open_file(&app),
            None => eprintln!("flick: herdr: no app \"{name}\" to bring to the front"),
        });
    },
};

pub struct Herdr {
    settings: Settings,
    transport: Transport,
    shared: Arc<Shared>,
    hooks: Hooks,
    /// `Started` arrived: threads may run. Before it (and in a throwaway instance that a
    /// reload only configures) nothing starts.
    started: bool,
    /// The agent view `agent` shows: (machine, pane id).
    detail: Option<(String, String)>,
}

impl Default for Herdr {
    fn default() -> Self {
        Herdr::with_hooks(HOOKS)
    }
}

impl Drop for Herdr {
    fn drop(&mut self) {
        io::stop_local(&self.shared);
        io::stop_timer(&self.shared);
    }
}

impl Herdr {
    fn with_hooks(hooks: Hooks) -> Herdr {
        let settings = Settings::default();
        Herdr {
            transport: settings.transport(),
            settings,
            shared: Arc::default(),
            hooks,
            started: false,
            detail: None,
        }
    }

    fn now(&self) -> u64 {
        (self.hooks.now)()
    }

    /// Set the machines and start or stop threads for `self.settings`. `old`: the settings
    /// before a reload, `None` at `Started`.
    fn apply(&mut self, old: Option<&Settings>) {
        let explicit = !self.settings.machines.is_empty();
        if explicit {
            self.shared.set_machines(&self.settings.machines);
        } else {
            if old.is_none_or(|o| !o.machines.is_empty()) {
                self.shared.set_machines(&[LOCAL.to_string()]);
            }
            io::discover(&self.shared, &self.transport.remote, self.hooks);
        }
        if old.is_some_and(|o| o.socket != self.settings.socket) {
            io::stop_local(&self.shared);
        }
        self.ensure_local();
        io::stop_timer(&self.shared);
        if self.settings.remote_refresh_secs > 0 {
            let every = Duration::from_secs(self.settings.remote_refresh_secs);
            io::start_timer(&self.shared, every, self.hooks);
        }
    }

    /// Connect to the local server unless connected, when the fleet has it.
    fn ensure_local(&self) {
        if lock(&self.shared.fleet).machine(LOCAL).is_some() {
            io::start_local(&self.shared, self.transport.local.clone(), self.hooks);
        } else {
            io::stop_local(&self.shared);
        }
    }

    /// Read due remote machines: every `VISIBLE_EVERY` while the launcher is open (and post
    /// once more after that, to keep going), else on the opt-in background interval.
    fn poll(&self, visible: bool) {
        let bg = self.settings.remote_refresh_secs;
        let (min_age, again) = if visible {
            (VISIBLE_EVERY, Some(Duration::from_secs(VISIBLE_EVERY)))
        } else if bg > 0 {
            // A tick may land a little before a full interval since the last read ended.
            (bg * 3 / 4, None)
        } else {
            return;
        };
        io::poll_remote(&self.shared, &self.transport.remote, min_age, again, self.hooks);
    }

    fn jump(&self, machine: &str, pane_id: &str, cx: &mut Cx) -> Outcome {
        cx.hide();
        io::jump(&self.shared, &self.transport, machine, pane_id, &self.settings.terminal, self.hooks);
        Outcome::Hide
    }

    fn show_output(&mut self, machine: &str, pane_id: &str) -> Outcome {
        self.detail = Some((machine.to_string(), pane_id.to_string()));
        Outcome::Push(ListView::new(ID, "agent"))
    }

    fn command_ls(&self, args: &[String], json: bool) -> Result<String, String> {
        if !args.is_empty() {
            return Err("herdr: usage: herdr ls".into());
        }
        if self.started {
            self.ensure_local();
            self.poll(true);
        }
        let fleet = lock(&self.shared.fleet);
        Ok(if json { cli::ls_json(&fleet).to_string() } else { cli::ls_text(&fleet, self.now()) })
    }

    fn command_jump(&self, spec: &str) -> Result<String, String> {
        let (machine, pane_id) = {
            let fleet = lock(&self.shared.fleet);
            let agent = cli::resolve(&fleet, spec)?;
            (agent.machine.clone(), agent.pane_id.clone())
        };
        let (shared, t, terminal) = (&self.shared, &self.transport, &self.settings.terminal);
        io::jump(shared, t, &machine, &pane_id, terminal, self.hooks);
        Ok(format!("jumping to {machine}/{pane_id}"))
    }
}

impl Module for Herdr {
    fn id(&self) -> &'static str {
        ID
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let settings = table.get::<Settings>()?.check()?;
        let old = std::mem::replace(&mut self.settings, settings);
        self.transport = self.settings.transport();
        if self.started && old != self.settings {
            self.apply(Some(&old));
        }
        Ok(())
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<crate::core::Item> {
        vec![views::root_item(&lock(&self.shared.fleet))]
    }

    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        match view {
            "agents" => {
                if self.started {
                    self.ensure_local();
                    self.poll(true);
                }
                Some(ListView {
                    placeholder: "Search agents…".into(),
                    empty: "No agents".into(),
                    ..ListView::new(ID, view)
                })
            }
            "agent" => {
                let (machine, pane_id) = self.detail.clone()?;
                let lines = self.settings.preview_lines;
                io::fetch_preview(&self.shared, &self.transport, &machine, &pane_id, lines, self.hooks);
                Some(ListView { placeholder: "Agent output".into(), ..ListView::new(ID, view) })
            }
            _ => None,
        }
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        let now = self.now();
        let fleet = lock(&self.shared.fleet);
        let items = if view.name == "agent" {
            let agent = self.detail.as_ref().and_then(|(m, p)| fleet.agent(m, p));
            view.footer = agent.map_or("Agent gone".into(), |a| format!("{} · {}  ·  esc to go back", a.label(), a.machine));
            views::detail_items(agent, lock(&self.shared.preview).as_ref(), now)
        } else {
            view.footer = format!("{}  ·  ⌘K Show Output  ·  esc to go back", views::summary_line(&fleet));
            views::agents_items(&fleet, now)
        };
        drop(fleet);
        // The bonus keeps fleet order (waiting first) for equal scores.
        let order: HashMap<String, usize> =
            items.iter().enumerate().map(|(i, item)| (item.id.to_string(), i)).collect();
        view.items = cx.ranker.rank(cx.query, items, |i| -(order[i.id.as_str()] as f64) * 1e-3);
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        match Key::parse(id.key()) {
            Some(Key::Agents) => Outcome::Push(ListView::new(ID, "agents")),
            Some(Key::Agent { machine, pane_id }) => self.jump(machine, pane_id, cx),
            Some(Key::Machine(m)) => {
                let fleet = lock(&self.shared.fleet);
                let note = fleet.machine(m).and_then(|s| views::machine_note(s, self.now()));
                Outcome::Stay(note.map(|n| format!("{m}: {n}")))
            }
            Some(Key::Line) => match self.detail.clone() {
                Some((machine, pane_id)) => self.jump(&machine, &pane_id, cx),
                None => Outcome::Stay(None),
            },
            None => Outcome::Stay(None),
        }
    }

    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        match Key::parse(id.key()) {
            Some(Key::Agent { .. }) => vec![
                Action::new("jump", "Jump to Agent", Icon::Symbol("arrow.up.forward.app")),
                Action::new("output", "Show Output", Icon::Symbol("text.alignleft")),
            ],
            _ => vec![],
        }
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        match (Key::parse(id.key()), key) {
            (Some(Key::Agent { machine, pane_id }), "jump") => self.jump(machine, pane_id, cx),
            (Some(Key::Agent { machine, pane_id }), "output") => self.show_output(machine, pane_id),
            _ => Outcome::Stay(None),
        }
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        match event {
            Event::Started => {
                self.started = true;
                self.apply(None);
            }
            Event::LauncherOpened if self.started => {
                self.ensure_local();
                self.poll(true);
            }
            Event::Wake if self.started => {
                self.ensure_local();
                self.poll(false);
            }
            Event::ModuleChanged { module: ID } if self.started => self.poll((self.hooks.visible)()),
            _ => {}
        }
        false
    }

    fn hotkeys(&self) -> Vec<Binding> {
        let spec = self.settings.hotkey.iter().filter(|s| !s.trim().is_empty());
        spec.map(|spec| Binding { spec: spec.clone(), key: Ok("agents".into()) }).collect()
    }

    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        (key == "agents").then(|| ListView::new(ID, "agents"))
    }

    /// `--json` (`cx.json`) makes `ls` and `status` answer with JSON.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v, rest @ ..] if v == "ls" => self.command_ls(rest, cx.json),
            [v, spec] if v == "jump" => self.command_jump(spec),
            [v] if v == "jump" => Err("herdr: usage: herdr jump <machine>/<pane id or name>".into()),
            [v] if v == "status" => {
                let (fleet, info) = (lock(&self.shared.fleet), lock(&self.shared.info));
                let paths = cli::Paths {
                    socket: self.transport.local.socket(),
                    herdr: self.transport.remote.herdr(),
                };
                Ok(if cx.json {
                    cli::status_json(&fleet, &info, &paths).to_string()
                } else {
                    cli::status_text(&fleet, &info, &paths, self.now())
                })
            }
            _ => Err(unknown_verb(ID, args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "herdr ls | herdr jump <machine>/<pane id or name> | herdr status"
    }
}

#[cfg(test)]
mod tests;
