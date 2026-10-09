//! Fixtures are synthesized from the herdr 0.9.1 schema (`herdr api schema --json`); none
//! hold real terminal output.

use super::*;
use serde_json::json;

const LIST: &str = include_str!("../fixtures/agent_list.json");
const EVENTS: &str = include_str!("../fixtures/events.jsonl");
const READ: &str = include_str!("../fixtures/agent_read.json");

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn listed(machine: &str) -> Vec<Agent> {
    parse_agents(machine, &value(LIST)).unwrap()
}

fn events(machine: &str) -> Vec<Option<Change>> {
    EVENTS.lines().map(|l| parse_event(machine, &value(l))).collect()
}

fn agent(machine: &str, pane: &str, status: Status) -> Agent {
    Agent {
        machine: machine.into(),
        pane_id: pane.into(),
        workspace_id: "w1".into(),
        kind: Some("claude".into()),
        name: None,
        status,
        cwd: None,
        title: None,
        focused: false,
        seq: 0,
        changed_at: 0,
    }
}

fn order(fleet: &Fleet) -> Vec<String> {
    fleet.ordered().iter().map(|a| format!("{}/{}", a.machine, a.pane_id)).collect()
}

#[test]
fn parses_an_agent_list_reply() {
    let agents = listed("local");
    assert_eq!(agents.len(), 4);
    let a = &agents[0];
    assert_eq!(
        (a.machine.as_str(), a.pane_id.as_str(), a.workspace_id.as_str()),
        ("local", "w1:p1", "w1")
    );
    assert_eq!((a.kind.as_deref(), a.name.as_deref()), (Some("claude"), Some("api-refactor")));
    assert_eq!((a.status, a.seq, a.focused), (Status::Working, 7, false));
    assert_eq!(a.cwd.as_deref(), Some("/Users/example/src/api"));
    assert_eq!(a.title.as_deref(), Some("claude: refactor"));
    // display_agent wins over agent; the plain title fills a missing stripped title.
    assert_eq!(agents[1].kind.as_deref(), Some("pi (custom)"));
    assert!(agents[1].focused);
    assert_eq!((agents[2].title.as_deref(), agents[2].seq), (Some("codex"), 0));
    // Unknown status and unknown fields still parse.
    assert_eq!((agents[3].status, agents[3].kind.as_deref()), (Status::Unknown, None));
    // A bare result parses too.
    let bare = value(LIST)["result"].clone();
    assert_eq!(parse_agents("local", &bare).unwrap(), agents);
}

#[test]
fn reply_errors_surface() {
    let err = json!({"id": "1", "error": {"code": "not_found", "message": "no such pane"}});
    assert_eq!(parse_agents("local", &err).unwrap_err(), "no such pane");
    let odd = json!({"id": "1", "error": "boom"});
    assert_eq!(reply_result(&odd).unwrap_err(), "\"boom\"");
    let bad = json!({"id": "1", "result": {"type": "pong"}});
    assert!(parse_agents("mbp", &bad).unwrap_err().starts_with("mbp: bad agent.list reply"));
}

#[test]
fn parses_an_agent_read_reply() {
    let text = parse_read(&value(READ)).unwrap();
    assert!(text.starts_with("$ cargo test"));
    assert_eq!(
        parse_read(&json!({"result": {"type": "pong"}})).unwrap_err(),
        "bad agent.read reply"
    );
    assert!(parse_read(&json!({"error": {"message": "gone"}})).is_err());
}

#[test]
fn labels_fall_back_to_kind_then_pane() {
    let agents = listed("local");
    assert_eq!(agents[0].label(), "api-refactor");
    assert_eq!(agents[2].label(), "codex");
    assert_eq!(agents[3].label(), "w2:p2");
}

#[test]
fn statuses() {
    let all = [Status::Blocked, Status::Done, Status::Idle, Status::Working, Status::Unknown];
    let names: Vec<_> = all.iter().map(|s| s.as_str()).collect();
    assert_eq!(names, ["blocked", "done", "idle", "working", "unknown"]);
    let waiting: Vec<_> = all.iter().map(|s| s.waits_on_me()).collect();
    assert_eq!(waiting, [true, true, false, false, false]);
    let mut sorted = all;
    sorted.reverse();
    sorted.sort();
    assert_eq!(sorted, all);
}

