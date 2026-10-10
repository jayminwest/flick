//! Module `sys`: this Mac's health and the services it checks, for the fleet dashboard
//! (plan flick-b5d0). Steps so far: the probe and a cached `sys snapshot` (flick-bd74),
//! `[[sys.service]]` checks and `sys services` (flick-4573), the `[[sys.machine]]` fleet and
//! `sys fleet` (flick-3608), the launcher views (flick-9eb1), the fleet actions (flick-4a4c).
//!
//! Launcher (`views.rs`): root item `sys:fleet` "Fleet", only while `[[sys.machine]]`
//! tables exist (with none it is hidden and nothing polls). It opens view `fleet`: a row
//! per machine (status icon, metrics, age) and per service of each, then Herdr Agents,
//! which pushes herdr's `agents` view by name. Enter on a machine or a service opens view
//! `machine`: its state, each probe fact, its services.
//!
//! Actions (cmd+K, `ops.rs`, `act.rs`): a machine offers Screen Sharing (its `vnc` URL) and
//! Open Dash (its `dash`). A service that this Mac's config defines (its `[[sys.service]]`
//! under the `via = "local"` machine, or a `[[sys.machine.service]]` of a machine with
//! `ssh`) offers Tail Log with a `log` (view `log`, the last `act::TAIL_LINES` lines as its
//! text) and Restart with `restart = true`: a destructive confirm (cmd+Enter) showing the
//! exact `launchctl kickstart -k` command, then it runs here or over ssh, never through a
//! peer's Flick. `flick sys tail [<machine>] <service>` prints the lines;
//! `flick sys restart [<machine>] <service>` prints the command and restarts only with
//! `--yes`. Both are in `NET_DENIED` (they run code and read files).
//!
//! `flick sys snapshot [--json]`: CPU load, memory, disks, battery, thermal and uptime
//! from one run of `probe::PROBE` (stock tools, no FFI), plus the services' last verdicts.
//! `flick sys services [--json]`: one line per `[[sys.service]]` with ok, warn, fail or
//! unknown and a reason. Both answer from the cache and start a refresh on a thread
//! (`io.rs`); the first call with an empty cache waits for it, at most `FIRST_WAIT`. The
//! JSON (`report.rs`) is what a peer's fleet view reads over the network: both verbs are
//! read-only and outside `NET_DENIED`.
//!
//! `flick sys fleet [--json]`: every `[[sys.machine]]` (`fleet.rs`): this Mac's own cache
//! (via = local), a peer Flick's `sys snapshot --json` through `PeerHooks` (via = flick;
//! `flick too old` or unreachable falls back to ssh when the machine has a target), or the
//! probe over ssh (via = ssh, `ssh.rs`). Each with its age; old or failed data is stale and
//! keeps the last snapshot. Read-only, outside `NET_DENIED`; a remote caller gets the cache
//! and starts no round, so a peer never makes this Mac ssh.
//!
//! Table `[sys]`: `service` (array of tables; none by default): `name`, `kind` (http, tcp,
//! launchd, process, command), `target`, `warn`, `fail`, `log`, `restart`; `machine` (none
//! by default): `name`, `via`, `host`, `ssh`, `vnc`, `dash`, `service`; `refresh_secs` (0)
//! (`settings.rs`).
//!
//! Window (`window.rs`, flick-a2ed): `flick sys window` and cmd+K Open Fleet Window on
//! `Fleet` show the fleet on the floating surface "fleet", one bubble per machine; it polls
//! every `VISIBLE_EVERY` s while it shows; Esc hides it. In `NET_DENIED` (it pops a window).
//!
//! Cadence (`poll.rs`): the fleet polls on launcher open, on wake and on `sys fleet`; every
//! `VISIBLE_EVERY` s while the fleet view shows (`fleet_view`, set by the view, flick-9eb1);
//! every `refresh_secs` when set. Sleep stops the timer and drops rounds in flight. Idle
//! cost: with no machines, or `refresh_secs = 0` and the launcher closed, nothing runs.

