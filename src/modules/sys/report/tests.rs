use super::*;
use crate::modules::sys::probe::{Battery, Thermal, parse};
use crate::modules::sys::testkit::SERVER;

#[test]
fn snapshot_json_is_the_peer_contract() {
    let snap = parse(SERVER, 1_000);
    let v = snapshot_json(&snap, None, 1_004);
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
    });
    assert_eq!(v, want);
    let empty = snapshot_json(&Snapshot::default(), Some("probe: timed out"), 0);
    assert_eq!(empty["error"], "probe: timed out");
    assert_eq!(empty["battery"], Value::Null);
    assert_eq!(empty["disks"], serde_json::json!([]));
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
                thermal  nominal";
    assert_eq!(snapshot_text(&snap, None, 1_004), want);
}

#[test]
fn snapshot_text_skips_what_the_probe_missed() {
    let bare = Snapshot { at: 100, ..Snapshot::default() };
    assert_eq!(snapshot_text(&bare, Some("probe: no output"), 100), "this Mac · read 0s ago\nbattery  none\nlast probe failed: probe: no output");
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
    assert_eq!(snapshot_text(&hot, None, 60), want);
}

#[test]
fn durations() {
    assert_eq!([0, 59, 60, 3_599, 3_600, 86_399, 86_400].map(ago), ["0s", "59s", "1m", "59m", "1h", "23h", "1d"]);
    assert_eq!([59, 60, 3_660, 90_000].map(span), ["0m", "1m", "1h 1m", "1d 1h"]);
}
