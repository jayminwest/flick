use super::*;
use crate::core::test_cx;
use testkit::HOOKS;

fn sys(_config: &str, hooks: Hooks) -> Sys {
    Sys::with_hooks(hooks)
}

fn ask(m: &mut Sys, words: &[&str], json: bool) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    test_cx("", |cx| {
        cx.json = json;
        m.command(&args, cx)
    })
}

#[test]
fn snapshot_answers_inline_the_first_time() {
    let mut m = sys("", HOOKS);
    let text = ask(&mut m, &["snapshot"], false).unwrap();
    assert!(text.starts_with("mbp-server · up 3d 13h · read 0s ago\nload     1.42 2.06 2.42 · 11 CPUs\n"), "{text}");
    let v: serde_json::Value = serde_json::from_str(&ask(&mut m, &["snapshot"], true).unwrap()).unwrap();
    assert_eq!((v["host"].as_str(), v["ncpu"].as_u64()), (Some("mbp-server"), Some(11)));
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
fn verbs_and_unknown_ones() {
    let mut m = sys("", HOOKS);
    assert_eq!(m.id(), "sys");
    assert_eq!(m.verbs(), "sys snapshot");
    assert_eq!(ask(&mut m, &["reboot"], false).unwrap_err(), "sys: unknown command \"reboot\"");
    assert_eq!(ask(&mut m, &["snapshot", "now"], false).unwrap_err(), "sys: unknown command \"snapshot\"");
    // The real module builds with the real hooks and runs nothing until asked.
    let real = Sys::default();
    let st = real.shared.lock();
    assert!(st.snapshot.is_none() && !st.probing);
    assert!(unix_now() > 1_700_000_000);
}
