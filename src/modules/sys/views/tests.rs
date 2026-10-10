use serde_json::json;

use super::*;
use crate::config::parse;
use crate::modules::sys::fleet::Fetched;
use crate::modules::sys::settings::{Settings, Via};
use crate::modules::sys::{probe, testkit};

const THREE: &str = r#"
[[sys.machine]]
name = "laptop"
via = "local"
[[sys.machine]]
name = "server"
via = "flick"
[[sys.machine]]
name = "pro"
via = "ssh"
ssh = "jaymin@pro"
"#;

fn fleet(text: &str) -> Fleet {
    let settings = parse(text).unwrap().section("sys").unwrap().unwrap().get::<Settings>().unwrap().check().unwrap();
    let mut f = Fleet::default();
    f.set_machines(&settings.machine);
    f
}

fn snap() -> Value {
    json!({ "host": "server", "at": 90, "cpu_load": [1.5, 1.0, 0.5], "mem_free_pct": 40,
        "services": [{ "name": "web", "kind": "http", "status": "ok", "reason": "HTTP 200" },
                     { "name": "agent", "kind": "launchd", "status": "fail", "reason": "not loaded" }] })
}

fn store(f: &mut Fleet, i: usize, got: Result<Value, String>, source: Via, note: Option<&str>, now: u64) {
    let machine = f.slots[i].machine.clone();
    let got = got.map(|snapshot| Fetched { snapshot, source, note: note.map(str::to_string) });
    assert!(f.apply(f.epoch, i, &machine, got, now));
}

fn no_local() -> Local {
    Local { snapshot: None, at: None, error: None }
}

/// laptop pending, server fresh (1 fail), pro down.
fn mixed() -> Fleet {
    let mut f = fleet(THREE);
    store(&mut f, 1, Ok(snap()), Via::Flick, None, 100);
    store(&mut f, 2, Err("ssh: timed out".into()), Via::Ssh, None, 100);
    f
}

fn titles(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.title.as_str()).collect()
}

#[test]
fn keys_parse_back() {
    let f = fleet(&THREE.replace("\"pro\"", "\"pro/2\"").replace("\"server\"", "\"pro\""));
    assert_eq!(Key::parse("fleet", &f), Some(Key::Fleet));
    assert_eq!(Key::parse("head", &f), Some(Key::Head));
    assert_eq!(Key::parse("agents", &f), Some(Key::Agents));
    assert_eq!(Key::parse("fact/3", &f), Some(Key::Fact));
    assert_eq!(Key::parse("machine/a/b", &f), Some(Key::Machine("a/b")));
    assert_eq!(Key::parse("check/web", &f), Some(Key::Check("web")));
    // The longest machine name that fits wins.
    assert_eq!(Key::parse("service/pro/2/web", &f), Some(Key::Service { machine: "pro/2", service: "web" }));
    assert_eq!(Key::parse("service/pro/web", &f), Some(Key::Service { machine: "pro", service: "web" }));
    assert_eq!(Key::parse("service/gone/web", &f), None);
    assert_eq!(Key::parse("service/laptopx", &f), None);
    assert_eq!(Key::parse("other", &f), None);
}

#[test]
fn services_read_what_the_snapshot_has() {
    assert!(services(None).is_empty());
    assert!(services(Some(&json!({}))).is_empty());
    let v = json!({ "services": [{}, { "name": "web", "kind": "http", "status": "warn", "reason": "slow" }] });
    let got = services(Some(&v));
    assert_eq!(got[0], Svc { name: "", kind: "", status: "unknown", reason: "" });
    assert_eq!(got[1], Svc { name: "web", kind: "http", status: "warn", reason: "slow" });
}

#[test]
fn icons_follow_state_then_the_worst_service() {
    let svc = |status| Svc { name: "s", kind: "tcp", status, reason: "" };
    assert_eq!(status_icon("ok"), Icon::Symbol("checkmark.circle"));
    assert_eq!(status_icon("warn"), Icon::Symbol("exclamationmark.triangle"));
    assert_eq!(status_icon("fail"), Icon::Symbol("xmark.octagon"));
    assert_eq!(status_icon("odd"), Icon::Symbol("questionmark.circle"));
    assert_eq!(machine_icon("pending", &[]), Icon::Symbol("hourglass"));
    assert_eq!(machine_icon("down", &[]), Icon::Symbol("bolt.horizontal.circle"));
    assert_eq!(machine_icon("stale", &[svc("fail")]), Icon::Symbol("clock.arrow.circlepath"));
    assert_eq!(machine_icon("fresh", &[]), status_icon("ok"));
    assert_eq!(machine_icon("fresh", &[svc("ok"), svc("unknown")]), status_icon("unknown"));
    assert_eq!(machine_icon("fresh", &[svc("unknown"), svc("warn")]), status_icon("warn"));
    assert_eq!(machine_icon("fresh", &[svc("warn"), svc("fail"), svc("ok")]), status_icon("fail"));
}

