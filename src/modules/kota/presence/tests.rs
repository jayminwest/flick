use super::*;
use crate::modules::kota::testkit::{AGENT_LIST, OK, UNKNOWN_MACHINE, exit};

const KOTA: &str = "/Users/jaymin/kota";

fn pane(id: &str, status: &str) -> Pane {
    Pane { id: id.into(), agent: "claude".into(), status: status.into(), cwd: KOTA.into(), ..Pane::default() }
}

fn checks(failing: &[&str]) -> Vec<(String, bool)> {
    ["health", "ticks", "queue"].iter().map(|n| ((*n).to_string(), !failing.contains(n))).collect()
}

fn seen(state: State) -> Seen {
    Seen { state, ..Seen::default() }
}

#[test]
fn the_real_agent_list_has_the_kota_pane() {
    let panes = parse_agents(AGENT_LIST).unwrap();
    assert_eq!(panes.len(), 1);
    let p = &panes[0];
    assert_eq!((p.id.as_str(), p.agent.as_str(), p.status.as_str()), ("wD:p1", "claude", "working"));
    assert_eq!((p.cwd.as_str(), p.name.as_str(), p.focused), (KOTA, "", true));
    assert_eq!(p.title, "Flick KOTA cards design brainstorm");
    assert_eq!(find_pane(&panes, KOTA, ""), Some(p));
}

