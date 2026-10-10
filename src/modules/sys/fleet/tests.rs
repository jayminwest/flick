use super::*;
use crate::config::parse;
use crate::modules::sys::settings::Settings;

fn machines(text: &str) -> Vec<Machine> {
    parse(text).unwrap().section("sys").unwrap().unwrap().get::<Settings>().unwrap().check().unwrap().machine
}

const THREE: &str = r#"
[[sys.machine]]
name = "laptop"
via = "local"
[[sys.machine]]
name = "server"
via = "flick"
vnc = "vnc://server"
[[sys.machine]]
name = "pro"
via = "ssh"
ssh = "jaymin@pro"
"#;

fn fleet() -> Fleet {
    let mut f = Fleet::default();
    f.set_machines(&machines(THREE));
    f
}

#[expect(clippy::unnecessary_wraps, reason = "stands in for a fetch result")]
fn got(host: &str) -> Result<Fetched, String> {
    let snapshot = json!({ "host": host, "cpu_load": [1.5, 1.0, 0.5], "mem_free_pct": 40,
        "disks": [{ "used_pct": 61 }, { "used_pct": 12 }], "battery": { "percent": 80 },
        "services": [{ "name": "web", "status": "ok", "reason": "HTTP 200" },
                     { "name": "agent", "status": "fail", "reason": "not loaded" }] });
    Ok(Fetched { snapshot, source: Via::Flick, note: None })
}

fn no_local() -> Local {
    Local { snapshot: None, at: None, error: None }
}

#[test]
fn due_machines_are_the_remote_ones_not_tried_lately() {
    let mut f = fleet();
    let names = |d: Vec<(usize, Machine)>| d.into_iter().map(|(i, m)| format!("{i}:{}", m.name)).collect::<Vec<_>>();
    assert!(f.never_tried() && f.has_local());
    assert_eq!(names(f.due(100, 15)), ["1:server", "2:pro"]);
    assert!(f.apply(f.epoch, 1, &f.slots[1].machine.clone(), got("server"), 100));
    assert!(!f.never_tried());
    assert_eq!(names(f.due(110, 15)), ["2:pro"]);
    assert_eq!(names(f.due(115, 15)), ["1:server", "2:pro"]);
    assert_eq!(names(f.due(100, 0)), ["1:server", "2:pro"]);
}

#[test]
fn results_of_a_dropped_round_or_a_changed_machine_are_not_applied() {
    let mut f = fleet();
    let (epoch, server) = (f.epoch, f.slots[1].machine.clone());
    f.busy = true;
    // Sleep (or a reload) drops the round in flight.
    f.forget_round();
    assert!(!f.busy);
    assert!(!f.apply(epoch, 1, &server, got("server"), 100));
    assert_eq!(f.slots[1].snapshot, None);
    // A machine at that index that is not the one asked about.
    assert!(!f.apply(f.epoch, 2, &server, got("server"), 100));
    assert!(!f.apply(f.epoch, 9, &server, got("server"), 100));
    assert!(f.apply(f.epoch, 1, &server, got("server"), 100));
}

#[test]
fn a_failure_keeps_the_last_snapshot_and_a_reload_keeps_unchanged_machines() {
    let mut f = fleet();
    let server = f.slots[1].machine.clone();
    f.apply(f.epoch, 1, &server, got("server"), 100);
    f.apply(f.epoch, 1, &server, Err("can't reach".into()), 130);
    let s = &f.slots[1];
    assert_eq!((s.fetched_at, s.tried_at, s.error.as_deref()), (Some(100), Some(130), Some("can't reach")));
    assert!(s.snapshot.is_some());
    let epoch = f.epoch;
    // The same server stays; a changed pro starts over.
    f.set_machines(&machines(&THREE.replace("jaymin@pro", "admin@pro")));
    assert!(f.epoch > epoch);
    assert_eq!(f.slots[1].fetched_at, Some(100));
    f.slots[2].tried_at = Some(1);
    f.set_machines(&machines(THREE));
    assert_eq!(f.slots[2].tried_at, None);
    f.set_machines(&[]);
    assert!(f.slots.is_empty() && f.never_tried() && !f.has_local());
}