#[test]
fn parses_events() {
    let e = events("local");
    assert_eq!(
        e[0],
        Some(Change::Status {
            pane_id: "w1:p1".into(),
            workspace_id: "w1".into(),
            status: Status::Blocked,
            kind: Some("claude".into()),
        })
    );
    let Some(Change::Status { kind, status, .. }) = &e[1] else { panic!("{:?}", e[1]) };
    assert_eq!((kind.as_deref(), *status), (Some("codex"), Status::Idle));
    let Some(Change::Pane(p)) = &e[2] else { panic!("{:?}", e[2]) };
    assert_eq!((p.pane_id.as_str(), p.kind.as_deref()), ("w3:p1", None));
    let Some(Change::Pane(p)) = &e[3] else { panic!("{:?}", e[3]) };
    assert_eq!((p.name.as_deref(), p.status), (Some("web-tests"), Status::Working));
    assert_eq!(
        e[4],
        Some(Change::Detected {
            pane_id: "w3:p1".into(),
            workspace_id: "w3".into(),
            kind: "claude".into()
        })
    );
    assert_eq!(e[5], Some(Change::Gone { pane_id: "w2:p2".into() }));
    assert_eq!(e[6], Some(Change::Gone { pane_id: "w2:p1".into() }));
    assert_eq!(e[7], Some(Change::Gone { pane_id: "w1:p1".into() }));
    assert_eq!(e[8], None);
}

#[test]
fn ignores_malformed_events() {
    for bad in [
        json!({}),
        json!({"event": 1, "data": {}}),
        json!({"event": "pane_closed"}),
        json!({"event": "pane_closed", "data": {}}),
        json!({"event": "pane_agent_status_changed", "data": {"pane_id": "p"}}),
        json!({"event": "pane_agent_status_changed", "data": {"agent_status": "idle"}}),
        json!({"event": "pane_agent_status_changed", "data": {"pane_id": "p", "agent_status": 3}}),
        json!({"event": "pane_updated", "data": {}}),
        json!({"event": "pane_updated", "data": {"pane": {"pane_id": "p"}}}),
        json!({"event": "pane_agent_detected", "data": {"agent": "pi"}}),
        json!({"event": "pane_agent_detected", "data": {}}),
    ] {
        assert_eq!(parse_event("local", &bad), None, "{bad}");
    }
    // Missing workspace ids default to empty.
    let e = json!({"event": "pane_agent_detected", "data": {"pane_id": "p", "agent": "pi"}});
    let Some(Change::Detected { workspace_id, .. }) = parse_event("m", &e) else { panic!() };
    assert!(workspace_id.is_empty());
}

#[test]
fn orders_waiting_first_then_most_recent() {
    let mut fleet = Fleet::new(["local", "mbp-server"]);
    fleet.apply_list("mbp-server", vec![agent("mbp-server", "a", Status::Blocked)], 10);
    fleet.apply_list("local", listed("local"), 20);
    // local blocked (newer) before mbp blocked; then done, working, unknown.
    assert_eq!(
        order(&fleet),
        ["local/w1:p2", "mbp-server/a", "local/w2:p1", "local/w1:p1", "local/w2:p2"]
    );
    // Same time and status: higher seq first, then machine order, then pane id.
    let mut tie = Fleet::new(["b", "a"]);
    let mut high = agent("a", "x", Status::Idle);
    high.seq = 5;
    tie.apply_list("a", vec![agent("a", "y", Status::Idle), agent("a", "z", Status::Idle)], 1);
    tie.apply_list("b", vec![agent("b", "z", Status::Idle)], 1);
    tie.apply_list("c", vec![high], 1);
    assert_eq!(order(&tie), ["c/x", "b/z", "a/y", "a/z"]);
    assert_eq!(tie.machine_names(), ["b", "a", "c"]);
}

