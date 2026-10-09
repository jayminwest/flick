//! The module over fake herdr transports (`testkit`): never the real socket or CLI.

use super::*;
use crate::config::parse;
use crate::core::test_cx;
use crate::modules::herdr::model::Status;
use testkit::{CLI, CLICKS, Cli, HOOKS, LISTENS, NOTES, Server, info, status_event, wait};

fn configured(text: &str) -> Result<Herdr, String> {
    let mut h = Herdr::with_hooks(HOOKS);
    let config = parse(text)?;
    h.configure(&config.section("herdr")?.ok_or("disabled")?)?;
    Ok(h)
}

/// A started module on a fake server and CLI, with `machines`.
fn started(server: &Server, cli: &Cli, machines: &str) -> Herdr {
    let text = format!(
        "[herdr]\nsocket = \"{}\"\nherdr = \"{}\"\nmachines = {machines}\n",
        server.socket.display(),
        cli.herdr.display()
    );
    let mut h = configured(&text).unwrap();
    test_cx("", |cx| h.on_event(Event::Started, cx));
    h
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_string()).collect()
}

fn status(h: &Herdr, machine: &str, pane: &str) -> Option<Status> {
    lock(&h.shared.fleet).agent(machine, pane).map(|a| a.status)
}

#[test]
fn settings_have_defaults_and_reject_bad_values() {
    let h = configured("").unwrap();
    assert_eq!(h.settings, Settings::default());
    assert!(h.hotkeys().is_empty());
    let bad = |text: &str| configured(text).err().unwrap();
    assert_eq!(bad("[herdr]\npreview_lines = 0"), "[herdr]: preview_lines must be 1 to 40");
    assert_eq!(bad("[herdr]\nremote_refresh_secs = 5"), "[herdr]: remote_refresh_secs must be 0 or at least 15");
    assert_eq!(bad("[herdr]\nmachines = [\"a/b\"]"), "[herdr]: bad machine name \"a/b\"");
    assert!(bad("[herdr]\nnotify_typo = 1").starts_with("[herdr]: unknown field"));
    assert_eq!(bad("[herdr]\nnotify = [\"working\"]"), "[herdr]: notify takes \"blocked\" and \"done\", not \"working\"");
    // The commented example in DEFAULT_CONFIG (config.rs).
    let example = "[herdr]\nmachines = [\"local\", \"mbp-server\"]\nremote_refresh_secs = 60\nterminal = \"WezTerm\"\n\
                   hotkey = \"cmd+ctrl+alt+shift+KeyA\"\npreview_lines = 6\nnotify = [\"blocked\", \"done\"]";
    assert_eq!(configured(example).unwrap().settings.notify, ["blocked", "done"]);
    let h = configured("[herdr]\nhotkey = \"cmd+shift+A\"").unwrap();
    assert_eq!(h.hotkeys(), [Binding { spec: "cmd+shift+A".into(), key: Ok("agents".into()) }]);
}

#[test]
fn the_root_item_opens_the_agents_view_and_hotkeys_toggle_it() {
    let mut h = configured("").unwrap();
    test_cx("", |cx| {
        let items = h.items(cx);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "herdr:agents");
        assert!(matches!(h.activate(&items[0].id, cx), Outcome::Push(v) if v.is(ID, "agents")));
        assert!(h.hotkey("agents", cx).is_some_and(|v| v.is(ID, "agents")));
        assert!(h.hotkey("other", cx).is_none());
    });
}

#[test]
fn unknown_views_items_and_actions_do_nothing() {
    let mut h = configured("").unwrap();
    test_cx("", |cx| {
        assert!(h.open("nope", cx).is_none());
        // No agent chosen yet: no detail view.
        assert!(h.open("agent", cx).is_none());
        assert!(matches!(h.activate(&ItemId::new(ID, "line/0"), cx), Outcome::Stay(None)));
        assert!(matches!(h.activate(&ItemId::new(ID, "bogus"), cx), Outcome::Stay(None)));
        assert!(h.actions(&ItemId::new(ID, "agents"), cx).is_empty());
        assert!(matches!(h.act(&ItemId::new(ID, "agents"), "jump", cx), Outcome::Stay(None)));
    });
}

#[test]
fn started_lists_explicit_machines_and_the_view_shows_them_waiting_first() {
    let server = Server::start("view", vec![info("w1:p1", "working"), info("w1:p2", "blocked")]);
    let cli = Cli::new("view", CLI);
    let mut h = started(&server, &cli, "[\"local\", \"hub\", \"off\"]");
    wait("local list", || status(&h, "local", "w1:p2") == Some(Status::Blocked));
    test_cx("", |cx| {
        let mut view = h.open("agents", cx).unwrap();
        assert!(!view.record_use);
        wait("remote list", || status(&h, "hub", "w1:p1").is_some());
        wait("remote error", || lock(&h.shared.fleet).machine("off").unwrap().error.is_some());
        h.refresh(&mut view, cx);
        let ids: Vec<&str> = view.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "herdr:agent/hub/w1:p1",
                "herdr:agent/local/w1:p2",
                "herdr:agent/local/w1:p1",
                "herdr:machine/off"
            ]
        );
        assert!(view.footer.starts_with("2 waiting · 3 agents · 3 machines"), "{}", view.footer);
        let off = &view.items[3].id;
        assert_eq!(
            match h.activate(off, cx) {
                Outcome::Stay(s) => s,
                _ => None,
            }
            .as_deref(),
            Some("off: off: unreachable")
        );
    });
    test_cx("hub", |cx| {
        let mut view = ListView::new(ID, "agents");
        h.refresh(&mut view, cx);
        assert_eq!(view.items[0].id, "herdr:agent/hub/w1:p1");
    });
}

