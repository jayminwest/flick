use super::*;
use crate::modules::sys::check::Verdict;
use crate::modules::sys::probe::{Battery, Thermal, parse};
use crate::modules::sys::settings::{Domain, Kind, Service, Target};
use crate::modules::sys::testkit::SERVER;

fn entry(name: &str, kind: Kind, target: Target, verdict: Option<(Status, &str, Option<f64>)>) -> Entry {
    let service = Service { name: name.into(), kind, target, warn: None, fail: None, log: None, restart: false, domain: Domain::Gui };
    let verdict = verdict.map(|(status, reason, value)| Verdict { status, reason: reason.into(), value });
    let checked_at = verdict.as_ref().map(|_| 990);
    Entry { service, verdict, checked_at }
}

fn entries() -> Vec<Entry> {
    let mut life = entry("life-ops", Kind::Launchd, Target::One("org.example.life-ops".into()), Some((Status::Ok, "running (pid 919)", None)));
    life.service.log = Some("~/Library/Logs/life-ops.log".into());
    life.service.restart = true;
    vec![
        entry("memory", Kind::Http, Target::One("http://127.0.0.1:8300/health".into()), Some((Status::Warn, "HTTP 200 in 250 ms (warn >= 100)", Some(250.0)))),
        life,
        entry("backlog", Kind::Command, Target::Argv(vec!["/bin/sh".into(), "-c".into(), "secret".into()]), None),
    ]
}

#[test]
fn snapshot_json_is_the_peer_contract() {
    let snap = parse(SERVER, 1_000);
    let v = snapshot_json(&snap, None, &entries(), 1_004);
    let want = serde_json::json!({
        "schema": 1,
        "host": "mbp-server",
        "cpu_load": [1.42, 2.06, 2.42],
        "ncpu": 11,
        "mem_total": 19_327_352_832_u64,
        "mem_free_pct": 73,
        "disks": [
            { "mount": "/", "device": "/dev/disk3s3s1", "total": 994_662_584_320_u64, "used": 13_654_192_128_u64, "avail": 876_673_978_368_u64, "used_pct": 2 },
            { "mount": "/System/Volumes/Data", "device": "/dev/disk3s1", "total": 994_662_584_320_u64, "used": 79_517_691_904_u64, "avail": 876_673_978_368_u64, "used_pct": 9 },
        ],
        "battery": { "percent": 100, "state": "charged", "source": "AC Power", "remaining_mins": 0 },
        "thermal": { "nominal": true, "warning_level": null, "performance_level": null, "cpu_speed_limit": null },
        "uptime_secs": 307_392,
        "at": 1_000,
        "age_secs": 4,
        "error": null,
        "services": [
            { "name": "memory", "kind": "http", "target": "http://127.0.0.1:8300/health", "status": "warn",
              "reason": "HTTP 200 in 250 ms (warn >= 100)", "value": 250.0, "checked_at": 990, "age_secs": 14,
              "log": null, "restart": false },
            { "name": "life-ops", "kind": "launchd", "target": "org.example.life-ops", "status": "ok",
              "reason": "running (pid 919)", "value": null, "checked_at": 990, "age_secs": 14,
              "log": "~/Library/Logs/life-ops.log", "restart": true },
            // A command's arguments never leave this Mac.
            { "name": "backlog", "kind": "command", "target": "/bin/sh", "status": "unknown",
              "reason": "not checked yet", "value": null, "checked_at": null, "age_secs": null,
              "log": null, "restart": false },
        ],
    });
    assert_eq!(v, want);
    let empty = snapshot_json(&Snapshot::default(), Some("probe: timed out"), &[], 0);
    assert_eq!(empty["error"], "probe: timed out");
    assert_eq!(empty["battery"], Value::Null);
    assert_eq!(empty["disks"], serde_json::json!([]));
}

#[test]
fn services_json_lists_every_service() {
    let v = services_json(&entries(), 1_000);
    assert_eq!(v["schema"], 1);
    let names: Vec<&str> = v["services"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["memory", "life-ops", "backlog"]);
    assert_eq!(services_json(&[], 0), serde_json::json!({ "schema": 1, "services": [] }));
}

#[test]
fn snapshot_text_reads_like_a_status_line() {
    let snap = parse(SERVER, 1_000);
    let want = "mbp-server · up 3d 13h · read 4s ago\n\
                load     1.42 2.06 2.42 · 11 CPUs\n\
                memory   18.0 GiB · 73% free\n\
                disk     / 2% used · 816.5 GiB free\n\
                disk     /System/Volumes/Data 9% used · 816.5 GiB free\n\
                battery  100% · charged · AC Power\n\
                thermal  nominal\n\
                services 1 ok · 1 warn · 1 unknown";
    assert_eq!(snapshot_text(&snap, None, &entries(), 1_004), want);
}

#[test]
fn snapshot_text_skips_what_the_probe_missed() {
    let bare = Snapshot { at: 100, ..Snapshot::default() };
    assert_eq!(snapshot_text(&bare, Some("probe: no output"), &[], 100), "this Mac · read 0s ago\nbattery  none\nlast probe failed: probe: no output");
    let hot = Snapshot {
        host: Some("mac-pro".into()),
        mem_free_pct: Some(40),
        uptime_secs: Some(7_500),
        battery: Some(Battery { percent: Some(85), state: Some("discharging".into()), source: None, remaining_mins: Some(252) }),
        thermal: Some(Thermal { nominal: false, warning_level: Some(1), performance_level: Some(2), cpu_speed_limit: Some(80) }),
        cpu_load: Some([1.0, 0.5, 0.25]),
        disks: vec![crate::modules::sys::probe::Disk { mount: "/".into(), device: "d".into(), total: 1, used: 1, avail: 512 * 1024 * 1024, used_pct: None }],
        ..Snapshot::default()
    };
    let want = "mac-pro · up 2h 5m · read 1m ago\n\
                load     1.00 0.50 0.25\n\
                memory   40% free\n\
                disk     / 512.0 MiB free\n\
                battery  85% · discharging · 4h 12m left\n\
                thermal  warning level 1 · performance level 2 · CPU speed 80%";
    assert_eq!(snapshot_text(&hot, None, &[], 60), want);
}

#[test]
fn services_text_lines_up() {
    let want = "warn     memory    http     HTTP 200 in 250 ms (warn >= 100) · 10s ago\n\
                ok       life-ops  launchd  running (pid 919) · 10s ago\n\
                unknown  backlog   command  not checked yet";
    assert_eq!(services_text(&entries(), 1_000), want);
    assert_eq!(services_text(&[], 0), "no services (add [[sys.service]] tables to config.toml)");
    let failed = entry("x", Kind::Tcp, Target::One("h:1".into()), Some((Status::Fail, "refused", None)));
    assert_eq!(summary(&[failed]), "1 fail");
}

#[test]
fn durations() {
    assert_eq!([0, 59, 60, 3_599, 3_600, 86_399, 86_400].map(ago), ["0s", "59s", "1m", "59m", "1h", "23h", "1d"]);
    assert_eq!([59, 60, 3_660, 90_000].map(span), ["0m", "1m", "1h 1m", "1d 1h"]);
}
