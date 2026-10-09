use super::*;
use crate::modules::herdr::model::Status;
use crate::modules::herdr::testkit::{CLI, Cli, HOOKS, Server, info, status_event, wait};

fn shared(machines: &[&str]) -> Arc<Shared> {
    let shared = Arc::new(Shared::default());
    shared.set_machines(&machines.iter().map(|m| (*m).to_string()).collect::<Vec<_>>());
    shared
}

fn status_of(shared: &Shared, machine: &str, pane: &str) -> Option<Status> {
    lock(&shared.fleet).agent(machine, pane).map(|a| a.status)
}

fn transport(server: &Server, cli: &Cli) -> Transport {
    Transport { local: Local::new(&server.socket), remote: Remote::new(&cli.herdr) }
}

#[test]
fn the_local_thread_lists_follows_events_and_relists_new_panes() {
    let server = Server::start("follow", vec![info("w1:p1", "working")]);
    let sh = shared(&[LOCAL]);
    start_local(&sh, Local::new(&server.socket), HOOKS);
    // A second start while it runs is a no-op.
    start_local(&sh, Local::new(&server.socket), HOOKS);
    wait("first list", || status_of(&sh, LOCAL, "w1:p1") == Some(Status::Working));
    wait("subscription", || server.subscribers() == 1);
    assert!(lock(&sh.fleet).machine(LOCAL).unwrap().live);
    assert_eq!(lock(&sh.info).pong.as_ref().unwrap().version, "0.9.1");

    server.push(&json_event("ignored"));
    server.push(&status_event("w1:p1", "blocked"));
    wait("status event", || status_of(&sh, LOCAL, "w1:p1") == Some(Status::Blocked));

    // An unknown pane makes the thread list and subscribe again.
    server.agents.lock().unwrap().push(info("w2:p1", "idle"));
    server.push(&status_event("w2:p1", "idle"));
    wait("relist", || server.calls().iter().filter(|c| *c == "events.subscribe").count() >= 2);
    wait("new pane", || status_of(&sh, LOCAL, "w2:p1") == Some(Status::Idle));

    stop_local(&sh);
    wait("stopped", || lock(&sh.local).closer.is_none());
    // Stopping marks no error.
    thread::sleep(Duration::from_millis(50));
    assert_eq!(lock(&sh.fleet).machine(LOCAL).unwrap().error, None);
}

fn json_event(name: &str) -> serde_json::Value {
    serde_json::json!({ "event": name, "data": {} })
}

#[test]
fn a_server_that_quits_marks_the_machine_and_a_missing_one_says_not_running() {
    let server = Server::start("quit", vec![info("w1:p1", "idle")]);
    let sh = shared(&[LOCAL]);
    start_local(&sh, Local::new(&server.socket), HOOKS);
    wait("subscription", || server.subscribers() == 1);
    server.hang_up();
    wait("disconnect", || {
        lock(&sh.fleet).machine(LOCAL).unwrap().error.as_deref() == Some("herdr disconnected")
    });
    let fleet = lock(&sh.fleet);
    assert!(!fleet.machine(LOCAL).unwrap().live);
    // Cached rows stay.
    assert!(fleet.agent(LOCAL, "w1:p1").is_some());
    drop(fleet);

    let none = Arc::new(Shared::default());
    none.set_machines(&[LOCAL.to_string()]);
    start_local(&none, Local::new(server.dir.join("none.sock")), HOOKS);
    wait("not running", || {
        lock(&none.fleet).machine(LOCAL).unwrap().error.as_deref() == Some("herdr not running")
    });
}

