use super::*;
use crate::config::parse;
use crate::core::test_cx;
use testkit::HOOKS;

fn sys(config: &str, hooks: Hooks) -> Sys {
    let mut m = Sys::with_hooks(hooks);
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
    assert_eq!(m.verbs(), "sys snapshot | sys services");
    assert_eq!(ask(&mut m, &["reboot"], false).unwrap_err(), "sys: unknown command \"reboot\"");
    assert_eq!(ask(&mut m, &["snapshot", "now"], false).unwrap_err(), "sys: unknown command \"snapshot\"");
    // The real module builds with the real hooks and runs nothing until asked.
    let real = Sys::default();
    let st = real.shared.lock();
    assert!(st.snapshot.is_none() && !st.probing);
    assert!(unix_now() > 1_700_000_000);
}