#[test]
fn agent_lists_tolerate_nulls_and_report_errors() {
    let list = r#"{"result":{"agents":[{"pane_id":"w1:p2","agent":null,"cwd":null,"name":"x","focused":null}]}}"#;
    let panes = parse_agents(list).unwrap();
    assert_eq!(panes, [Pane { id: "w1:p2".into(), name: "x".into(), ..Pane::default() }]);
    // A reply without the `result` wrapper is read as the result itself.
    assert_eq!(parse_agents(r#"{"agents":[]}"#).unwrap(), []);
    assert_eq!(parse_agents(r#"{"error":{"code":"x","message":"no pane"}}"#), Err("no pane".into()));
    assert_eq!(parse_agents(r#"{"error":"flat"}"#), Err("\"flat\"".into()));
    assert!(parse_agents("not json").unwrap_err().starts_with("bad herdr reply: "));
    assert!(parse_agents(r#"{"result":{}}"#).unwrap_err().starts_with("bad herdr reply: missing field `agents`"));
}

#[test]
fn the_real_ok_reply_is_all_checks() {
    let checks = parse_ok(OK).unwrap();
    let names: Vec<&str> = checks.iter().map(|(n, _)| n.as_str()).collect();
    let mut want = ["health", "ticks", "queue", "memory", "calls", "agent_log"];
    want.sort_unstable();
    let mut got = names.clone();
    got.sort_unstable();
    assert_eq!(got, want);
    assert!(checks.iter().all(|(_, ok)| *ok));
    // Members that are not booleans are not checks.
    assert_eq!(parse_ok(r#"{"queue": false, "version": "1"}"#).unwrap(), [("queue".to_string(), false)]);
    assert_eq!(parse_ok(r#"{"version": "1"}"#), Err("bad /ok reply: no checks".into()));
    assert_eq!(parse_ok("[true]"), Err("bad /ok reply: not an object".into()));
    assert!(parse_ok("<html>").unwrap_err().starts_with("bad /ok reply: "));
}

#[test]
fn child_results_carry_the_childs_own_error() {
    assert_eq!(herdr_result(exit(0, AGENT_LIST, "")).unwrap().len(), 1);
    assert_eq!(
        herdr_result(exit(2, "", UNKNOWN_MACHINE)),
        Err("unknown machine 'nope'; use `herdr machine list`".into())
    );
    assert_eq!(herdr_result(exit(1, "", "{\"error\":{\"message\":\"server gone\"}}\n")), Err("server gone".into()));
    assert_eq!(herdr_result(exit(1, "", "{\"x\":1}")), Err("{\"x\":1}".into()));
    assert_eq!(herdr_result(exit(1, "", "Error: Capital\n")), Err("Capital".into()));
    assert_eq!(herdr_result(exit(1, "", "short")), Err("short".into()));
    assert_eq!(herdr_result(exit(3, "", " \n")), Err("herdr exited with Some(3)".into()));
    assert_eq!(herdr_result(Err("timed out after 10 s".into())), Err("timed out after 10 s".into()));
    assert_eq!(dash_result(exit(0, OK, "")).unwrap().len(), 6);
    assert_eq!(
        dash_result(exit(22, "", "curl: (22) The requested URL returned error: 404\n")),
        Err("curl: (22) The requested URL returned error: 404".into())
    );
    assert_eq!(dash_result(exit(7, "", "")), Err("curl exited with Some(7)".into()));
    assert_eq!(dash_result(Err("x".into())), Err("x".into()));
}

#[test]
fn the_kota_pane_is_claude_in_cwd_by_name_focus_then_id() {
    let other_dir = Pane { cwd: "/tmp".into(), ..pane("w1:p1", "idle") };
    let other_agent = Pane { agent: "codex".into(), ..pane("w1:p0", "idle") };
    let p10 = pane("w1:p10", "idle");
    let p2 = Pane { cwd: format!("{KOTA}/"), ..pane("w1:p2", "idle") };
    let panes = vec![other_dir.clone(), other_agent, p10.clone(), p2.clone()];
    assert_eq!(find_pane(&panes, KOTA, "").map(|p| p.id.as_str()), Some("w1:p2"));
    assert_eq!(find_pane(&panes, &format!("{KOTA}/"), "").map(|p| p.id.as_str()), Some("w1:p2"));
    let focused = Pane { focused: true, ..p10.clone() };
    let panes = vec![p2.clone(), focused];
    assert_eq!(find_pane(&panes, KOTA, "").map(|p| p.id.as_str()), Some("w1:p10"));
    let named = Pane { name: "kota".into(), ..p2.clone() };
    let panes = vec![Pane { focused: true, ..p10 }, named];
    assert_eq!(find_pane(&panes, KOTA, "kota").map(|p| p.id.as_str()), Some("w1:p2"));
    assert_eq!(find_pane(&panes, KOTA, "").map(|p| p.id.as_str()), Some("w1:p10"));
    assert_eq!(find_pane(&[other_dir], KOTA, ""), None);
    // A pane without a cwd never matches, and "/" is its own directory.
    assert_eq!(find_pane(&[Pane { cwd: String::new(), ..pane("w1:p3", "idle") }], "", ""), None);
    assert!(find_pane(&[Pane { cwd: "/".into(), ..pane("w1:p3", "idle") }], "/", "").is_some());
}

fn state(herdr: Result<Vec<Pane>, String>, dash: Result<Vec<(String, bool)>, String>) -> State {
    observe(&Round { herdr, dash }, KOTA, "").state
}

#[test]
fn a_round_maps_to_one_state() {
    let ok = || Ok(checks(&[]));
    let one = |status: &str| Ok(vec![pane("w1:p1", status)]);
    assert_eq!(state(one("working"), ok()), State::Thinking);
    assert_eq!(state(one("blocked"), ok()), State::Blocked);
    assert_eq!(state(one("idle"), ok()), State::Idle);
    assert_eq!(state(one("done"), ok()), State::Idle);
    assert_eq!(state(one("unknown"), ok()), State::Unknown);
    assert_eq!(state(one("working"), Ok(checks(&["queue"]))), State::Degraded);
    assert_eq!(state(one("idle"), Err("refused".into())), State::Degraded);
    // An approval waiting outranks a failing check.
    assert_eq!(state(one("blocked"), Err("refused".into())), State::Blocked);
    // The server answered and KOTA is not there.
    assert_eq!(state(Ok(vec![]), ok()), State::Down);
    assert_eq!(state(Ok(vec![]), Err("refused".into())), State::Down);
    assert_eq!(state(Err("timed out".into()), Ok(checks(&["health"]))), State::Down);
    // herdr failed but kota-dash says the server is healthy: degraded, not down.
    assert_eq!(state(Err("timed out".into()), ok()), State::Degraded);
    // Nothing answered: this Mac may be the one offline.
    assert_eq!(state(Err("timed out".into()), Err("refused".into())), State::Offline);
    // A pane present with health false is degraded: KOTA is visibly there.
    assert_eq!(state(one("working"), Ok(checks(&["health"]))), State::Degraded);
}

#[test]
fn a_round_names_failing_checks_and_errors() {
    let round = Round { herdr: Err("timed out".into()), dash: Ok(checks(&["queue", "ticks"])) };
    let seen = observe(&round, KOTA, "");
    assert_eq!(seen.failing, ["herdr", "ticks", "queue"]);
    assert_eq!(seen.errors, ["herdr: timed out"]);
    assert_eq!(seen.pane, None);
    let round = Round { herdr: Err("timed out".into()), dash: Err("refused".into()) };
    assert_eq!(observe(&round, KOTA, "").errors, ["herdr: timed out", "dash: refused"]);
    let round = Round { herdr: Ok(vec![pane("w1:p1", "idle")]), dash: Err("refused".into()) };
    let seen = observe(&round, KOTA, "");
    assert_eq!((seen.failing, seen.errors), (vec!["dash".to_string()], vec!["dash: refused".to_string()]));
    assert_eq!(seen.pane.map(|p| p.id), Some("w1:p1".into()));
}

#[test]
fn states_have_words() {
    let all = [
        (State::Thinking, "thinking"),
        (State::Blocked, "blocked"),
        (State::Idle, "idle"),
        (State::Degraded, "degraded"),
        (State::Down, "down"),
        (State::Offline, "offline"),
        (State::Unknown, "unknown"),
    ];
    for (s, word) in all {
        assert_eq!(s.word(), word);
        assert_eq!(s.failing(), matches!(s, State::Down | State::Offline), "{word}");
    }
    assert_eq!(State::default(), State::Unknown);
}

#[test]
fn good_rounds_commit_at_once_and_report_changes() {
    let mut p = Presence::default();
    assert_eq!(p.apply(seen(State::Unknown), 10, false), None);
    assert_eq!((p.state, p.since, p.checked_at), (State::Unknown, Some(10), Some(10)));
    let t = p.apply(seen(State::Thinking), 20, false);
    assert_eq!(t, Some(Transition { from: State::Unknown, to: State::Thinking }));
    assert_eq!(p.apply(seen(State::Thinking), 30, false), None);
    assert_eq!((p.since, p.checked_at), (Some(20), Some(30)));
}

#[test]
fn down_and_offline_need_two_rounds_in_a_row() {
    let mut p = Presence::default();
    p.apply(seen(State::Idle), 10, false);
    let down = Seen { errors: vec!["herdr: x".into()], ..seen(State::Down) };
    assert_eq!(p.apply(down.clone(), 20, false), None);
    assert!(p.pending());
    assert_eq!((p.state, p.checked_at, p.errors.len()), (State::Idle, Some(20), 1));
    // A good round in between resets the count.
    p.apply(seen(State::Idle), 30, false);
    assert!(!p.pending() && p.errors.is_empty());
    p.apply(seen(State::Offline), 40, false);
    // Two failing rounds in a row commit the second's state, whatever the first's was.
    assert_eq!(p.apply(down, 50, false), Some(Transition { from: State::Idle, to: State::Down }));
    assert_eq!((p.state, p.since, p.pending()), (State::Down, Some(50), false));
    // Once down, a failing round commits at once.
    assert_eq!(p.apply(seen(State::Down), 60, false), None);
    assert!(!p.pending());
    assert_eq!(p.apply(seen(State::Offline), 70, false), Some(Transition { from: State::Down, to: State::Offline }));
}

#[test]
fn sleep_marks_stale_and_wake_grace_keeps_the_last_state() {
    let mut p = Presence::default();
    p.apply(seen(State::Thinking), 10, false);
    p.apply(seen(State::Offline), 20, false);
    p.mark_stale();
    assert!(p.stale && !p.pending());
    // Failures in the grace period count for nothing.
    assert_eq!(p.apply(seen(State::Offline), 100, true), None);
    assert_eq!(p.apply(seen(State::Offline), 105, true), None);
    assert_eq!((p.state, p.stale, p.pending(), p.checked_at), (State::Thinking, true, false, Some(20)));
    // A good round clears stale, in the grace period or after.
    assert_eq!(p.apply(seen(State::Idle), 110, true), Some(Transition { from: State::Thinking, to: State::Idle }));
    assert!(!p.stale);
    assert!(!in_grace(100, None));
    assert!(in_grace(100, Some(80)));
    assert!(!in_grace(110, Some(80)));
    assert!(!in_grace(u64::MAX, Some(u64::MAX - 1)));
}

#[test]
fn cadence_follows_the_state() {
    let mut p = Presence::default();
    assert_eq!(next_in(&p, 0, true), None);
    assert_eq!(next_in(&p, 60, false), Some(60));
    assert_eq!(next_in(&p, 60, true), Some(FAST_SECS));
    p.apply(seen(State::Thinking), 1, false);
    assert_eq!(next_in(&p, 60, false), Some(FAST_SECS));
    p.apply(seen(State::Blocked), 2, false);
    assert_eq!(next_in(&p, 600, false), Some(FAST_SECS));
    p.apply(seen(State::Idle), 3, false);
    assert_eq!(next_in(&p, 600, false), Some(600));
    p.apply(seen(State::Down), 4, false);
    assert_eq!(next_in(&p, 600, false), Some(RETRY_SECS));
    assert_eq!(next_in(&p, 0, false), None);
}
