//! `flick sys snapshot` answers, as text and as JSON. Pure. The JSON is the peer
//! contract: the fleet view on another Mac reads it over `flick --host <mac> --json`, so
//! fields are only added, never renamed (`SCHEMA` counts breaking changes).

use serde_json::Value;

use super::probe::Snapshot;

/// Bumped on a breaking change to the JSON below.
pub const SCHEMA: u32 = 1;

/// `{"schema", <Snapshot fields>, "age_secs", "error"}`.
pub fn snapshot_json(snap: &Snapshot, error: Option<&str>, now: u64) -> Value {
    let mut v = serde_json::to_value(snap).unwrap_or(Value::Null);
    v["schema"] = SCHEMA.into();
    v["age_secs"] = now.saturating_sub(snap.at).into();
    v["error"] = error.into();
    v
}

/// One line per fact the probe read, then the probe's error.
pub fn snapshot_text(snap: &Snapshot, error: Option<&str>, now: u64) -> String {
    let mut head = vec![snap.host.clone().unwrap_or_else(|| "this Mac".into())];
    if let Some(up) = snap.uptime_secs {
        head.push(format!("up {}", span(up)));
    }
    head.push(format!("read {} ago", ago(now.saturating_sub(snap.at))));
    let mut lines = vec![head.join(" · ")];
    let mut row = |label: &str, text: String| lines.push(format!("{label:<9}{text}"));
    if let Some([a, b, c]) = snap.cpu_load {
        let cpus = snap.ncpu.map_or(String::new(), |n| format!(" · {n} CPUs"));
        row("load", format!("{a:.2} {b:.2} {c:.2}{cpus}"));
    }
    let mem = [snap.mem_total.map(bytes), snap.mem_free_pct.map(|p| format!("{p}% free"))];
    let mem: Vec<String> = mem.into_iter().flatten().collect();
    if !mem.is_empty() {
        row("memory", mem.join(" · "));
    }
    for d in &snap.disks {
        let used = d.used_pct.map_or(String::new(), |p| format!(" {p}% used ·"));
        row("disk", format!("{}{used} {} free", d.mount, bytes(d.avail)));
    }
    row("battery", snap.battery.as_ref().map_or("none".into(), |b| {
        let parts = [
            b.percent.map(|p| format!("{p}%")),
            b.state.clone(),
            b.source.clone(),
            b.remaining_mins.filter(|m| *m > 0).map(|m| format!("{} left", span(u64::from(m) * 60))),
        ];
        parts.into_iter().flatten().collect::<Vec<_>>().join(" · ")
    }));
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
        row("thermal", text);
    }
    if let Some(e) = error {
        lines.push(format!("last probe failed: {e}"));
    }
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
