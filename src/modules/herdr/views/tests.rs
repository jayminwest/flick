use super::*;

pub fn agent(machine: &str, pane: &str, status: Status) -> Agent {
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

fn fleet() -> Fleet {
    let mut fleet = Fleet::new(["local", "hub", "off"]);
    let named = Agent {
        name: Some("api".into()),
        cwd: Some("/Users/example/src/api/".into()),
        title: Some("claude: refactor".into()),
        ..agent("local", "w1:p1", Status::Working)
    };
    fleet.apply_list("local", vec![named, agent("local", "w1:p2", Status::Done)], 900);
    fleet.apply_list("hub", vec![agent("hub", "w1:p1", Status::Blocked)], 940);
    fleet.apply_error("off", "off: unreachable", 990);
    fleet
}

#[test]
fn keys_parse_and_round_trip() {
    assert_eq!(Key::parse("agents"), Some(Key::Agents));
    assert_eq!(
        Key::parse(&agent_key("hub", "w1:p1")),
        Some(Key::Agent { machine: "hub", pane_id: "w1:p1" })
    );
    assert_eq!(Key::parse("agent/hub"), None);
    assert_eq!(Key::parse("machine/off"), Some(Key::Machine("off")));
    assert_eq!(Key::parse("line/3"), Some(Key::Line));
    assert_eq!(Key::parse("reply"), Some(Key::Reply));
    assert_eq!(Key::parse("other"), None);
}

#[test]
fn ages_are_short() {
    let ages: Vec<String> = [0, 42, 180, 7200, 432_000].into_iter().map(age).collect();
    assert_eq!(ages, ["now", "42 s", "3 min", "2 h", "5 d"]);
    assert_eq!((ago(1), ago(60)), ("just now".to_string(), "1 min ago".to_string()));
}

#[test]
fn the_root_item_counts_waiting_agents_and_machines() {
    let item = root_item(&fleet());
    assert_eq!(item.id, "herdr:agents");
    assert_eq!(item.title, "Herdr Agents");
    assert_eq!(item.subtitle, "2 waiting · 3 agents · 3 machines");
    let mut one = Fleet::new(["local"]);
    one.apply_list("local", vec![agent("local", "p", Status::Idle)], 1);
    assert_eq!(summary_line(&one), "1 agent · 1 machine");
}

#[test]
fn the_agents_view_lists_waiting_first_then_machine_notes() {
    let items = agents_items(&fleet(), 1_000);
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "herdr:agent/hub/w1:p1",
            "herdr:agent/local/w1:p2",
            "herdr:agent/local/w1:p1",
            "herdr:machine/off"
        ]
    );
    assert_eq!(items[0].title, "claude · hub");
    assert_eq!(items[0].subtitle, "blocked");
    assert_eq!(items[0].accessory, "1 min");
    assert_eq!(items[0].icon, Icon::Symbol("exclamationmark.bubble"));
    assert_eq!(items[2].title, "api · local");
    assert_eq!(items[2].subtitle, "working · api · claude: refactor");
    assert_eq!(items[3].subtitle, "off: unreachable");
    assert!(items[3].keywords.contains(&"off: unreachable".to_string()));
}

#[test]
fn machine_notes_say_what_is_missing() {
    let mut state = MachineState::default();
    assert_eq!(machine_note(&state, 10).as_deref(), Some("loading…"));
    state.live = true;
    assert_eq!(machine_note(&state, 10), None);
    state.error = Some("herdr not running".into());
    assert_eq!(machine_note(&state, 10).as_deref(), Some("herdr not running"));
    state.updated_at = Some(10);
    assert_eq!(machine_note(&state, 190).as_deref(), Some("herdr not running, last read 3 min ago"));
    state.error = None;
    state.live = false;
    assert_eq!(machine_note(&state, 190), None);
}

#[test]
fn icons_follow_the_status() {
    let icons: Vec<Icon> =
        [Status::Done, Status::Idle, Status::Working, Status::Unknown].into_iter().map(status_icon).collect();
    let names = ["checkmark.circle", "pause.circle", "gearshape", "questionmark.circle"];
    assert_eq!(icons, names.map(Icon::Symbol));
    let mut odd = agent("local", "p", Status::Idle);
    odd.cwd = Some("/".into());
    odd.title = Some(String::new());
    assert_eq!(agent_subtitle(&odd), "idle");
    odd.cwd = Some("repo".into());
    assert_eq!(agent_subtitle(&odd), "idle · repo");
    assert_eq!(agent_item(&odd, 5).accessory, "");
}

#[test]
fn the_detail_view_puts_jump_first_then_the_reply() {
    let fleet = fleet();
    let a = fleet.agent("hub", "w1:p1");
    let (none, text) = detail(None, None, 0);
    assert!(none.is_empty() && text.is_empty());
    let shown = |preview: Option<&Preview>| -> (Vec<String>, String) {
        let (items, text) = detail(a, preview, 1_000);
        (items.into_iter().map(|i| i.title).collect(), text)
    };
    let titles = |preview: Option<&Preview>| shown(preview).0;
    assert_eq!(titles(None), ["Jump to claude", "Loading output…"]);
    let p = |reply| Preview { machine: "hub".into(), pane_id: "w1:p1".into(), reply };
    assert_eq!(titles(Some(&p(Some(Err("hub: timed out".into()))))), ["Jump to claude", "hub: timed out"]);
    assert_eq!(shown(Some(&p(Some(Ok(String::new()))))), (vec!["Jump to claude".into(), "No reply".into()], String::new()));
    let (items, text) = detail(a, Some(&p(Some(Ok("a\nb".into())))), 1_000);
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["herdr:agent/hub/w1:p1", "herdr:reply"]);
    assert_eq!((items[1].subtitle.as_str(), text.as_str()), ("2 lines", "a\nb"));
    // Jump has no Tab; an agent row in the list does.
    assert_eq!((items[0].tab, agent_item(a.unwrap(), 0).tab), (Tab::None, Tab::Act("output")));
    let one = detail(a, Some(&p(Some(Ok("a".into())))), 1_000).0;
    assert_eq!(one[1].subtitle, "1 line");
    // Another agent's preview is not this one's.
    let other = Preview { pane_id: "w9:p9".into(), ..p(Some(Ok("x".into()))) };
    assert_eq!(titles(Some(&other)), ["Jump to claude", "Loading output…"]);
    assert_eq!(loaded_reply(a.unwrap(), Some(&other)), None);
}