#[test]
fn list_transitions_and_changes() {
    let mut fleet = Fleet::new(["local"]);
    // The first list makes no transitions, even for blocked agents.
    assert!(fleet.apply_list("local", listed("local"), 1).changed);
    assert!(fleet.take_transitions().is_empty());
    // The same list again changes nothing and keeps change times.
    assert_eq!(fleet.apply_list("local", listed("local"), 2), Applied::default());
    assert_eq!(fleet.agent("local", "w1:p2").unwrap().changed_at, 1);
    // working -> blocked transitions; blocked -> idle does not; a new blocked agent does not.
    let mut next = listed("local");
    next[0].status = Status::Blocked;
    next[1].status = Status::Idle;
    next.push(agent("local", "new", Status::Blocked));
    next.remove(3);
    assert!(fleet.apply_list("local", next, 3).changed);
    let t = fleet.take_transitions();
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].from, t[0].agent.pane_id.as_str()), (Status::Working, "w1:p1"));
    assert!(fleet.take_transitions().is_empty());
    let local = fleet.machine("local").unwrap();
    assert_eq!(local.agents.len(), 4);
    assert!(!local.agents.contains_key("w2:p2"));
    assert_eq!(local.agents["w1:p1"].changed_at, 3);
    assert_eq!(local.agents["w2:p1"].changed_at, 1);
    // An agent that only disappears is a change.
    let mut fewer: Vec<_> = local.agents.values().cloned().collect();
    fewer.pop();
    assert!(fleet.apply_list("local", fewer, 4).changed);
}

#[test]
fn errors_keep_cached_rows_until_a_good_list() {
    let mut fleet = Fleet::new(["mbp"]);
    fleet.apply_list("mbp", vec![agent("mbp", "a", Status::Idle)], 10);
    assert!(fleet.apply_error("mbp", "unreachable", 20).changed);
    assert!(!fleet.apply_error("mbp", "unreachable", 30).changed);
    let m = fleet.machine("mbp").unwrap();
    assert_eq!(
        (m.updated_at, m.checked_at, m.error.as_deref()),
        (Some(10), Some(30), Some("unreachable"))
    );
    assert_eq!(m.agents.len(), 1);
    // A good list with the same agents is still a change: the error row goes away.
    assert!(fleet.apply_list("mbp", vec![agent("mbp", "a", Status::Idle)], 40).changed);
    assert_eq!(fleet.machine("mbp").unwrap().error, None);
    // A machine first seen through an error.
    assert!(fleet.apply_error("kota", "asleep", 5).changed);
    assert_eq!(fleet.machine("kota").unwrap().updated_at, None);
}

#[test]
fn refresh_is_due_for_polled_machines_only() {
    let mut fleet = Fleet::new(["local", "mbp"]);
    assert!(fleet.needs_refresh("mbp", 0, 15));
    assert!(fleet.needs_refresh("other", 0, 15));
    fleet.apply_error("mbp", "timeout", 100);
    assert!(!fleet.needs_refresh("mbp", 114, 15));
    assert!(fleet.needs_refresh("mbp", 115, 15));
    // A clock that went backwards is not due.
    assert!(!fleet.needs_refresh("mbp", 50, 15));
    fleet.set_live("local", true);
    assert!(!fleet.needs_refresh("local", 1000, 15));
    fleet.set_live("local", false);
    assert!(fleet.needs_refresh("local", 1000, 15));
}

#[test]
fn applies_events() {
    let mut fleet = Fleet::new(["local"]);
    fleet.apply_list("local", listed("local"), 1);
    fleet.take_transitions();
    let e: Vec<Change> = events("local").into_iter().flatten().collect();
    let mut apply = |i: usize, now| fleet.apply_event("local", e[i].clone(), now);
    // working -> blocked.
    assert_eq!(apply(0, 5), Applied { changed: true, relist: false });
    // done -> idle.
    assert_eq!(apply(1, 6), Applied { changed: true, relist: false });
    // A new pane with no agent yet is ignored.
    assert_eq!(apply(2, 7), Applied::default());
    // pane_updated on a known pane: blocked -> working, name and title follow.
    assert_eq!(apply(3, 8), Applied { changed: true, relist: false });
    // An agent detected on an unknown pane adds a row and asks for a list.
    assert_eq!(apply(4, 9), Applied { changed: true, relist: true });
    // Released and exited agents go away.
    assert_eq!(apply(5, 10), Applied { changed: true, relist: false });
    assert_eq!(apply(6, 11), Applied { changed: true, relist: false });
    check_after_events(&mut fleet);
    // Closing a pane twice changes nothing the second time.
    assert!(fleet.apply_event("local", e[7].clone(), 12).changed);
    assert!(!fleet.apply_event("local", e[7].clone(), 13).changed);
}

