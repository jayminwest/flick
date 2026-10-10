//! `flick sys snapshot|services` answers, as text and as JSON. Pure. The JSON is the peer
//! contract: the fleet view on another Mac reads it over `flick --host <mac> --json`, so
//! fields are only added, never renamed (`SCHEMA` counts breaking changes).

use serde_json::{Value, json};

use super::check::Status;
use super::io::Entry;
use super::probe::Snapshot;

/// Bumped on a breaking change to the JSON below.
pub const SCHEMA: u32 = 1;

/// `{"schema", <Snapshot fields>, "age_secs", "error", "services": [<service>]}`.
pub fn snapshot_json(snap: &Snapshot, error: Option<&str>, services: &[Entry], now: u64) -> Value {
    let mut v = serde_json::to_value(snap).unwrap_or(Value::Null);
    v["schema"] = SCHEMA.into();
    v["age_secs"] = now.saturating_sub(snap.at).into();
    v["error"] = error.into();
    v["services"] = services.iter().map(|e| service_json(e, now)).collect();
    v
}

/// `{"schema", "services": [<service>]}`.
pub fn services_json(services: &[Entry], now: u64) -> Value {
    let list: Vec<Value> = services.iter().map(|e| service_json(e, now)).collect();
    json!({ "schema": SCHEMA, "services": list })
}

/// One service: its config (a command's target is only its program) and last verdict;
/// `unknown` with null times before the first check.
fn service_json(e: &Entry, now: u64) -> Value {
    let (status, reason, value) = verdict(e);
    json!({
        "name": e.service.name,
        "kind": e.service.kind.as_str(),
        "target": e.service.shown_target(),
        "status": status.as_str(),
        "reason": reason,
        "value": value,
        "checked_at": e.checked_at,
        "age_secs": e.checked_at.map(|t| now.saturating_sub(t)),
        "log": e.service.log,
        "restart": e.service.restart,
    })
}

fn verdict(e: &Entry) -> (Status, &str, Option<f64>) {
    match &e.verdict {
        Some(v) => (v.status, v.reason.as_str(), v.value),
        None => (Status::Unknown, "not checked yet", None),
    }
}

/// One line per fact the probe read, then a services summary and the probe's error.
pub fn snapshot_text(snap: &Snapshot, error: Option<&str>, services: &[Entry], now: u64) -> String {
    let mut lines = vec![head(snap, now)];
    for (label, text) in facts(snap) {
        lines.push(format!("{label:<9}{text}"));
    }
    if !services.is_empty() {
        lines.push(format!("{:<9}{}", "services", summary(services)));
    }
    if let Some(e) = error {
        lines.push(format!("last probe failed: {e}"));
    }
    lines.join("\n")
}

/// `mbp-server · up 3d 13h · read 4s ago`.
pub fn head(snap: &Snapshot, now: u64) -> String {
    let mut head = vec![snap.host.clone().unwrap_or_else(|| "this Mac".into())];
    if let Some(up) = snap.uptime_secs {
        head.push(format!("up {}", span(up)));
    }
    head.push(format!("read {} ago", ago(now.saturating_sub(snap.at))));
    head.join(" · ")
}

/// Each fact the probe read as (label, text): load, memory, each disk, battery, thermal.
pub fn facts(snap: &Snapshot) -> Vec<(&'static str, String)> {
    let mut rows = vec![];
    if let Some([a, b, c]) = snap.cpu_load {
        let cpus = snap.ncpu.map_or(String::new(), |n| format!(" · {n} CPUs"));
        rows.push(("load", format!("{a:.2} {b:.2} {c:.2}{cpus}")));
    }
    let mem = [snap.mem_total.map(bytes), snap.mem_free_pct.map(|p| format!("{p}% free"))];
    let mem: Vec<String> = mem.into_iter().flatten().collect();
    if !mem.is_empty() {
        rows.push(("memory", mem.join(" · ")));
    }
    for d in &snap.disks {
        let used = d.used_pct.map_or(String::new(), |p| format!(" {p}% used ·"));
        rows.push(("disk", format!("{}{used} {} free", d.mount, bytes(d.avail))));
    }
    rows.push(("battery", snap.battery.as_ref().map_or("none".into(), |b| {
        let parts = [
            b.percent.map(|p| format!("{p}%")),
            b.state.clone(),
            b.source.clone(),
            b.remaining_mins.filter(|m| *m > 0).map(|m| format!("{} left", span(u64::from(m) * 60))),
        ];
        parts.into_iter().flatten().collect::<Vec<_>>().join(" · ")
    })));
    if let Some(t) = &snap.thermal {
        let text = if t.nominal {
            "nominal".to_string()
        } else {
            let parts = [
                t.warning_level.map(|l| format!("warning level {l}")),
                t.performance_level.map(|l| format!("performance level {l}")),
                t.cpu_speed_limit.map(|l| format!("CPU speed {l}%")),
            ];
            parts.into_iter().flatten().collect::<Vec<_>>().join(" · ")
        };
        rows.push(("thermal", text));
    }
    rows
}

/// `3 ok · 1 fail`, in status order, skipping zero counts.
pub fn summary(services: &[Entry]) -> String {
    let all = [Status::Ok, Status::Warn, Status::Fail, Status::Unknown];
    let count = |s: Status| services.iter().filter(|e| verdict(e).0 == s).count();
    let parts: Vec<String> =
        all.into_iter().map(|s| (count(s), s)).filter(|(n, _)| *n > 0).map(|(n, s)| format!("{n} {}", s.as_str())).collect();
    parts.join(" · ")
}

/// One line per service: status, name, kind, reason and the verdict's age.
pub fn services_text(services: &[Entry], now: u64) -> String {
    if services.is_empty() {
        return "no services (add [[sys.service]] tables to config.toml)".into();
    }
    let width = services.iter().map(|e| e.service.name.chars().count()).max().unwrap_or(0);
    let lines: Vec<String> = services
        .iter()
        .map(|e| {
            let (status, reason, _) = verdict(e);
            let age = e.checked_at.map_or(String::new(), |t| format!(" · {} ago", ago(now.saturating_sub(t))));
            let (name, kind) = (&e.service.name, e.service.kind.as_str());
            format!("{:<8} {name:<width$}  {kind:<8} {reason}{age}", status.as_str())
        })
        .collect();
    lines.join("\n")
}

/// `45s`, `3m`, `2h`, `4d`.
pub fn ago(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// `3d 4h`, `2h 5m`, `4m`.
fn span(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

/// Binary units with one decimal: `18.0 GiB`, `512.0 MiB`.
fn bytes(n: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let n = n as f64;
    if n >= GIB { format!("{:.1} GiB", n / GIB) } else { format!("{:.1} MiB", n / 1024.0 / 1024.0) }
}

#[cfg(test)]
mod tests;
