//! The fleet in the launcher (flick-9eb1): the root item, view `fleet` (a row per machine,
//! a row per service of each, then Herdr Agents) and view `machine` (one machine's facts
//! and services). Pure: no locks, no clock (callers pass `now`), no I/O.
//!
//! Item keys: `fleet` (the root item; permanent, it keys the usage table), and in views
//! (`record_use = false`, never stored) `machine/<machine>` and `service/<machine>/<service>`
//! in `fleet`; `head`, `fact/<n>` and `check/<service>` in `machine`; `agents` in both
//! (pushes herdr's `agents` view by name).

use serde_json::Value;

use super::ID;
use super::fleet::{Fleet, Local, Row, Slot, metrics};
use super::probe::Snapshot;
use super::report::{self, ago};
use crate::core::{Icon, Item, ItemId};

/// What an item key names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    Fleet,
    Machine(&'a str),
    Service { machine: &'a str, service: &'a str },
    Head,
    Fact,
    Check(&'a str),
    Agents,
}

impl Key<'_> {
    /// `fleet` names the machines, so a `service/` key splits after a machine's name even
    /// when a name holds a `/`.
    pub fn parse<'a>(key: &'a str, fleet: &Fleet) -> Option<Key<'a>> {
        match key {
            "fleet" => return Some(Key::Fleet),
            "head" => return Some(Key::Head),
            "agents" => return Some(Key::Agents),
            _ => {}
        }
        if let Some(machine) = key.strip_prefix("machine/") {
            return Some(Key::Machine(machine));
        }
        if let Some(rest) = key.strip_prefix("service/") {
            let split = |s: &Slot| {
                let n = s.machine.name.len();
                rest.strip_prefix(s.machine.name.as_str())?.strip_prefix('/').map(|service| (n, service))
            };
            let (n, service) = fleet.slots.iter().filter_map(split).max_by_key(|(n, _)| *n)?;
            return Some(Key::Service { machine: &rest[..n], service });
        }
        if let Some(service) = key.strip_prefix("check/") {
            return Some(Key::Check(service));
        }
        key.starts_with("fact/").then_some(Key::Fact)
    }
}

/// A service in a snapshot's `services` list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Svc<'a> {
    pub name: &'a str,
    pub kind: &'a str,
    pub status: &'a str,
    pub reason: &'a str,
}

/// The services a snapshot carries, in its order.
pub fn services(snap: Option<&Value>) -> Vec<Svc<'_>> {
    let list = snap.and_then(|s| s["services"].as_array()).map_or(&[][..], Vec::as_slice);
    list.iter().map(svc).collect()
}

fn svc(s: &Value) -> Svc<'_> {
    Svc {
        name: s["name"].as_str().unwrap_or_default(),
        kind: s["kind"].as_str().unwrap_or_default(),
        status: s["status"].as_str().unwrap_or("unknown"),
        reason: s["reason"].as_str().unwrap_or_default(),
    }
}

/// A service status's icon.
pub fn status_icon(status: &str) -> Icon {
    Icon::Symbol(match status {
        "ok" => "checkmark.circle",
        "warn" => "exclamationmark.triangle",
        "fail" => "xmark.octagon",
        _ => "questionmark.circle",
    })
}

/// The worst status of `services`: fail, warn, unknown, then ok (also for none).
fn worst(services: &[Svc]) -> &'static str {
    let has = |st: &str| services.iter().any(|s| s.status == st);
    ["fail", "warn", "unknown"].into_iter().find(|st| has(st)).unwrap_or("ok")
}

/// A machine's icon: its state when it has no fresh data, else its worst service.
pub fn machine_icon(state: &str, services: &[Svc]) -> Icon {
    match state {
        "pending" => Icon::Symbol("hourglass"),
        "down" => Icon::Symbol("bolt.horizontal.circle"),
        "stale" => Icon::Symbol("clock.arrow.circlepath"),
        _ => status_icon(worst(services)),
    }
}

/// What the fleet knows of one machine, ready for rows.
pub struct Seen<'a> {
    pub slot: &'a Slot,
    pub row: Row<'a>,
    /// `fresh`, `stale`, `down` or `pending` (`Row::state`).
    pub state: &'static str,
    pub age: Option<u64>,
    pub services: Vec<Svc<'a>>,
}

impl<'a> Seen<'a> {
    pub fn new(slot: &'a Slot, local: &'a Local, now: u64, stale_after: u64) -> Seen<'a> {
        let row = Row::new(slot, local);
        let state = row.state(now, stale_after);
        let age = row.age(now);
        let services = services(row.snapshot);
        Seen { slot, row, state, age, services }
    }

    fn name(&self) -> &'a str {
        &self.slot.machine.name
    }

    /// `load 1.42 · mem 37% free · disk 61% · 3 ok`, the error when nothing was read, or
    /// `loading…`; then `via ssh` and the reason after a fallback, or the last error.
    pub fn subtitle(&self) -> String {
        let mut parts = self.row.snapshot.map(metrics).unwrap_or_default();
        match (self.state, self.row.error) {
            ("pending", _) => parts.push("loading…".into()),
            ("down", Some(e)) => parts.push(e.to_string()),
            (_, Some(e)) => parts.push(format!("last read failed: {e}")),
            _ => {}
        }
        if let Some(note) = &self.slot.note {
            let via = self.row.source.unwrap_or(self.slot.machine.via).as_str();
            parts.push(format!("via {via}: {note}"));
        }
        parts.join(" · ")
    }

    /// `12s ago`, or nothing before the first read.
    fn accessory(&self) -> String {
        self.age.map_or(String::new(), |a| format!("{} ago", ago(a)))
    }

    /// The machine's row in `fleet`; Enter shows the machine.
    pub fn item(&self) -> Item {
        let via = self.row.source.unwrap_or(self.slot.machine.via).as_str();
        Item {
            subtitle: self.subtitle(),
            accessory: self.accessory(),
            keywords: vec![via.into(), self.state.into(), "machine".into()],
            ..Item::new(
                ItemId::new(ID, format!("machine/{}", self.name())),
                self.name(),
                "Show Machine",
                machine_icon(self.state, &self.services),
            )
        }
    }

    /// One service's row in `fleet`; Enter shows its machine.
    fn service_item(&self, s: Svc) -> Item {
        Item {
            subtitle: format!("{} · {}", s.status, s.reason),
            accessory: s.kind.into(),
            keywords: vec![s.status.into(), s.kind.into(), s.reason.into()],
            ..Item::new(
                ItemId::new(ID, format!("service/{}/{}", self.name(), s.name)),
                format!("{} · {}", s.name, self.name()),
                "Show Machine",
                status_icon(s.status),
            )
        }
    }
}

