//! The probe: one fixed shell script of stock macOS tools, and its pure parser. The same
//! script serves this Mac (`/bin/sh -c PROBE`) and, later, a machine reached over ssh
//! (`/bin/sh -s` with the script on stdin), so a Mac without Flick needs nothing installed.
//!
//! The script prints sections, each after a `@@ <name>` line. Every command's stderr is
//! dropped and a failing command leaves its section empty, so the parser never errors: a
//! field it cannot read (a Mac without a battery, a key an older macOS lacks, output a
//! newer one reshaped) is `None`. `sysctl` prints `name: value` (not `-n`), so an unknown
//! key on one macOS version cannot shift the others.

use serde::Serialize;

/// The probe script. Fixed: nothing from config or a request is ever added to it.
pub const PROBE: &str = "echo '@@ host'
/usr/sbin/scutil --get LocalHostName 2>/dev/null || /bin/hostname -s 2>/dev/null
echo '@@ sysctl'
/usr/sbin/sysctl hw.ncpu hw.memsize kern.memorystatus_level vm.loadavg kern.boottime 2>/dev/null
echo '@@ df'
/bin/df -kP / /System/Volumes/Data 2>/dev/null
echo '@@ batt'
/usr/bin/pmset -g batt 2>/dev/null
echo '@@ therm'
/usr/bin/pmset -g therm 2>/dev/null
echo '@@ now'
/bin/date +%s
";

/// One machine's health at one moment. Every field the probe could not read is `None`
/// (an empty list for `disks`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Snapshot {
    /// The machine's local host name (`scutil --get LocalHostName`, else `hostname -s`).
    pub host: Option<String>,
    /// Load averages over 1, 5 and 15 minutes.
    pub cpu_load: Option<[f64; 3]>,
    /// Logical CPUs.
    pub ncpu: Option<u32>,
    /// Physical memory, bytes.
    pub mem_total: Option<u64>,
    /// Free memory as macOS rates it for memory pressure (`kern.memorystatus_level`), 0-100.
    pub mem_free_pct: Option<u8>,
    pub disks: Vec<Disk>,
    /// `None` on a Mac without an internal battery.
    pub battery: Option<Battery>,
    pub thermal: Option<Thermal>,
    pub uptime_secs: Option<u64>,
    /// When the probe ran, unix seconds on the collecting Flick's clock.
    pub at: u64,
}

/// One `df` line. Sizes in bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Disk {
    pub mount: String,
    pub device: String,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
    pub used_pct: Option<u8>,
}

/// The internal battery (`pmset -g batt`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Battery {
    pub percent: Option<u8>,
    /// As pmset says it: `charged`, `charging`, `discharging`, `finishing charge`, `AC
    /// attached`, ...
    pub state: Option<String>,
    /// `AC Power`, `Battery Power` or `UPS Power`.
    pub source: Option<String>,
    /// Minutes to empty or full; `None` while macOS has no estimate.
    pub remaining_mins: Option<u32>,
}

/// Thermal state (`pmset -g therm`). A level macOS has not recorded is `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Thermal {
    /// No warning level above 0 and no CPU speed limit below 100.
    pub nominal: bool,
    pub warning_level: Option<u32>,
    pub performance_level: Option<u32>,
    /// Percent of full CPU speed allowed.
    pub cpu_speed_limit: Option<u32>,
}

/// The probe's output as a snapshot taken at `at`. Never fails; see the module comment.
pub fn parse(text: &str, at: u64) -> Snapshot {
    let s = Sections::new(text);
    let sysctl = |key: &str| {
        s.lines("sysctl").find_map(|l| l.strip_prefix(key)?.strip_prefix(':').map(str::trim))
    };
    let num = |key: &str| sysctl(key).and_then(|v| v.parse::<u64>().ok());
    let now = s.lines("now").find_map(|l| l.trim().parse::<u64>().ok()).unwrap_or(at);
    let boot = sysctl("kern.boottime").and_then(boot_secs);
    Snapshot {
        host: s.lines("host").map(str::trim).find(|l| !l.is_empty()).map(str::to_string),
        cpu_load: sysctl("vm.loadavg").and_then(loadavg),
        ncpu: num("hw.ncpu").and_then(|n| u32::try_from(n).ok()),
        mem_total: num("hw.memsize"),
        mem_free_pct: num("kern.memorystatus_level").and_then(|n| u8::try_from(n).ok()).filter(|n| *n <= 100),
        disks: disks(s.lines("df")),
        battery: battery(s.lines("batt")),
        thermal: thermal(s.lines("therm")),
        uptime_secs: boot.and_then(|b| now.checked_sub(b)),
        at,
    }
}

/// The text split at `@@ <name>` lines.
struct Sections<'a>(Vec<(&'a str, Vec<&'a str>)>);