mod act;
mod check;
mod fleet;
mod io;
mod jobs;
mod ops;
mod poll;
mod probe;
mod report;
mod run;
mod settings;
mod ssh;
#[cfg(test)]
mod testkit;
mod views;
mod window;
mod wire;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Section;
use crate::core::control::PeerHooks;
use crate::core::{Action, Cx, Event, Item, ItemId, ListView, Module, Outcome, unknown_verb};
use fleet::{Fleet, Local, VISIBLE_DUE, VISIBLE_EVERY};
use io::{Hooks, Shared, State};
use settings::Settings;
use views::{Key, Seen};

pub const ID: &str = "sys";

/// How long the first answer with an empty cache waits for its round: the probe's budget
/// plus time for a killed child's result to land.
const FIRST_WAIT: Duration = Duration::from_millis(2_250);

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How a fleet poll was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Poll {
    /// Launcher opened, `sys fleet`: machines read `VISIBLE_EVERY` s ago or longer.
    Open,
    /// Wake: every machine.
    Now,
    /// A `ModuleChanged` (a result, the visible round's post, the timer): due machines only.
    Tick,
}

/// What Enter on a fleet row does.
enum Go {
    Machine(String),
    Agents,
    Fleet,
    Log,
    Stay(Option<String>),
}

pub struct Sys {
    shared: Arc<Shared>,
    hooks: Hooks,
    first_wait: Duration,
    refresh_secs: u64,
    /// `Started` was seen: events may start threads.
    started: bool,
    /// View `fleet` or `machine` is open: with the launcher on screen the fleet polls every
    /// `VISIBLE_EVERY` s. Set by `open`; cleared when root search lists items, on
    /// `LauncherOpened` and when Herdr Agents replaces the view.
    fleet_view: bool,
    /// The machine view `machine` shows.
    detail: Option<String>,
    /// The fleet window (`window.rs`).
    win: window::Win,
    /// The visible cadence, `VISIBLE_EVERY` s (tests shorten it).
    tick: Duration,
}

impl Sys {
    /// The module with the real hooks; `peer` asks other Macs' Flicks (`cli::PEER`).
    pub fn new(peer: PeerHooks) -> Sys {
        Sys::with_hooks(wire::hooks(peer), wire::WINDOW)
    }

    fn with_hooks(hooks: Hooks, win: window::Hooks) -> Sys {
        let shared = Arc::default();
        let win = window::Win::new(win);
        let tick = Duration::from_secs(VISIBLE_EVERY);
        Sys { shared, hooks, first_wait: FIRST_WAIT, refresh_secs: 0, started: false, fleet_view: false, detail: None, win, tick }
    }

    /// Poll the fleet as `kind` asks, if it has machines. Refreshes this Mac's own cache
    /// too when a machine is `via = "local"`. The fleet view on screen, or the fleet window,
    /// keeps it polling: each poll then leaves a tick pending (`poll::tick_after`).
    fn poll(&self, kind: Poll) {
        if self.shared.lock().fleet.slots.is_empty() {
            return;
        }
        let visible = (self.fleet_view && (self.hooks.visible)()) || self.win.visible();
        let min_age = match kind {
            Poll::Now => 0,
            Poll::Open | Poll::Tick if visible => VISIBLE_DUE,
            Poll::Open => VISIBLE_EVERY - 1,
            // A tick may land a little before a full interval since the last read ended.
            Poll::Tick if self.refresh_secs > 0 => self.refresh_secs * 3 / 4,
            Poll::Tick => return,
        };
        let now = (self.hooks.now)();
        let local = {
            let st = self.shared.lock();
            st.fleet.has_local() && st.snapshot.as_ref().is_none_or(|s| now.saturating_sub(s.at) >= min_age)
        };
        if local {
            io::refresh_probe(&self.shared, self.hooks);
            io::refresh_services(&self.shared, self.hooks);
        }
        poll::round(&self.shared, min_age, self.hooks);
        if visible {
            poll::tick_after(&self.shared, self.tick, self.hooks);
        }
    }

    /// Run the background timer iff started with `refresh_secs` and machines.
    fn restart_timer(&self) {
        poll::stop_timer(&self.shared);
        let machines = !self.shared.lock().fleet.slots.is_empty();
        if self.started && self.refresh_secs > 0 && machines {
            poll::start_timer(&self.shared, Duration::from_secs(self.refresh_secs), self.hooks);
        }
    }

