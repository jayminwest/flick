//! Module `kota`: KOTA's presence in the menu bar, its pending-card badge and quick ask
//! (plan flick-4354). KOTA is the always-on Claude Code session in a herdr pane on
//! mbp-server. Steps so far: the presence model and `kota status` (flick-3b9b), the
//! poller and `kota refresh` (flick-4f39). No items, views or menu bar item yet.
//!
//! Presence: a round runs `herdr [--machine <machine>] agent list` and curl on kota-dash's
//! `/ok` in parallel (`io.rs`), finds the KOTA pane (agent `claude` in `cwd`) and maps
//! the round to thinking, blocked, idle, degraded, down, offline or unknown, debounced
//! (`presence.rs`). `view.rs` renders it as plain data.
//!
//! Rounds run on a timer only when the `[kota]` table sets a key (any key; Flick cannot
//! tell an empty `[kota]` from a missing one): every `poll_secs` (default 60), every 15 s
//! while KOTA works, and 15 s after a first failure. Without one, an idle Flick runs no
//! thread and no child for this module: `[kota]` is in the same config.toml on every Mac,
//! and only the Mac that wants KOTA's presence should poll. `kota refresh` runs a round on
//! demand either way (at most one per 10 s). Sleep and lock stop the timer; wake and
//! unlock run a round after 5 s.
//!
//! `flick kota status [--json]` (no I/O) and `flick kota refresh` are allowed over the
//! network: both only read, and refresh is rate-limited.
//!
//! Table `[kota]` (`settings.rs`): `machine`, `cwd`, `pane`, `herdr`, `dash`, `ssh`,
//! `kota_ask`, `poll_secs`, `fast_secs`, `status_item`, `notify_down`, `hotkey`.

mod io;
mod presence;
mod run;
mod settings;
#[cfg(test)]
mod testkit;
mod view;
mod wire;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Section;
use crate::core::{Cx, Event, Module, unknown_verb};
use io::{Hooks, Shared};
use settings::Settings;
use view::{Extra, Polling};

pub const ID: &str = "kota";

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub struct Kota {
    settings: Settings,
    /// The `[kota]` table sets a key: rounds run on a timer.
    active: bool,
    /// `Started` arrived: threads may run. Before it (and in a throwaway instance that a
    /// reload only configures) nothing starts.
    started: bool,
    shared: Arc<Shared>,
    hooks: Hooks,
    /// Cards waiting on the user, from the last `Event::CardsPending` (the `message`
    /// module owns the cards).
    pending: u32,
}

impl Default for Kota {
    fn default() -> Self {
        Kota::with_hooks(wire::HOOKS)
    }
}

impl Drop for Kota {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

impl Kota {
    fn with_hooks(hooks: Hooks) -> Kota {
        Kota { settings: Settings::default(), active: false, started: false, shared: Arc::default(), hooks, pending: 0 }
    }

    /// Timed rounds run.
    fn polling(&self) -> bool {
        self.started && self.active
    }

    fn status(&self, json: bool) -> String {
        let polling = match (self.active, self.settings.poll_secs) {
            (false, _) => Polling::Off,
            (true, 0) => Polling::OnDemand,
            (true, secs) => Polling::Every(secs),
        };
        let extra = Extra { pending: self.pending, polling };
        let p = self.shared.lock();
        if json {
            return view::status_json(&p.presence, extra).to_string();
        }
        let now = (self.hooks.now)();
        let offset = (self.hooks.utc_offset)(i64::try_from(now).unwrap_or(0));
        view::status_text(&p.presence, extra, now, offset)
    }
}

impl Module for Kota {
    fn id(&self) -> &'static str {
        ID
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let settings = table.get::<Settings>()?.check()?;
        let active = !table.get::<toml::Table>()?.is_empty();
        let old = std::mem::replace(&mut self.settings, settings);
        let was = std::mem::replace(&mut self.active, active);
        if !self.started {
            return Ok(());
        }
        let s = &self.settings;
        let target = |s: &Settings| (s.machine.clone(), s.cwd.clone(), s.pane.clone(), s.herdr.clone(), s.dash.clone());
        if target(&old) != target(s) {
            self.shared.reset();
        } else if was && !active {
            self.shared.stop();
        }
        if self.active && (old != *s || !was) {
            io::tick(&self.shared, s, self.hooks);
        }
        Ok(())
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        match event {
            Event::Started => self.started = true,
            Event::CardsPending { count } => self.pending = count,
            _ => {}
        }
        if !self.polling() {
            return false;
        }
        let (shared, s, hooks) = (&self.shared, &self.settings, self.hooks);
        match event {
            Event::Started => io::tick(shared, s, hooks),
            Event::ModuleChanged { module: ID } => {
                // flick-78b9 notifies on a change to down; until then nothing reads them.
                drop(std::mem::take(&mut shared.lock().transitions));
                io::tick(shared, s, hooks);
            }
            Event::Sleep | Event::Locked => io::suspend(shared),
            Event::Wake | Event::Unlocked => io::resume(shared, s, hooks),
            Event::LauncherOpened if s.poll_secs == 0 => drop(io::refresh(shared, s, hooks)),
            _ => {}
        }
        false
    }

    /// `--json` (`cx.json`) makes `status` answer with JSON (`view::status_json`).
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v] if v == "status" => Ok(self.status(cx.json)),
            [v] if v == "refresh" => Ok(io::refresh(&self.shared, &self.settings, self.hooks)),
            _ => Err(unknown_verb(ID, args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "kota status | kota refresh"
    }
}

#[cfg(test)]
mod tests;
