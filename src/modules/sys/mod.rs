//! Module `sys`: this Mac's health and the services it checks, for the fleet dashboard
//! (plan flick-b5d0). Steps so far: the probe and a cached `sys snapshot` (flick-bd74),
//! `[[sys.service]]` checks and `sys services` (flick-4573), the `[[sys.machine]]` fleet and
//! `sys fleet` (flick-3608). No items or views yet.
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
//! Cadence (`poll.rs`): the fleet polls on launcher open, on wake and on `sys fleet`; every
//! `VISIBLE_EVERY` s while the fleet view shows (`fleet_view`, set by the view, flick-9eb1);
//! every `refresh_secs` when set. Sleep stops the timer and drops rounds in flight. Idle
//! cost: with no machines, or `refresh_secs = 0` and the launcher closed, nothing runs.

mod check;
mod fleet;
mod io;
mod poll;
mod probe;
mod report;
mod run;
mod settings;
mod ssh;
#[cfg(test)]
mod testkit;
mod wire;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Section;
use crate::core::control::PeerHooks;
use crate::core::{Cx, Event, Module, unknown_verb};
use fleet::{Local, VISIBLE_EVERY};
use io::{Hooks, Shared};
use settings::Settings;

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

pub struct Sys {
    shared: Arc<Shared>,
    hooks: Hooks,
    first_wait: Duration,
    refresh_secs: u64,
    /// `Started` was seen: events may start threads.
    started: bool,
    /// The fleet view is open (flick-9eb1 sets it); with the launcher on screen it polls
    /// every `VISIBLE_EVERY` s.
    fleet_view: bool,
}

impl Sys {
    /// The module with the real hooks; `peer` asks other Macs' Flicks (`cli::PEER`).
    pub fn new(peer: PeerHooks) -> Sys {
        Sys::with_hooks(wire::hooks(peer))
    }

    fn with_hooks(hooks: Hooks) -> Sys {
        let shared = Arc::default();
        Sys { shared, hooks, first_wait: FIRST_WAIT, refresh_secs: 0, started: false, fleet_view: false }
    }

    /// Poll the fleet as `kind` asks, if it has machines. Refreshes this Mac's own cache
    /// too when a machine is `via = "local"`.
    fn poll(&self, kind: Poll) {
        let visible = self.fleet_view && (self.hooks.visible)();
        let (min_age, again) = match kind {
            Poll::Now => (0, None),
            Poll::Open | Poll::Tick if visible => (VISIBLE_EVERY - 1, Some(Duration::from_secs(VISIBLE_EVERY))),
            Poll::Open => (VISIBLE_EVERY - 1, None),
            // A tick may land a little before a full interval since the last read ended.
            Poll::Tick if self.refresh_secs > 0 => (self.refresh_secs * 3 / 4, None),
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
        poll::round(&self.shared, min_age, again, self.hooks);
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
            self.shared.wait(self.first_wait, |s| !s.fleet.busy && !s.probing);
        }
        let now = (self.hooks.now)();
        let st = self.shared.lock();
        let local = Local {
            snapshot: st.snapshot.as_ref().map(|s| report::snapshot_json(s, st.probe_error.as_deref(), &st.services, now)),
            at: st.snapshot.as_ref().map(|s| s.at),
            error: st.probe_error.clone(),
        };
        let stale = fleet::stale_after(self.refresh_secs);
        if json {
            fleet::fleet_json(&st.fleet, &local, now, stale).to_string()
        } else {
            fleet::fleet_text(&st.fleet, &local, now, stale)
        }
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

    /// `--json` (`cx.json`) answers with the JSON in `report.rs`.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v] if v == "snapshot" => self.snapshot(cx.json),
            [v] if v == "services" => Ok(self.services(cx.json)),
            [v] if v == "fleet" => Ok(self.fleet(cx.json, cx.remote)),
            _ => Err(unknown_verb(ID, args)),
        }
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        match event {
            Event::Started => {
                self.started = true;
                self.restart_timer();
            }
            Event::LauncherOpened if self.started => self.poll(Poll::Open),
            Event::Wake if self.started => {
                self.restart_timer();
                self.poll(Poll::Now);
            }
            Event::Sleep => {
                poll::stop_timer(&self.shared);
                self.shared.lock().fleet.forget_round();
            }
            Event::ModuleChanged { module: ID } if self.started => self.poll(Poll::Tick),
            _ => {}
        }
        false
    }

    fn verbs(&self) -> &'static str {
        "sys snapshot | sys services | sys fleet"
    }
}

#[cfg(test)]
mod tests;
