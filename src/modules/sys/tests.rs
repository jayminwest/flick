use super::*;
use crate::config::parse;
use crate::core::test_cx;
use testkit::HOOKS;

fn sys(config: &str, hooks: Hooks) -> Sys {
    let mut m = Sys::with_hooks(hooks, testkit::WINDOW);
    let table = parse(config).unwrap().section(ID).unwrap().unwrap();
    m.configure(&table).unwrap();
    m
}

fn ask(m: &mut Sys, words: &[&str], json: bool) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    test_cx("", |cx| {
        cx.json = json;
        m.command(&args, cx)
    })
}

const SERVICES: &str = r#"
[[sys.service]]
name = "web"
kind = "http"
target = "http://ok/"
[[sys.service]]
name = "agent"
kind = "launchd"
target = "killed"
"#;

#[test]
fn snapshot_answers_inline_the_first_time() {
    let mut m = sys("", HOOKS);
    let text = ask(&mut m, &["snapshot"], false).unwrap();
    assert!(text.starts_with("mbp-server · up 3d 13h · read 0s ago\nload     1.42 2.06 2.42 · 11 CPUs\n"), "{text}");
    let v: serde_json::Value = serde_json::from_str(&ask(&mut m, &["snapshot"], true).unwrap()).unwrap();
    assert_eq!((v["host"].as_str(), v["ncpu"].as_u64(), v["services"].as_array().map(Vec::len)), (Some("mbp-server"), Some(11), Some(0)));
}

#[test]
fn snapshot_reports_a_probe_that_never_answered() {
    let fails = Hooks { run: |_, _| Err("timed out after 2 s".into()), ..HOOKS };
    let mut m = sys("", fails);
    assert_eq!(ask(&mut m, &["snapshot"], false).unwrap_err(), "sys: no snapshot yet (probe: timed out after 2 s)");
    // A probe slower than the first wait: the command gives up, the round goes on.
    let slow = Hooks {
        run: |_, _| {
            std::thread::sleep(Duration::from_millis(300));
            Err("late".into())
        },
        ..HOOKS
    };
    let mut m = sys("", slow);
    m.first_wait = Duration::from_millis(20);
    assert_eq!(ask(&mut m, &["snapshot"], false).unwrap_err(), "sys: no snapshot yet (probe still running)");
}

