//! Module `sys`: this Mac's health, for the fleet dashboard (plan flick-b5d0). Steps so
//! far: the probe and a cached `sys snapshot` (flick-bd74). No items or views yet.
//!
//! `flick sys snapshot [--json]`: CPU load, memory, disks, battery, thermal and uptime
//! from one run of `probe::PROBE` (stock tools, no FFI). It answers from the cache and
//! starts a refresh on a thread (`io.rs`); the first call with an empty cache waits for
//! it, at most `FIRST_WAIT`. The JSON (`report.rs`) is what a peer's fleet view reads over
//! the network: the verb is read-only and outside `NET_DENIED`.
//!
//! Table `[sys]`: no settings yet. Idle cost: nothing runs until a verb asks; no timers.

mod io;
mod probe;
mod report;
mod run;
#[cfg(test)]
mod testkit;
mod wire;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::{Cx, Module, unknown_verb};
use io::{Hooks, Shared};

pub const ID: &str = "sys";

/// How long the first answer with an empty cache waits for its round: the probe's budget
/// plus time for a killed child's result to land.
const FIRST_WAIT: Duration = Duration::from_millis(2_250);

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub struct Sys {
    shared: Arc<Shared>,
    hooks: Hooks,
    first_wait: Duration,
}

impl Default for Sys {
    fn default() -> Self {
        Sys::with_hooks(wire::HOOKS)
    }
}

impl Sys {
    fn with_hooks(hooks: Hooks) -> Sys {
        Sys { shared: Arc::default(), hooks, first_wait: FIRST_WAIT }
    }

    fn snapshot(&self, json: bool) -> Result<String, String> {
        let empty = self.shared.lock().snapshot.is_none();
        io::refresh_probe(&self.shared, self.hooks);
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
            report::snapshot_json(snap, error, now).to_string()
        } else {
            report::snapshot_text(snap, error, now)
        })
    }


}

impl Module for Sys {
    fn id(&self) -> &'static str {
        ID
    }

    /// `--json` (`cx.json`) answers with the JSON in `report.rs`.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v] if v == "snapshot" => self.snapshot(cx.json),
            _ => Err(unknown_verb(ID, args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "sys snapshot"
    }
}

#[cfg(test)]
mod tests;