impl<'a> Sections<'a> {
    fn new(text: &'a str) -> Self {
        let mut out: Vec<(&str, Vec<&str>)> = vec![];
        for line in text.lines() {
            if let Some(name) = line.strip_prefix("@@ ") {
                out.push((name.trim(), vec![]));
            } else if let Some((_, lines)) = out.last_mut() {
                lines.push(line);
            }
        }
        Sections(out)
    }

    fn lines<'b>(&'b self, name: &'b str) -> impl Iterator<Item = &'a str> + 'b {
        self.0.iter().filter(move |(n, _)| *n == name).flat_map(|(_, l)| l.iter().copied())
    }
}

/// `{ 1.42 2.06 2.42 }`.
fn loadavg(v: &str) -> Option<[f64; 3]> {
    let n: Vec<f64> = v
        .trim_matches(|c| c == '{' || c == '}' || char::is_whitespace(c))
        .split_whitespace()
        .map(|w| w.parse().ok())
        .collect::<Option<_>>()?;
    n.try_into().ok()
}

/// `{ sec = 1791307644, usec = 544610 } Tue Oct  6 10:27:24 2026`.
fn boot_secs(v: &str) -> Option<u64> {
    let part = v.split(['{', ',', '}']).find_map(|p| p.trim().strip_prefix("sec = "))?;
    part.trim().parse().ok()
}

/// `df -kP` lines after the header: device, 1024-blocks, used, available, capacity, mount.
fn disks<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<Disk> {
    let mut out: Vec<Disk> = vec![];
    for line in lines {
        let w: Vec<&str> = line.split_whitespace().collect();
        let [device, total, used, avail, cap, mount @ ..] = w.as_slice() else { continue };
        let kib = |s: &str| s.parse::<u64>().ok().map(|k| k.saturating_mul(1024));
        let (Some(total), Some(used), Some(avail)) = (kib(total), kib(used), kib(avail)) else {
            continue;
        };
        let mount = mount.join(" ");
        if mount.is_empty() || out.iter().any(|d| d.mount == mount) {
            continue;
        }
        let used_pct = cap.strip_suffix('%').and_then(|p| p.parse().ok());
        out.push(Disk { mount, device: (*device).to_string(), total, used, avail, used_pct });
    }
    out
}

/// `pmset -g batt`: `Now drawing from 'AC Power'`, then per battery
/// ` -InternalBattery-0 (id=…)\t85%; discharging; 4:12 remaining present: true`.
fn battery<'a>(lines: impl Iterator<Item = &'a str>) -> Option<Battery> {
    let mut source = None;
    let mut found = None;
    for line in lines {
        if let Some(rest) = line.strip_prefix("Now drawing from ") {
            source = Some(rest.trim().trim_matches('\'').to_string());
        } else if line.trim_start().starts_with("-InternalBattery") {
            found = Some(line);
        }
    }
    let line = found?;
    let fields = line.split_once('\t').map_or(line, |(_, f)| f);
    let mut parts = fields.split(';').map(str::trim);
    let percent = parts.next().and_then(|p| p.strip_suffix('%')).and_then(|p| p.parse().ok());
    let state = parts.next().filter(|s| !s.is_empty()).map(str::to_string);
    let remaining_mins = parts.next().and_then(remaining);
    Some(Battery { percent, state, source, remaining_mins })
}

/// `4:12 remaining present: true` as minutes; `(no estimate)` is `None`.
fn remaining(v: &str) -> Option<u32> {
    let clock = v.split_whitespace().next()?;
    let (h, m) = clock.split_once(':')?;
    Some(h.parse::<u32>().ok()? * 60 + m.parse::<u32>().ok()?)
}

/// `pmset -g therm`: notes when a level was never recorded, else `... level set to N` lines
/// and a `CPU_Speed_Limit = N` line. `None` when no line is recognised.
fn thermal<'a>(lines: impl Iterator<Item = &'a str>) -> Option<Thermal> {
    let mut t = Thermal::default();
    let mut seen = false;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        let level = || lower.split(|c: char| !c.is_ascii_digit()).find(|w| !w.is_empty())?.parse().ok();
        if lower.contains("thermal warning level") {
            seen = true;
            t.warning_level = level();
        } else if lower.contains("performance warning level") {
            seen = true;
            t.performance_level = level();
        } else if lower.contains("cpu power status") {
            seen = true;
        } else if lower.trim_start().starts_with("cpu_speed_limit") {
            seen = true;
            t.cpu_speed_limit = level();
        }
    }
    let raised = |l: Option<u32>| l.is_some_and(|l| l > 0);
    t.nominal = !raised(t.warning_level)
        && !raised(t.performance_level)
        && t.cpu_speed_limit.is_none_or(|l| l >= 100);
    seen.then_some(t)
}

#[cfg(test)]
mod tests;