/// Every machine of `fleet` as `Seen`.
pub fn seen<'a>(fleet: &'a Fleet, local: &'a Local, now: u64, stale_after: u64) -> Vec<Seen<'a>> {
    fleet.slots.iter().map(|s| Seen::new(s, local, now, stale_after)).collect()
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `3 machines · 1 down · 1 stale · 2 fail · 1 warn`, skipping zero counts.
pub fn summary_line(seen: &[Seen]) -> String {
    let mut parts = vec![count(seen.len(), "machine", "machines")];
    for state in ["down", "stale"] {
        let n = seen.iter().filter(|s| s.state == state).count();
        if n > 0 {
            parts.push(format!("{n} {state}"));
        }
    }
    for status in ["fail", "warn"] {
        let n = seen.iter().flat_map(|s| &s.services).filter(|s| s.status == status).count();
        if n > 0 {
            parts.push(format!("{n} {status}"));
        }
    }
    parts.join(" · ")
}

/// The root item, `Fleet`: shown only while `[[sys.machine]]` tables are configured.
pub fn root_item(seen: &[Seen]) -> Option<Item> {
    if seen.is_empty() {
        return None;
    }
    Some(Item {
        subtitle: summary_line(seen),
        accessory: "sys".into(),
        keywords: vec!["fleet machines servers services health status dashboard".into()],
        ..Item::new(ItemId::new(ID, "fleet"), "Fleet", "Show Fleet", Icon::Symbol("server.rack"))
    })
}

/// The row that pushes herdr's agents view.
fn agents_item() -> Item {
    Item {
        subtitle: "Coding agents on every machine".into(),
        accessory: "herdr".into(),
        keywords: vec!["herdr agents claude codex".into()],
        ..Item::new(ItemId::new(ID, "agents"), "Herdr Agents", "Show Agents", Icon::Symbol("terminal"))
    }
}

/// View `fleet`: each machine, then its services; Herdr Agents last.
pub fn fleet_items(seen: &[Seen]) -> Vec<Item> {
    let mut items = vec![];
    for s in seen {
        items.push(s.item());
        items.extend(s.services.iter().map(|svc| s.service_item(*svc)));
    }
    if !seen.is_empty() {
        items.push(agents_item());
    }
    items
}

/// A fact label's icon.
fn fact_icon(label: &str) -> Icon {
    Icon::Symbol(match label {
        "load" => "cpu",
        "memory" => "memorychip",
        "disk" => "internaldrive",
        "battery" => "battery.100",
        _ => "thermometer",
    })
}

/// View `machine`: the machine (its state, how it was read, its error), each fact its
/// snapshot holds, its services, then Herdr Agents. The footer line comes with it.
pub fn machine_items(seen: Option<&Seen>, now: u64) -> (Vec<Item>, String) {
    let Some(s) = seen else {
        return (vec![], "Machine gone (config changed)  ·  esc to go back".into());
    };
    let snap: Option<Snapshot> = s.row.snapshot.and_then(|v| serde_json::from_value(v.clone()).ok());
    let title = snap.as_ref().map_or_else(|| s.name().to_string(), |snap| report::head(snap, now));
    let head = Item {
        title: format!("{} · {title}", s.state),
        ..Item { id: ItemId::new(ID, "head"), verb: "Show Fleet", ..s.item() }
    };
    let mut items = vec![head];
    let facts = snap.as_ref().map(report::facts).unwrap_or_default();
    for (n, (label, text)) in facts.into_iter().enumerate() {
        let item = Item::new(ItemId::new(ID, format!("fact/{n}")), text, "Show Fleet", fact_icon(label));
        items.push(Item { subtitle: label.into(), keywords: vec![label.into()], ..item });
    }
    for svc in &s.services {
        let item = Item { id: ItemId::new(ID, format!("check/{}", svc.name)), title: svc.name.into(), verb: "Show Status", ..s.service_item(*svc) };
        items.push(item);
    }
    items.push(agents_item());
    let via = s.row.source.unwrap_or(s.slot.machine.via).as_str();
    (items, format!("{} · {} via {via}  ·  esc to go back", s.name(), s.state))
}

/// The status line for Enter on service `name` of `seen`.
pub fn check_status(seen: &Seen, name: &str) -> Option<String> {
    let svc = seen.services.iter().find(|s| s.name == name)?;
    Some(format!("{}: {} · {}", svc.name, svc.status, svc.reason))
}

#[cfg(test)]
mod tests;