#[test]
fn enter_and_actions_jump_and_show_output() {
    let server = Server::start("jumps", vec![info("w1:p1", "idle")]);
    let cli = Cli::new("jumps", CLI);
    let mut h = started(&server, &cli, "[\"local\", \"hub\"]");
    wait("local list", || status(&h, "local", "w1:p1").is_some());
    let id = ItemId::new(ID, "agent/local/w1:p1");
    test_cx("", |cx| {
        let keys: Vec<&str> = h.actions(&id, cx).iter().map(|a| a.key).collect();
        assert_eq!(keys, ["jump", "output"]);
        assert!(matches!(h.activate(&id, cx), Outcome::Hide));
        wait("focus", || server.calls().contains(&"agent.focus w1:p1".to_string()));
        assert!(matches!(h.act(&id, "jump", cx), Outcome::Hide));

        let Outcome::Push(view) = h.act(&id, "output", cx) else { panic!("no push") };
        let mut view = h.open(&view.name, cx).unwrap();
        wait("preview", || lock(&h.shared.preview).as_ref().is_some_and(|p| p.lines.is_some()));
        h.refresh(&mut view, cx);
        let titles: Vec<&str> = view.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Jump to n-w1:p1", "one", "two"]);
        assert!(view.footer.starts_with("n-w1:p1 · local"), "{}", view.footer);
        let calls = server.calls().iter().filter(|c| c.starts_with("agent.focus")).count();
        assert!(matches!(h.activate(&view.items[1].id, cx), Outcome::Hide));
        wait("line jump", || {
            server.calls().iter().filter(|c| c.starts_with("agent.focus")).count() > calls
        });
    });
    // The agent went away: the detail view says so.
    h.detail = Some(("local".into(), "gone".into()));
    test_cx("", |cx| {
        let mut view = ListView::new(ID, "agent");
        h.refresh(&mut view, cx);
        assert!(view.items.is_empty());
        assert_eq!(view.footer, "Agent gone");
    });
}

#[test]
fn commands_list_jump_and_report_status() {
    let server = Server::start("cmds", vec![info("w1:p1", "blocked")]);
    let cli = Cli::new("cmds", CLI);
    let mut h = started(&server, &cli, "[\"local\"]");
    wait("local list", || status(&h, "local", "w1:p1").is_some());
    test_cx("", |cx| {
        let ls = h.command(&args(&["ls"]), cx).unwrap();
        assert!(ls.starts_with("local/w1:p1"), "{ls}");
        assert_eq!(h.command(&args(&["ls", "-x"]), cx).unwrap_err(), "herdr: usage: herdr ls");
        assert_eq!(h.command(&args(&["jump", "local/w1:p1"]), cx).unwrap(), "jumping to local/w1:p1");
        wait("cli focus", || server.calls().contains(&"agent.focus w1:p1".to_string()));
        assert!(h.command(&args(&["jump", "local/zzz"]), cx).is_err());
        assert!(h.command(&args(&["jump"]), cx).unwrap_err().starts_with("herdr: usage"));
        let st = h.command(&args(&["status"]), cx).unwrap();
        assert!(st.starts_with("local            live · 1 agent"), "{st}");
        assert_eq!(h.command(&args(&["nope"]), cx).unwrap_err(), "herdr: unknown command \"nope\"");
        cx.json = true;
        let ls: serde_json::Value = serde_json::from_str(&h.command(&args(&["ls"]), cx).unwrap()).unwrap();
        assert_eq!(ls["machines"]["local"]["agents"][0]["pane_id"], "w1:p1");
        let st: serde_json::Value =
            serde_json::from_str(&h.command(&args(&["status"]), cx).unwrap()).unwrap();
        assert_eq!(st["machines"]["local"]["live"], true);
    });
    assert_eq!(h.verbs(), "herdr ls | herdr jump <machine>/<pane id or name> | herdr status");
}