#[test]
fn json_has_every_machine_with_its_state() {
    let mut f = fleet();
    let server = f.slots[1].machine.clone();
    let pro = f.slots[2].machine.clone();
    f.apply(f.epoch, 1, &server, got("server"), 100);
    f.apply(f.epoch, 2, &pro, Err("ssh: timed out".into()), 100);
    let local = Local { snapshot: Some(json!({ "host": "laptop" })), at: Some(90), error: None };
    let v = fleet_json(&f, &local, 110, stale_after(0));
    assert_eq!((v["schema"].as_u64(), v["stale_after_secs"].as_u64()), (Some(1), Some(45)));
    let m = &v["machines"];
    assert_eq!(m[0], json!({
        "name": "laptop", "via": "local", "host": null, "ssh": null, "vnc": null, "dash": null,
        "source": "local", "state": "fresh", "stale": false, "fetched_at": 90, "age_secs": 20,
        "error": null, "note": null, "snapshot": { "host": "laptop" },
    }));
    assert_eq!((m[1]["host"].as_str(), m[1]["vnc"].as_str(), m[1]["source"].as_str()), (Some("server"), Some("vnc://server"), Some("flick")));
    assert_eq!(m[1]["snapshot"]["host"], "server");
    assert_eq!((m[2]["state"].as_str(), m[2]["error"].as_str(), m[2]["ssh"].as_str()), (Some("down"), Some("ssh: timed out"), Some("jaymin@pro")));
    // Old data is stale; a failure after good data is stale too; nothing yet is pending.
    let states = |v: &Value| v["machines"].as_array().unwrap().iter().map(|m| m["state"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(states(&fleet_json(&f, &local, 200, 45)), ["stale", "stale", "down"]);
    f.apply(f.epoch, 1, &server, Err("gone".into()), 201);
    assert_eq!(states(&fleet_json(&f, &no_local(), 202, 3600)), ["pending", "stale", "down"]);
    let failed = Local { snapshot: None, at: None, error: Some("probe: no output".into()) };
    assert_eq!(states(&fleet_json(&f, &failed, 202, 3600))[0], "down");
}

#[test]
fn text_has_a_line_per_machine_and_its_trouble() {
    let mut f = fleet();
    let server = f.slots[1].machine.clone();
    f.apply(f.epoch, 1, &server, got("server").map(|g| Fetched { source: Via::Ssh, note: Some("flick too old (no sys snapshot)".into()), ..g }), 100);
    let pro = f.slots[2].machine.clone();
    f.apply(f.epoch, 2, &pro, Err("ssh: timed out".into()), 100);
    let want = "pending  laptop  local\n\
                fresh    server  ssh    load 1.50 · mem 40% free · disk 61% · batt 80% · 1 ok · 1 fail · 10s ago\n\
                \x20        note: flick too old (no sys snapshot)\n\
                \x20        fail     agent  not loaded\n\
                down     pro     ssh\n\
                \x20        error: ssh: timed out";
    assert_eq!(fleet_text(&f, &no_local(), 110, 45), want);
    assert_eq!(fleet_text(&Fleet::default(), &no_local(), 0, 45), "no machines (add [[sys.machine]] tables to config.toml)");
    // A snapshot without any of the fields shows no metrics.
    let bare = Local { snapshot: Some(json!({})), at: Some(110), error: None };
    assert!(fleet_text(&f, &bare, 110, 45).starts_with("fresh    laptop  local  0s ago\n"));
}

#[test]
fn staleness_is_three_cadences() {
    assert_eq!(stale_after(0), 45);
    assert_eq!(stale_after(60), 180);
}