#[test]
fn a_remote_round_lists_due_machines_in_parallel_and_keeps_errors() {
    let cli = Cli::new("round", CLI);
    let sh = shared(&[LOCAL, "hub", "off"]);
    let remote = Remote::new(&cli.herdr);
    assert!(poll_remote(&sh, &remote, 15, None, HOOKS));
    wait("hub", || status_of(&sh, "hub", "w1:p1") == Some(Status::Blocked));
    wait("off", || lock(&sh.fleet).machine("off").unwrap().error.is_some());
    wait("round done", || !sh.remote_busy.load(Ordering::Acquire));
    assert_eq!(lock(&sh.fleet).machine("off").unwrap().error.as_deref(), Some("off: unreachable"));
    let mut args = cli.args();
    args.sort();
    assert_eq!(args, ["--machine hub agent list", "--machine off agent list"]);
    // Both were read at `now`: nothing is due again yet, and the local machine never is.
    assert!(!poll_remote(&sh, &remote, 15, None, HOOKS));
    assert!(poll_remote(&sh, &remote, 0, Some(Duration::from_millis(1)), HOOKS));
    wait("second round", || cli.args().len() == 4);
}

#[test]
fn discovery_sets_local_plus_enabled_machines_unless_the_list_changed_meanwhile() {
    let cli = Cli::new("discover", CLI);
    let sh = shared(&[LOCAL]);
    discover(&sh, &Remote::new(&cli.herdr), HOOKS);
    wait("machines", || lock(&sh.fleet).machine_names().len() == 3);
    assert_eq!(lock(&sh.fleet).machine_names(), [LOCAL, "hub", "off"]);

    let broken = Cli::new("discover-bad", "echo 'error: boom' >&2; exit 2");
    let sh = shared(&[LOCAL]);
    discover(&sh, &Remote::new(&broken.herdr), HOOKS);
    wait("error", || lock(&sh.info).discovery.is_some());
    assert_eq!(lock(&sh.info).discovery.as_deref(), Some("herdr: boom"));
    assert_eq!(lock(&sh.fleet).machine_names(), [LOCAL]);
}

#[test]
fn a_preview_keeps_the_last_lines_of_the_agent_it_was_asked_for() {
    let server = Server::start("read", vec![]);
    let cli = Cli::new("read", CLI);
    let t = transport(&server, &cli);
    let sh = shared(&[LOCAL, "hub"]);
    fetch_preview(&sh, &t, LOCAL, "w1:p1", 6, HOOKS);
    let lines = |sh: &Shared| lock(&sh.preview).as_ref().and_then(|p| p.reply.clone());
    wait("local preview", || lines(&sh).is_some());
    assert_eq!(lines(&sh), Some(Ok("one\ntwo".to_string())));
    assert!(server.calls().contains(&"agent.read w1:p1".to_string()));

    fetch_preview(&sh, &t, "hub", "w1:p1", 1, HOOKS);
    wait("remote preview", || lines(&sh).is_some());
    assert_eq!(lines(&sh), Some(Ok("remote two".to_string())));
}

#[test]
fn a_jump_focuses_the_pane_and_records_a_failure() {
    let server = Server::start("jump", vec![]);
    let cli = Cli::new("jump", CLI);
    let t = transport(&server, &cli);
    let sh = shared(&[LOCAL, "hub", "off"]);
    jump(&sh, &t, LOCAL, "w1:p2", "WezTerm", HOOKS);
    wait("local focus", || server.calls().contains(&"agent.focus w1:p2".to_string()));
    jump(&sh, &t, "hub", "w1:p1", "WezTerm", HOOKS);
    wait("remote focus", || cli.args().contains(&"--machine hub agent focus w1:p1".to_string()));
    jump(&sh, &t, "off", "w1:p1", "WezTerm", HOOKS);
    wait("failure", || lock(&sh.info).jump.is_some());
    assert_eq!(lock(&sh.info).jump.as_deref(), Some("jump to off/w1:p1: off: unreachable"));
}

#[test]
fn the_timer_posts_until_stopped() {
    let sh = shared(&[]);
    start_timer(&sh, Duration::from_millis(5), HOOKS);
    let epoch = sh.timer_gen.load(Ordering::Acquire);
    stop_timer(&sh);
    assert_eq!(sh.timer_gen.load(Ordering::Acquire), epoch + 1);
    thread::sleep(Duration::from_millis(20));
}
