use super::*;
use crate::modules::herdr::local::Pong;
use crate::modules::herdr::model::Status;

fn agent(machine: &str, pane: &str, name: Option<&str>, status: Status) -> Agent {
    Agent {
        machine: machine.into(),
        pane_id: pane.into(),
        workspace_id: "w1".into(),
        kind: Some("claude".into()),
        name: name.map(str::to_string),
        status,
        cwd: Some("/src/api".into()),
        title: None,
        focused: false,
        seq: 4,
        changed_at: 0,
    }
}

fn fleet() -> Fleet {
    let mut fleet = Fleet::new(["local", "hub", "off"]);
    let local = vec![
        agent("local", "w1:p1", Some("api"), Status::Working),
        agent("local", "w1:p2", Some("twin"), Status::Idle),
    ];
    fleet.apply_list("local", local, 900);
    fleet.set_live("local", true);
    let hub = vec![
        agent("hub", "w1:p1", Some("twin"), Status::Blocked),
        agent("hub", "w2:p1", Some("web"), Status::Idle),
    ];
    fleet.apply_list("hub", hub, 940);
    fleet.apply_error("off", "off: unreachable", 990);
    fleet
}

fn found(fleet: &Fleet, spec: &str) -> Result<String, String> {
    resolve(fleet, spec).map(|a| format!("{}/{}", a.machine, a.pane_id))
}

#[test]
fn targets_resolve_by_pane_id_or_unique_name() {
    let f = fleet();
    assert_eq!(found(&f, "hub/w1:p1").unwrap(), "hub/w1:p1");
    assert_eq!(found(&f, "hub/twin").unwrap(), "hub/w1:p1");
    assert_eq!(found(&f, "web").unwrap(), "hub/w2:p1");
    assert_eq!(found(&f, "w2:p1").unwrap(), "hub/w2:p1");
    // A pane id on two machines: the waiting one is first.
    assert_eq!(found(&f, "w1:p1").unwrap(), "hub/w1:p1");
    assert_eq!(found(&f, "nope/w1:p1").unwrap_err(), "herdr: no machine \"nope\"");
    assert_eq!(found(&f, "hub/ghost").unwrap_err(), "herdr: no agent \"hub/ghost\"");
    let twins = found(&f, "twin").unwrap_err();
    assert_eq!(twins, "herdr: \"twin\" names 2 agents; use <machine>/<pane id>");
}

#[test]
fn ls_lists_every_machine_and_agent() {
    let f = fleet();
    let v = ls_json(&f);
    let machines: Vec<&String> = v["machines"].as_object().unwrap().keys().collect();
    assert_eq!(machines, ["hub", "local", "off"]);
    assert_eq!(v["machines"]["hub"]["agents"][0]["status"], "blocked");
    assert_eq!(v["machines"]["local"]["live"], true);
    assert_eq!(v["machines"]["off"]["error"], "off: unreachable");
    assert_eq!(v["agents"].as_array().unwrap().len(), 4);
    assert_eq!(v["agents"][0]["machine"], "hub");

    let text = ls_text(&f, 1_000);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 5);
    assert!(lines[0].starts_with("hub/w1:p1"), "{text}");
    assert!(lines[0].contains("blocked") && lines[0].contains("twin  blocked · api"), "{text}");
    assert_eq!(lines[4], "off: off: unreachable");
    assert_eq!(ls_text(&Fleet::default(), 0), "no agents");
}

#[test]
fn status_reports_each_machine_and_the_server() {
    let f = fleet();
    let socket = Path::new("/tmp/h.sock");
    let herdr = Path::new("/opt/homebrew/bin/herdr");
    let paths = Paths { socket, herdr };
    let mut info = Info::default();
    let text = status_text(&f, &info, &paths, 1_000);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "local            live · 2 agents · read 1 min ago");
    assert_eq!(lines[1], "hub              polled · 2 agents · read 1 min ago");
    assert_eq!(lines[2], "off              polled · 0 agents · never read · error: off: unreachable");
    assert_eq!(lines[3..], ["socket /tmp/h.sock", "cli    /opt/homebrew/bin/herdr"]);

    info.pong = Some(Pong { version: "0.9.1".into(), protocol: 22 });
    info.discovery = Some("herdr: boom".into());
    info.jump = Some("jump to off/p: unreachable".into());
    let mut one = Fleet::new(["local"]);
    one.apply_list("local", vec![agent("local", "p", None, Status::Idle)], 1);
    one.set_live("local", false);
    let text = status_text(&one, &info, &paths, 1);
    assert!(text.starts_with("local            not connected · 1 agent · read just now\nherdr 0.9.1 (protocol 22)"), "{text}");
    assert!(text.ends_with("machine list: herdr: boom\nlast jump: jump to off/p: unreachable"), "{text}");

    let v = status_json(&f, &info, &paths);
    assert_eq!(v["machines"]["hub"]["agents"], 2);
    assert_eq!(v["herdr"]["protocol"], 22);
    assert_eq!(v["socket"], "/tmp/h.sock");
    assert_eq!(v["jump_error"], "jump to off/p: unreachable");
}
