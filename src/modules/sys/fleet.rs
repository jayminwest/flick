//! The fleet: each `[[sys.machine]]`'s last snapshot (the `sys snapshot --json` shape of
//! `report.rs`, whoever produced it), its age and its last error, and `sys fleet` answers.
//! Pure: `poll.rs` fills it from threads, `mod.rs` renders it. A `via = "local"` machine has
//! no slot data; its snapshot is this Mac's own cache (`Local`), read when rendering.

use serde_json::{Value, json};

use super::report::{SCHEMA, ago};
use super::settings::{Machine, Via};

/// Poll cadence while the fleet view shows, seconds.
pub const VISIBLE_EVERY: u64 = 15;
/// While it shows, a machine (or this Mac's cache) read this long ago is due: a little under
/// the cadence, so a read that ended after the tick that started it (the probe takes up to
/// 2 s, clocks round to seconds) is still due at the next tick.
pub const VISIBLE_DUE: u64 = VISIBLE_EVERY - 3;

/// One machine and what the fleet knows about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub machine: Machine,
    /// The last good snapshot, `report::snapshot_json` shape.
    pub snapshot: Option<Value>,
    /// When `snapshot` arrived, unix seconds on this Mac's clock.
    pub fetched_at: Option<u64>,
    /// When the last attempt ended, good or bad.
    pub tried_at: Option<u64>,
    /// Why the last attempt failed; the snapshot before it stays.
    pub error: Option<String>,
    /// What answered last: the machine's `via`, or `ssh` after a fallback.
    pub source: Option<Via>,
    /// Why a fallback was taken (`flick too old ...`).
    pub note: Option<String>,
    /// A read of this machine runs. Per machine, so one slow ssh read does not hold the
    /// others' next reads (flick-fd36).
    pub busy: bool,
}

/// A good answer from one machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Fetched {
    pub snapshot: Value,
    pub source: Via,
    pub note: Option<String>,
}

#[derive(Debug, Default)]
pub struct Fleet {
    pub slots: Vec<Slot>,
    /// Bumped by a reload and by sleep: results of rounds started before are dropped.
    pub epoch: u64,
    /// Bumped to end the background timer.
    pub timer: u64,
    /// A visible tick is pending (`poll::tick_after`).
    pub ticking: bool,
}

/// This Mac's own snapshot for a `via = "local"` machine.
pub struct Local {
    pub snapshot: Option<Value>,
    pub at: Option<u64>,
    pub error: Option<String>,
}

impl Fleet {
    /// Use `machines`, in this order. A machine whose config did not change keeps what it
    /// knows; a round in flight drops its results.
    pub fn set_machines(&mut self, machines: &[Machine]) {
        let old = std::mem::take(&mut self.slots);
        self.slots = machines
            .iter()
            .map(|m| {
                let kept = old.iter().find(|s| s.machine == *m).cloned();
                kept.unwrap_or_else(|| Slot {
                    machine: m.clone(),
                    snapshot: None,
                    fetched_at: None,
                    tried_at: None,
                    error: None,
                    source: None,
                    note: None,
                    busy: false,
                })
            })
            .collect();
        self.forget_round();
    }

    /// Drop the reads in flight (reload, sleep): their late results are not applied.
    pub fn forget_round(&mut self) {
        self.epoch += 1;
        for slot in &mut self.slots {
            slot.busy = false;
        }
    }

    /// A read of some machine runs.
    pub fn busy(&self) -> bool {
        self.slots.iter().any(|s| s.busy)
    }

    pub fn has_local(&self) -> bool {
        self.slots.iter().any(|s| s.machine.via == Via::Local)
    }

    /// Remote machines (by index) not being read whose last attempt ended `min_age` seconds
    /// ago or longer.
    pub fn due(&self, now: u64, min_age: u64) -> Vec<(usize, Machine)> {
        let due = |s: &&Slot| !s.busy && s.tried_at.is_none_or(|t| now.saturating_sub(t) >= min_age);
        let remote = self.slots.iter().enumerate().filter(|(_, s)| s.machine.via != Via::Local);
        remote.filter(|(_, s)| due(s)).map(|(i, s)| (i, s.machine.clone())).collect()
    }

    /// Machine `i`'s read from `epoch` ended: it is no longer busy, unless a reload or sleep
    /// dropped that read (a newer one may run).
    pub fn release(&mut self, epoch: u64, i: usize) {
        if let Some(slot) = self.slots.get_mut(i).filter(|_| epoch == self.epoch) {
            slot.busy = false;
        }
    }

    /// Store machine `i`'s result from round `epoch`, unless the round was dropped or the
    /// machine changed meanwhile; either way its read is over (`release`). Whether it was
    /// stored.
    pub fn apply(&mut self, epoch: u64, i: usize, machine: &Machine, got: Result<Fetched, String>, now: u64) -> bool {
        self.release(epoch, i);
        if epoch != self.epoch {
            return false;
        }
        let Some(slot) = self.slots.get_mut(i).filter(|s| s.machine == *machine) else { return false };
        slot.tried_at = Some(now);
        match got {
            Ok(f) => {
                slot.snapshot = Some(f.snapshot);
                slot.fetched_at = Some(now);
                slot.error = None;
                slot.source = Some(f.source);
                slot.note = f.note;
            }
            Err(e) => slot.error = Some(e),
        }
        true
    }

    /// Nothing known yet about any remote machine and no answer pending.
    pub fn never_tried(&self) -> bool {
        self.slots.iter().all(|s| s.machine.via == Via::Local || s.tried_at.is_none())
    }
}