#[test]
fn machine_rows_show_state_metrics_and_age() {
    let f = mixed();
    let local = no_local();
    let seen = seen(&f, &local, 110, 45);
    let items: Vec<Item> = seen.iter().map(Seen::item).collect();
    assert_eq!(titles(&items), ["laptop", "server", "pro"]);
    assert_eq!(items[0].subtitle, "loading…");
    assert_eq!(items[0].accessory, "");
    assert_eq!(items[0].icon, Icon::Symbol("hourglass"));
    assert_eq!(items[1].subtitle, "load 1.50 · mem 40% free · 1 ok · 1 fail");
    assert_eq!(items[1].accessory, "10s ago");
    assert_eq!(items[1].icon, Icon::Symbol("xmark.octagon"));
    assert_eq!(items[1].id.as_str(), "sys:machine/server");
    assert_eq!(items[1].keywords, ["flick", "fresh", "machine"]);
    assert_eq!((items[2].subtitle.as_str(), items[2].verb), ("ssh: timed out", "Show Machine"));
    // A failure after good data keeps the data and says the last read failed; a fallback
    // says how the data was read.
    let mut f = mixed();
    store(&mut f, 1, Err("can't reach".into()), Via::Flick, None, 120);
    store(&mut f, 2, Ok(json!({})), Via::Ssh, Some("flick too old"), 120);
    let seen = super::seen(&f, &local, 130, 45);
    assert_eq!(seen[1].subtitle(), "load 1.50 · mem 40% free · 1 ok · 1 fail · last read failed: can't reach");
    assert_eq!(seen[1].item().icon, Icon::Symbol("clock.arrow.circlepath"));
    assert_eq!(seen[2].subtitle(), "via ssh: flick too old");
}

#[test]
fn root_item_shows_only_with_machines() {
    assert!(root_item(&[]).is_none());
    let f = mixed();
    let local = no_local();
    let item = root_item(&seen(&f, &local, 110, 45)).unwrap();
    assert_eq!((item.id.as_str(), item.title.as_str(), item.verb), ("sys:fleet", "Fleet", "Show Fleet"));
    assert_eq!(item.subtitle, "3 machines · 1 down · 1 fail");
    assert_eq!(item.icon, Icon::Symbol("server.rack"));
    let one = fleet("[[sys.machine]]\nname = \"a\"\nvia = \"local\"\n");
    let warm = Local { snapshot: Some(json!({ "services": [{ "status": "warn" }] })), at: Some(0), error: None };
    assert_eq!(summary_line(&seen(&one, &warm, 100, 45)), "1 machine · 1 stale · 1 warn");
}

#[test]
fn fleet_view_lists_machines_then_their_services_then_agents() {
    assert!(fleet_items(&[]).is_empty());
    let f = mixed();
    let local = no_local();
    let items = fleet_items(&seen(&f, &local, 110, 45));
    assert_eq!(titles(&items), ["laptop", "server", "web · server", "agent · server", "pro", "Herdr Agents"]);
    let agent = &items[3];
    assert_eq!(agent.id.as_str(), "sys:service/server/agent");
    assert_eq!((agent.subtitle.as_str(), agent.accessory.as_str()), ("fail · not loaded", "launchd"));
    assert_eq!(agent.icon, Icon::Symbol("xmark.octagon"));
    assert_eq!(agent.keywords, ["fail", "launchd", "not loaded"]);
    assert_eq!((items[5].id.as_str(), items[5].verb), ("sys:agents", "Show Agents"));
}

#[test]
fn machine_view_has_facts_and_services() {
    let (gone, footer) = machine_items(None, 0);
    assert!(gone.is_empty());
    assert_eq!(footer, "Machine gone (config changed)  ·  esc to go back");
    // This Mac's own snapshot, as `sys snapshot --json` renders it.
    let parsed = probe::parse(testkit::SERVER, 100);
    let mut f = fleet(THREE);
    let local = Local { snapshot: Some(crate::modules::sys::report::snapshot_json(&parsed, None, &[], 104)), at: Some(100), error: None };
    store(&mut f, 1, Ok(snap()), Via::Flick, None, 100);
    let seen = seen(&f, &local, 104, 45);
    let (items, footer) = machine_items(Some(&seen[0]), 104);
    assert_eq!(footer, "laptop · fresh via local  ·  esc to go back");
    assert_eq!(items[0].title, "fresh · mbp-server · up 3d 13h · read 4s ago");
    assert_eq!((items[0].id.as_str(), items[0].verb), ("sys:head", "Show Fleet"));
    let facts: Vec<(&str, &str)> = items[1..].iter().map(|i| (i.subtitle.as_str(), i.id.as_str())).collect();
    assert_eq!(facts[0], ("load", "sys:fact/0"));
    assert!(facts.iter().any(|f| f.0 == "disk") && facts.iter().any(|f| f.0 == "battery"), "{facts:?}");
    assert_eq!(items.last().unwrap().title, "Herdr Agents");
    // A peer's snapshot, its services as `check/` rows.
    let (items, _) = machine_items(Some(&seen[1]), 104);
    let check = items.iter().find(|i| i.id.as_str() == "sys:check/agent").unwrap();
    assert_eq!((check.title.as_str(), check.verb), ("agent", "Show Status"));
    assert_eq!(check_status(&seen[1], "agent").as_deref(), Some("agent: fail · not loaded"));
    assert_eq!(check_status(&seen[1], "gone"), None);
    // Nothing read yet: the name, no facts.
    let (items, _) = machine_items(Some(&seen[2]), 104);
    assert_eq!(titles(&items), ["pending · pro", "Herdr Agents"]);
    // A snapshot that does not read back as one shows no facts.
    let mut f = fleet(THREE);
    store(&mut f, 1, Ok(json!({ "at": "soon" })), Via::Flick, None, 100);
    let local = no_local();
    let seen = super::seen(&f, &local, 104, 45);
    assert_eq!(titles(&machine_items(Some(&seen[1]), 104).0), ["fresh · server", "Herdr Agents"]);
}

#[test]
fn fact_icons() {
    let icons: Vec<Icon> = ["load", "memory", "disk", "battery", "thermal"].into_iter().map(fact_icon).collect();
    let want = ["cpu", "memorychip", "internaldrive", "battery.100", "thermometer"].map(Icon::Symbol);
    assert_eq!(icons, want);
}