#[test]
fn empty_machines_discover_herdrs_profiles_and_reload_restarts_threads() {
    let server = Server::start("disc", vec![info("w1:p1", "idle")]);
    let cli = Cli::new("disc", CLI);
    let mut h = started(&server, &cli, "[]");
    wait("discovery", || lock(&h.shared.fleet).machine_names() == [LOCAL, "hub", "off"]);
    wait("subscribed", || server.subscribers() == 1);
    // Events and wake do not poll remote machines while the launcher is hidden.
    test_cx("", |cx| {
        assert!(!h.on_event(Event::ModuleChanged { module: ID }, cx));
        assert!(!h.on_event(Event::Wake, cx));
    });
    assert!(!cli.args().iter().any(|a| a.ends_with("agent list")));
    test_cx("", |cx| h.on_event(Event::LauncherOpened, cx));
    wait("remote poll", || cli.args().iter().filter(|a| a.ends_with("agent list")).count() == 2);

    // Reload onto another socket and explicit machines: the old subscription closes.
    let other = Server::start("disc2", vec![info("w9:p9", "done")]);
    let text = format!(
        "[herdr]\nsocket = \"{}\"\nherdr = \"{}\"\nmachines = [\"local\"]\nremote_refresh_secs = 15\n",
        other.socket.display(),
        cli.herdr.display()
    );
    let config = parse(&text).unwrap();
    h.configure(&config.section("herdr").unwrap().unwrap()).unwrap();
    wait("new server", || status(&h, "local", "w9:p9") == Some(Status::Done));
    assert_eq!(lock(&h.shared.fleet).machine_names(), [LOCAL]);
    // Background refresh on: a hidden tick polls (nothing remote here, so nothing runs).
    test_cx("", |cx| h.on_event(Event::ModuleChanged { module: ID }, cx));
    // Back to discovery.
    let text = text.replace("machines = [\"local\"]\n", "");
    h.configure(&parse(&text).unwrap().section("herdr").unwrap().unwrap()).unwrap();
    wait("rediscovered", || lock(&h.shared.fleet).machine_names().len() == 3);
}

#[test]
fn a_fleet_without_local_runs_no_local_thread() {
    let server = Server::start("nolocal", vec![]);
    let cli = Cli::new("nolocal", CLI);
    let h = started(&server, &cli, "[\"hub\"]");
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(server.calls().is_empty());
    assert_eq!(lock(&h.shared.fleet).machine_names(), ["hub"]);
    assert!(unix_now() > 1_700_000_000);
}

#[test]
fn an_agent_that_starts_to_wait_posts_one_notification_and_a_click_jumps() {
    let server = Server::start("notes", vec![info("w1:p1", "working"), info("w1:p2", "blocked")]);
    let cli = Cli::new("notes", CLI);
    let mut h = started(&server, &cli, "[\"local\"]");
    assert_eq!(LISTENS.take(), [true]);
    wait("subscribed", || server.subscribers() == 1);
    let changed = |h: &mut Herdr| test_cx("", |cx| h.on_event(Event::ModuleChanged { module: ID }, cx));
    // The first list (p2 already blocked) and a repeat of a status post nothing.
    changed(&mut h);
    server.push(&status_event("w1:p2", "blocked"));
    server.push(&status_event("w1:p1", "blocked"));
    wait("blocked", || status(&h, "local", "w1:p1") == Some(Status::Blocked));
    changed(&mut h);
    changed(&mut h);
    assert_eq!(NOTES.take(), ["herdr:agent/local/w1:p1 | n-w1:p1 is waiting | local · /Users/example/src/api"]);
    // `done` is not asked for by default.
    server.push(&status_event("w1:p1", "done"));
    wait("done", || status(&h, "local", "w1:p1") == Some(Status::Done));
    changed(&mut h);
    assert!(NOTES.take().is_empty());

    CLICKS.with_borrow_mut(|c| c.extend(["herdr:agents".into(), "herdr:agent/local/w1:p1".into()]));
    changed(&mut h);
    wait("click focus", || server.calls().contains(&"agent.focus w1:p1".to_string()));
    test_cx("", |cx| {
        let st = h.command(&args(&["status"]), cx).unwrap();
        assert!(st.ends_with("\nnotifications: test (blocked)"), "{st}");
        cx.json = true;
        let st: serde_json::Value = serde_json::from_str(&h.command(&args(&["status"]), cx).unwrap()).unwrap();
        assert_eq!(st["notifications"], "test (blocked)");
    });

    // notify = [] turns them off and asks for no permission.
    let text = format!(
        "[herdr]\nsocket = \"{}\"\nherdr = \"{}\"\nmachines = [\"local\"]\nnotify = []\n",
        server.socket.display(),
        cli.herdr.display()
    );
    h.configure(&parse(&text).unwrap().section("herdr").unwrap().unwrap()).unwrap();
    assert_eq!(LISTENS.take(), [false]);
    server.push(&status_event("w1:p1", "blocked"));
    wait("blocked again", || status(&h, "local", "w1:p1") == Some(Status::Blocked));
    changed(&mut h);
    assert!(NOTES.take().is_empty());
    test_cx("", |cx| assert!(h.command(&args(&["status"]), cx).unwrap().ends_with("notifications: off")));
}

#[test]
fn the_example_config_lists_every_key() {
    crate::config::example::assert_documents::<Settings>("herdr");
}