/// Data older than this is stale: three times the cadence (`refresh_secs`, else the
/// visible cadence).
pub fn stale_after(refresh_secs: u64) -> u64 {
    3 * if refresh_secs > 0 { refresh_secs } else { VISIBLE_EVERY }
}

/// One machine's row, whatever answered.
pub struct Row<'a> {
    /// The last good snapshot, `report::snapshot_json` shape.
    pub snapshot: Option<&'a Value>,
    /// When it was read, unix seconds on this Mac's clock.
    pub at: Option<u64>,
    pub error: Option<&'a str>,
    pub source: Option<Via>,
}

impl<'a> Row<'a> {
    pub fn new(slot: &'a Slot, local: &'a Local) -> Row<'a> {
        if slot.machine.via == Via::Local {
            let source = local.snapshot.as_ref().map(|_| Via::Local);
            return Row { snapshot: local.snapshot.as_ref(), at: local.at, error: local.error.as_deref(), source };
        }
        Row { snapshot: slot.snapshot.as_ref(), at: slot.fetched_at, error: slot.error.as_deref(), source: slot.source }
    }

    /// `fresh`, `stale` (old, or kept after a failure), `down` (failed, nothing kept) or
    /// `pending` (nothing yet).
    pub fn state(&self, now: u64, stale_after: u64) -> &'static str {
        match (self.snapshot, self.error) {
            (None, None) => "pending",
            (None, Some(_)) => "down",
            (Some(_), Some(_)) => "stale",
            (Some(_), None) if self.age(now).is_some_and(|a| a > stale_after) => "stale",
            (Some(_), None) => "fresh",
        }
    }

    pub fn age(&self, now: u64) -> Option<u64> {
        self.at.map(|t| now.saturating_sub(t))
    }
}

/// `{"schema", "stale_after_secs", "machines": [{name, via, host, ssh, vnc, dash, source,
/// state, stale, fetched_at, age_secs, error, note, snapshot}]}`.
pub fn fleet_json(fleet: &Fleet, local: &Local, now: u64, stale_after: u64) -> Value {
    let machines: Vec<Value> = fleet
        .slots
        .iter()
        .map(|slot| {
            let row = Row::new(slot, local);
            let m = &slot.machine;
            let state = row.state(now, stale_after);
            json!({
                "name": m.name,
                "via": m.via.as_str(),
                "host": (m.via == Via::Flick).then(|| m.flick_host()),
                "ssh": m.ssh,
                "vnc": m.vnc,
                "dash": m.dash,
                "source": row.source.map(Via::as_str),
                "state": state,
                "stale": state == "stale",
                "fetched_at": row.at,
                "age_secs": row.age(now),
                "error": row.error,
                "note": slot.note,
                "snapshot": row.snapshot,
            })
        })
        .collect();
    json!({ "schema": SCHEMA, "stale_after_secs": stale_after, "machines": machines })
}

/// One line per machine (state, name, how it was read, metrics, age), then its error and a
/// line per service that is not ok.
pub fn fleet_text(fleet: &Fleet, local: &Local, now: u64, stale_after: u64) -> String {
    if fleet.slots.is_empty() {
        return "no machines (add [[sys.machine]] tables to config.toml)".into();
    }
    let width = fleet.slots.iter().map(|s| s.machine.name.chars().count()).max().unwrap_or(0);
    let mut lines = vec![];
    for slot in &fleet.slots {
        let row = Row::new(slot, local);
        let via = row.source.unwrap_or(slot.machine.via).as_str();
        let mut parts = row.snapshot.map(metrics).unwrap_or_default();
        if let Some(age) = row.age(now) {
            parts.push(format!("{} ago", ago(age)));
        }
        let state = row.state(now, stale_after);
        lines.push(format!("{state:<8} {:<width$}  {via:<5}  {}", slot.machine.name, parts.join(" · ")).trim_end().to_string());
        if let Some(note) = &slot.note {
            lines.push(format!("{:9}note: {note}", ""));
        }
        if let Some(e) = row.error {
            lines.push(format!("{:9}error: {e}", ""));
        }
        let services = row.snapshot.and_then(|s| s["services"].as_array()).map_or(&[][..], Vec::as_slice);
        for s in services.iter().filter(|s| s["status"] != "ok") {
            let field = |k: &str| s[k].as_str().unwrap_or_default().to_string();
            lines.push(format!("{:9}{:<8} {}  {}", "", field("status"), field("name"), field("reason")));
        }
    }
    lines.join("\n")
}

/// `load 1.42 · mem 37% free · disk 61% · batt 80% · 3 ok · 1 fail` from a snapshot.
pub fn metrics(snap: &Value) -> Vec<String> {
    let mut parts = vec![];
    if let Some(load) = snap["cpu_load"][0].as_f64() {
        parts.push(format!("load {load:.2}"));
    }
    if let Some(free) = snap["mem_free_pct"].as_u64() {
        parts.push(format!("mem {free}% free"));
    }
    let disks = snap["disks"].as_array().map_or(&[][..], Vec::as_slice);
    if let Some(used) = disks.iter().filter_map(|d| d["used_pct"].as_u64()).max() {
        parts.push(format!("disk {used}%"));
    }
    if let Some(pct) = snap["battery"]["percent"].as_u64() {
        parts.push(format!("batt {pct}%"));
    }
    let services = snap["services"].as_array().map_or(&[][..], Vec::as_slice);
    for status in ["ok", "warn", "fail", "unknown"] {
        let n = services.iter().filter(|s| s["status"] == status).count();
        if n > 0 {
            parts.push(format!("{n} {status}"));
        }
    }
    parts
}

#[cfg(test)]
mod tests;