#[test]
fn services_wait_for_the_first_round_then_answer_from_cache() {
    let mut m = sys(SERVICES, HOOKS);
    let want = "ok       web    http     HTTP 200 in 12 ms · 0s ago\n\
                warn     agent  launchd  running (pid 95368); last killed: Killed: 9 · 0s ago";
    assert_eq!(ask(&mut m, &["services"], false).unwrap(), want);
    let v: serde_json::Value = serde_json::from_str(&ask(&mut m, &["services"], true).unwrap()).unwrap();
    let statuses: Vec<&str> = v["services"].as_array().unwrap().iter().map(|s| s["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["ok", "warn"]);
    // The snapshot carries the same verdicts.
    let snap: serde_json::Value = serde_json::from_str(&ask(&mut m, &["snapshot"], true).unwrap()).unwrap();
    assert_eq!(snap["services"], v["services"]);
}

#[test]
fn no_services_means_no_round() {
    let mut m = sys("", HOOKS);
    assert_eq!(ask(&mut m, &["services"], false).unwrap(), "no services (add [[sys.service]] tables to config.toml)");
    assert_eq!(ask(&mut m, &["services"], true).unwrap(), r#"{"schema":1,"services":[]}"#);
    assert!(!m.shared.lock().checking);
}

#[test]
fn a_reload_takes_the_new_services() {
    let mut m = sys(SERVICES, HOOKS);
    ask(&mut m, &["services"], false).unwrap();
    let one = "[[sys.service]]\nname = \"web\"\nkind = \"http\"\ntarget = \"http://ok/\"\n";
    m.configure(&parse(one).unwrap().section(ID).unwrap().unwrap()).unwrap();
    assert_eq!(ask(&mut m, &["services"], false).unwrap(), "ok       web  http     HTTP 200 in 12 ms · 0s ago");
    let bad = parse("[[sys.service]]\nname = \"x\"\nkind = \"tcp\"\ntarget = \"x\"\n").unwrap();
    assert!(m.configure(&bad.section(ID).unwrap().unwrap()).is_err());
}

#[test]
fn verbs_and_unknown_ones() {
    let mut m = sys("", HOOKS);
    assert_eq!(m.id(), "sys");
    assert_eq!(
        m.verbs(),
        "sys snapshot | sys services | sys fleet | sys tail [<machine>] <service> | sys restart [<machine>] <service> [--yes] | sys window [--snapshot <png>]"
    );
    assert_eq!(ask(&mut m, &["reboot"], false).unwrap_err(), "sys: unknown command \"reboot\"");
    assert_eq!(ask(&mut m, &["snapshot", "now"], false).unwrap_err(), "sys: unknown command \"snapshot\"");
    // The real module builds with the real hooks and runs nothing until asked.
    let real = Sys::new(PeerHooks { ask: |_, _, _| Err("no peers in tests".into()) });
    let st = real.shared.lock();
    assert!(st.snapshot.is_none() && !st.probing);
    assert!(unix_now() > 1_700_000_000);
}

const FLEET: &str = r#"
[[sys.machine]]
name = "laptop"
via = "local"
[[sys.machine]]
name = "server"
via = "flick"
[[sys.machine]]
name = "pro"
via = "ssh"
ssh = "pro"
[[sys.machine.service]]
name = "ollama"
kind = "process"
target = "ollama"
"#;

fn event(m: &mut Sys, e: Event) {
    assert!(!test_cx("", |cx| m.on_event(e, cx)));
}

fn settle(m: &Sys) {
    assert!(m.shared.wait(Duration::from_secs(15), |s| !s.fleet.busy() && !s.probing && !s.checking));
}

fn tried(m: &Sys) -> Vec<Option<u64>> {
    m.shared.lock().fleet.slots.iter().map(|s| s.tried_at).collect()
}

#[test]
fn fleet_reads_every_machine_the_first_time() {
    let mut m = sys(&FLEET.replace("name = \"server\"", "name = \"server\"\nhost = \"server\""), HOOKS);
    let text = ask(&mut m, &["fleet"], false).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("fresh    laptop  local  load 1.42 · mem "), "{text}");
    assert!(lines[1].starts_with("fresh    server  flick  load 0.50 · 0s ago"), "{text}");
    assert!(lines[2].starts_with("fresh    pro     ssh    load "), "{text}");
    let v: serde_json::Value = serde_json::from_str(&ask(&mut m, &["fleet"], true).unwrap()).unwrap();
    let hosts: Vec<&str> = v["machines"].as_array().unwrap().iter().map(|m| m["snapshot"]["host"].as_str().unwrap()).collect();
    assert_eq!(hosts, ["mbp-server", "server", "mac-pro"]);
    assert_eq!(v["machines"][2]["snapshot"]["services"][0]["reason"], "running (pid 812)");
}

#[test]
fn a_remote_caller_reads_the_fleet_cache_only() {
    let mut m = sys(FLEET, HOOKS);
    let text = test_cx("", |cx| {
        cx.remote = true;
        m.command(&["fleet".to_string()], cx)
    });
    assert_eq!(text.unwrap(), "pending  laptop  local
pending  server  flick
pending  pro     ssh");
    let st = m.shared.lock();
    assert!(!st.fleet.busy() && !st.probing && st.fleet.never_tried());
}

#[test]
fn fleet_without_machines_runs_nothing() {
    let mut m = sys("", HOOKS);
    assert_eq!(ask(&mut m, &["fleet"], false).unwrap(), "no machines (add [[sys.machine]] tables to config.toml)");
    assert_eq!(ask(&mut m, &["fleet"], true).unwrap(), r#"{"machines":[],"schema":1,"stale_after_secs":45}"#);
    for e in [Event::Started, Event::LauncherOpened, Event::Wake, Event::ModuleChanged { module: ID }] {
        event(&mut m, e);
    }
    let st = m.shared.lock();
    assert!(!st.probing && !st.fleet.busy() && st.snapshot.is_none());
    assert_eq!(st.fleet.timer, 2, "Started and Wake only stopped the timer; a start would bump it again");
}

#[test]
fn launcher_open_and_wake_poll_and_idle_ticks_do_not() {
    let mut m = sys(FLEET, HOOKS);
    // Not started yet: events start nothing.
    event(&mut m, Event::LauncherOpened);
    assert_eq!(tried(&m), [None, None, None]);
    event(&mut m, Event::Started);
    event(&mut m, Event::ModuleChanged { module: ID });
    settle(&m);
    assert_eq!(tried(&m), [None, None, None], "refresh_secs = 0 and the view closed: idle");
    event(&mut m, Event::LauncherOpened);
    settle(&m);
    assert_eq!(tried(&m), [None, Some(1_000), Some(1_000)]);
    assert!(m.shared.lock().snapshot.is_some(), "the local machine refreshed this Mac's cache");
    // Again within the cadence: nothing is due.
    m.shared.lock().fleet.slots[1].tried_at = Some(990);
    event(&mut m, Event::LauncherOpened);
    settle(&m);
    assert_eq!(tried(&m), [None, Some(990), Some(1_000)]);
    // Wake reads every machine.
    event(&mut m, Event::Wake);
    settle(&m);
    assert_eq!(tried(&m), [None, Some(1_000), Some(1_000)]);
}

#[test]
fn sleep_drops_the_round_in_flight_and_stops_the_timer() {
    let config = format!("[sys]\nrefresh_secs = 3600\n{}", FLEET.replace("ssh = \"pro\"", "ssh = \"slow\""));
    let mut m = sys(&config, HOOKS);
    event(&mut m, Event::Started);
    let timer = m.shared.lock().fleet.timer;
    event(&mut m, Event::Wake);
    event(&mut m, Event::Sleep);
    std::thread::sleep(Duration::from_millis(500));
    let st = m.shared.lock();
    assert_eq!(st.fleet.slots[2].tried_at, None, "the late ssh answer was dropped");
    assert!(st.fleet.timer > timer + 1, "wake restarted the timer, sleep stopped it");
}

#[test]
fn ticks_poll_on_the_background_cadence_or_while_the_view_shows() {
    let config = format!("[sys]\nrefresh_secs = 40\n{FLEET}");
    let mut m = sys(&config, HOOKS);
    event(&mut m, Event::Started);
    {
        let mut st = m.shared.lock();
        st.fleet.slots[1].tried_at = Some(975);
        st.fleet.slots[2].tried_at = Some(960);
    }
    // 30 s (3/4 of 40) since pro's last read: only pro is due.
    event(&mut m, Event::ModuleChanged { module: ID });
    settle(&m);
    assert_eq!(tried(&m), [None, Some(975), Some(1_000)]);
    // The fleet view on screen: every 15 s.
    let mut m = sys(FLEET, Hooks { visible: || true, ..HOOKS });
    event(&mut m, Event::Started);
    m.fleet_view = Some("fleet");
    m.shared.lock().fleet.slots[1].tried_at = Some(986);
    event(&mut m, Event::ModuleChanged { module: ID });
    settle(&m);
    assert_eq!(tried(&m), [None, Some(1_000), Some(1_000)]);
    // A reload keeps the timer to started modules and the machines' data.
    m.configure(&parse(&config).unwrap().section(ID).unwrap().unwrap()).unwrap();
    assert_eq!(m.refresh_secs, 40);
    assert_eq!(tried(&m), [None, Some(1_000), Some(1_000)]);
}

mod actions;
mod launcher;
mod menu;
mod window;