    /// `sys fleet`. A remote caller (a peer, or `--remote`) reads the cache only: it never
    /// makes this Mac ssh or ask its peers.
    fn fleet(&self, json: bool, remote: bool) -> String {
        let first = self.shared.lock().fleet.never_tried();
        if !remote {
            self.poll(Poll::Open);
        }
        if first && !remote {
            self.shared.wait(self.first_wait, |s| !s.fleet.busy() && !s.probing);
        }
        let now = (self.hooks.now)();
        let st = self.shared.lock();
        let local = local(&st, now);
        let stale = fleet::stale_after(self.refresh_secs);
        if json {
            fleet::fleet_json(&st.fleet, &local, now, stale).to_string()
        } else {
            fleet::fleet_text(&st.fleet, &local, now, stale)
        }
    }

    /// Open view `name` and poll as the launcher does, now that it shows the fleet.
    fn show(&mut self, name: &str, placeholder: &str) -> ListView {
        self.fleet_view = true;
        if self.started {
            self.poll(Poll::Open);
        }
        let empty = "No machines (add [[sys.machine]] tables to config.toml)".into();
        ListView { placeholder: placeholder.into(), empty, ..ListView::new(ID, name) }
    }

    /// Run `f` over every machine as the launcher shows it, and the fleet.
    fn with_seen<R>(&self, f: impl FnOnce(&[Seen], &Fleet) -> R) -> R {
        let now = (self.hooks.now)();
        let st = self.shared.lock();
        let local = local(&st, now);
        f(&views::seen(&st.fleet, &local, now, fleet::stale_after(self.refresh_secs)), &st.fleet)
    }

    fn snapshot(&self, json: bool) -> Result<String, String> {
        let empty = self.shared.lock().snapshot.is_none();
        io::refresh_probe(&self.shared, self.hooks);
        io::refresh_services(&self.shared, self.hooks);
        if empty {
            self.shared.wait(self.first_wait, |s| s.snapshot.is_some() || !s.probing);
        }
        let now = (self.hooks.now)();
        let st = self.shared.lock();
        let error = st.probe_error.as_deref();
        let Some(snap) = &st.snapshot else {
            return Err(format!("sys: no snapshot yet ({})", error.unwrap_or("probe still running")));
        };
        Ok(if json {
            report::snapshot_json(snap, error, &st.services, now).to_string()
        } else {
            report::snapshot_text(snap, error, &st.services, now)
        })
    }

    fn services(&self, json: bool) -> String {
        let unchecked = self.shared.lock().services.iter().all(|e| e.verdict.is_none());
        io::refresh_services(&self.shared, self.hooks);
        if unchecked {
            self.shared.wait(self.first_wait, |s| !s.checking);
        }
        let now = (self.hooks.now)();
        let st = self.shared.lock();
        if json {
            report::services_json(&st.services, now).to_string()
        } else {
            report::services_text(&st.services, now)
        }
    }
}

/// This Mac's own snapshot, for a `via = "local"` machine.
fn local(st: &State, now: u64) -> Local {
    Local {
        snapshot: st.snapshot.as_ref().map(|s| report::snapshot_json(s, st.probe_error.as_deref(), &st.services, now)),
        at: st.snapshot.as_ref().map(|s| s.at),
        error: st.probe_error.clone(),
    }
}