fn check_after_events(fleet: &mut Fleet) {
    let t = fleet.take_transitions();
    assert_eq!(t.len(), 1);
    assert_eq!(
        (t[0].from, t[0].agent.pane_id.as_str(), t[0].agent.changed_at),
        (Status::Working, "w1:p1", 5)
    );
    let p2 = fleet.agent("local", "w1:p2").unwrap();
    assert_eq!(
        (p2.status, p2.name.as_deref(), p2.title.as_deref()),
        (Status::Working, Some("web-tests"), Some("pi: running"))
    );
    assert_eq!((p2.kind.as_deref(), p2.changed_at, p2.focused), (Some("pi"), 8, false));
    let p3 = fleet.agent("local", "w3:p1").unwrap();
    assert_eq!(
        (p3.kind.as_deref(), p3.status, p3.changed_at),
        (Some("claude"), Status::Unknown, 9)
    );
    assert_eq!(order(fleet), ["local/w1:p1", "local/w1:p2", "local/w3:p1"]);
}

#[test]
fn event_merges_keep_known_fields() {
    let mut fleet = Fleet::new(["m"]);
    let mut a = agent("m", "p", Status::Idle);
    a.name = Some("n".into());
    a.cwd = Some("/c".into());
    a.title = Some("t".into());
    fleet.apply_list("m", vec![a], 1);
    // A status event without an agent name keeps the kind; the same status is no change.
    let same = Change::Status {
        pane_id: "p".into(),
        workspace_id: "w1".into(),
        status: Status::Idle,
        kind: None,
    };
    assert_eq!(fleet.apply_event("m", same, 2), Applied::default());
    // A pane update with empty fields keeps the known ones.
    let mut bare = agent("m", "p", Status::Done);
    bare.kind = None;
    bare.focused = true;
    assert!(fleet.apply_event("m", Change::Pane(bare), 3).changed);
    let p = fleet.agent("m", "p").unwrap();
    assert_eq!(
        (p.kind.as_deref(), p.name.as_deref(), p.cwd.as_deref(), p.title.as_deref()),
        (Some("claude"), Some("n"), Some("/c"), Some("t"))
    );
    assert_eq!((p.status, p.focused, p.changed_at), (Status::Done, true, 3));
    assert_eq!(fleet.take_transitions().len(), 1);
    // A detected event renames the program without a status change.
    let det =
        Change::Detected { pane_id: "p".into(), workspace_id: "w1".into(), kind: "pi".into() };
    assert!(fleet.apply_event("m", det, 4).changed);
    assert_eq!(fleet.agent("m", "p").unwrap().kind.as_deref(), Some("pi"));
    // A status event and an agent pane update for unknown panes add rows.
    let new = Change::Status {
        pane_id: "q".into(),
        workspace_id: "w1".into(),
        status: Status::Blocked,
        kind: None,
    };
    assert_eq!(fleet.apply_event("m", new, 5), Applied { changed: true, relist: true });
    assert_eq!(
        fleet.apply_event("other", Change::Pane(agent("other", "r", Status::Idle)), 6),
        Applied { changed: true, relist: true }
    );
    assert!(fleet.take_transitions().is_empty());
    assert_eq!(fleet.agent("m", "missing"), None);
    assert_eq!(fleet.agent("missing", "p"), None);
}

#[test]
fn summary_and_machine_set() {
    let mut fleet = Fleet::new(["local", "mbp"]);
    fleet.apply_list("local", listed("local"), 1);
    assert_eq!(fleet.summary(), Summary { blocked: 1, done: 1, agents: 4, machines: 2 });
    fleet.set_machines(["mbp", "kota"]);
    assert_eq!(fleet.machine_names(), ["mbp", "kota"]);
    assert_eq!(fleet.machine("local"), None);
    assert_eq!(fleet.summary(), Summary { blocked: 0, done: 0, agents: 0, machines: 2 });
}