impl Module for Sys {
    fn id(&self) -> &'static str {
        ID
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let settings = table.get::<Settings>()?.check()?;
        self.shared.set_services(&settings.service);
        self.shared.lock().fleet.set_machines(&settings.machine);
        self.refresh_secs = settings.refresh_secs;
        // Only a started module runs timers: a reload configures a throwaway one too.
        if self.started {
            self.restart_timer();
        }
        Ok(())
    }

    /// `Fleet`, only with machines. Root search is on screen, so no fleet view is.
    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        self.fleet_view = false;
        self.with_seen(|seen, _| views::root_item(seen).into_iter().collect())
    }

    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        match view {
            "fleet" => Some(self.show(view, "Search machines and services…")),
            "machine" if self.detail.is_some() => Some(self.show(view, "Search this machine…")),
            "log" => Some(self.show_log()),
            _ => None,
        }
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        if view.name == "log" {
            return self.fill_log(view);
        }
        let now = (self.hooks.now)();
        let acted = jobs::acted(self.shared.lock().acted.as_ref(), now).map(str::to_string);
        let detail = self.detail.as_deref();
        let (items, footer) = self.with_seen(|seen, _| {
            if view.name == "machine" {
                let mine = seen.iter().find(|s| detail == Some(s.slot.machine.name.as_str()));
                views::machine_items(mine, now)
            } else {
                (views::fleet_items(seen), format!("{}  ·  esc to go back", views::summary_line(seen)))
            }
        });
        view.footer = match acted {
            Some(a) => format!("{a}  ·  {footer}"),
            None => footer,
        };
        // The bonus keeps the view's order (machine, then its services) for equal scores.
        let order: HashMap<String, usize> =
            items.iter().enumerate().map(|(i, item)| (item.id.to_string(), i)).collect();
        view.items = cx.ranker.rank(cx.query, items, |i| -(order[i.id.as_str()] as f64) * 1e-3);
    }

    fn activate(&mut self, id: &ItemId, _cx: &mut Cx) -> Outcome {
        let detail = self.detail.clone();
        let go = self.with_seen(|seen, fleet| match Key::parse(id.key(), fleet) {
            Some(Key::Machine(m) | Key::Service { machine: m, .. }) => Go::Machine(m.to_string()),
            Some(Key::Check(name)) => {
                let mine = seen.iter().find(|s| detail.as_deref() == Some(s.slot.machine.name.as_str()));
                Go::Stay(mine.and_then(|s| views::check_status(s, name)))
            }
            Some(Key::Agents) => Go::Agents,
            Some(Key::Fleet | Key::Head | Key::Fact) => Go::Fleet,
            Some(Key::Log) => Go::Log,
            None => Go::Stay(None),
        });
        match go {
            Go::Machine(m) => {
                self.detail = Some(m);
                Outcome::Push(ListView::new(ID, "machine"))
            }
            Go::Agents => {
                self.fleet_view = false;
                Outcome::Push(ListView::new("herdr", "agents"))
            }
            Go::Fleet => Outcome::Push(ListView::new(ID, "fleet")),
            Go::Log => self.tail_again(),
            Go::Stay(status) => Outcome::Stay(status),
        }
    }

    /// Machines: Screen Sharing, Open Dash. Services this Mac's config defines: Tail Log,
    /// Restart (`ops.rs`).
    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        self.menu(id)
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        self.run_action(id, key, cx)
    }

    fn confirmed(&mut self, token: &str, _cx: &mut Cx) -> Outcome {
        self.confirm_restart(token)
    }

    /// `--json` (`cx.json`) answers with the JSON in `report.rs`.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v] if v == "snapshot" => self.snapshot(cx.json),
            [v] if v == "services" => Ok(self.services(cx.json)),
            [v] if v == "fleet" => Ok(self.fleet(cx.json, cx.remote)),
            [v, rest @ ..] if v == "tail" => self.tail_command(rest),
            [v, rest @ ..] if v == "restart" => self.restart_command(rest),
            [v, rest @ ..] if v == "window" => self.window_command(rest),
            _ => Err(unknown_verb(ID, args)),
        }
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        match event {
            Event::Started => {
                self.started = true;
                self.restart_timer();
            }
            Event::LauncherOpened if self.started => {
                self.fleet_view = false;
                self.poll(Poll::Open);
            }
            Event::Wake if self.started => {
                self.restart_timer();
                self.poll(Poll::Now);
            }
            Event::Sleep => {
                poll::stop_timer(&self.shared);
                self.shared.lock().fleet.forget_round();
            }
            Event::ModuleChanged { module: ID } if self.started => {
                self.window_drain();
                self.poll(Poll::Tick);
                self.window_refresh();
            }
            _ => {}
        }
        false
    }

    fn verbs(&self) -> &'static str {
        "sys snapshot | sys services | sys fleet | sys tail [<machine>] <service> | sys restart [<machine>] <service> [--yes] | sys window [--snapshot <png>]"
    }
}

#[cfg(test)]
mod tests;
