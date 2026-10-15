use crate::modules::ext::tests::{entry, module, seen, settle};
use super::*;
use crate::core::Module;
use crate::core::later::MAX_WAIT;
use crate::modules::ext::proto::Request;

const USAGE: &str = "usage: flick ext ls | ext refresh <name> | ext open <name> <view> | ext call <name> <json>|--stdin";

fn run(m: &mut Ext, args: &[&str]) -> Result<String, String> {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    m.run_verb(&args, false)
}

/// Run `ext call` and wait for its later answer, as the control socket does.
fn call(m: &mut Ext, args: &[&str]) -> Result<String, String> {
    let now = run(m, args);
    later::settle(now, later::take(), MAX_WAIT)
}

#[test]
fn ls_shows_each_extension_and_how_its_last_call_went() {
    assert_eq!(run(&mut module(""), &["ls"]).unwrap(), "ext: no extensions (add [[ext.extensions]] to config.toml)");
    let table = format!("{}{}", entry("echo", &["echo"], 2000), entry("broken", &["fail"], 2000));
    let mut m = module(&table);
    assert_eq!(run(&mut m, &["ls"]).unwrap(), "echo\t-\nbroken\t-");
    m.fetch("echo", Request::Root);
    m.fetch("broken", Request::Root);
    settle(&mut m);
    seen();
    let text = run(&mut m, &["ls"]).unwrap();
    let lines: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(lines[0][..3], ["echo", "root", "ok"]);
    assert!(lines[0][3].ends_with(" ms") && lines[0].len() == 4, "{text}");
    assert_eq!((lines[1][..3].to_vec(), lines[1][4]), (vec!["broken", "root", "error"], "broken: bad token"));
    let json: Value = serde_json::from_str(&m.run_verb(&["ls".into()], true).unwrap()).unwrap();
    assert_eq!(json[0]["name"], "echo");
    assert_eq!((json[0]["last"]["op"].as_str(), json[0]["last"]["ok"].as_bool()), (Some("root"), Some(true)));
    assert!(json[0]["last"]["ms"].is_u64() && json[0]["last"]["error"].is_null());
    assert_eq!(json[1]["last"]["error"], "broken: bad token");
    let fresh = module(&entry("idle", &["echo"], 2000)).run_verb(&["ls".into()], true);
    assert_eq!(fresh.unwrap(), r#"[{"name":"idle","last":null}]"#);
}

#[test]
fn refresh_asks_for_the_root_items_and_the_view_on_the_screen() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    assert_eq!(run(&mut m, &["refresh", "nope"]).unwrap_err(), "ext: no extension \"nope\"");
    m.showing = Some("echo/v".into());
    assert_eq!(run(&mut m, &["refresh", "echo", "x"]).unwrap_err(), USAGE);
    assert_eq!(run(&mut m, &["refresh", "echo"]).unwrap(), "Asked echo again");
    let want = [Request::Root, Request::Open { view: "v".into() }].map(|r| ("echo".to_string(), r));
    assert_eq!(m.pending, want);
    assert!(settle(&mut m));
}

#[test]
fn open_runs_the_view_like_its_hotkey() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    assert_eq!(run(&mut m, &["open", "echo", "inbox"]).unwrap(), "Opening echo/inbox");
    assert_eq!(seen(), ["open ext echo/inbox"]);
    assert_eq!(run(&mut m, &["open", "nope", "inbox"]).unwrap_err(), "ext: no extension \"nope\"");
    for bad in [&["open", "echo"][..], &["open", "echo", "a/b"], &["open", "echo", ""], &["open"], &["open", "echo", "v", "w"]] {
        assert_eq!(run(&mut m, bad).unwrap_err(), USAGE, "{bad:?}");
    }
    assert!(seen().is_empty() && m.workers.is_empty());
}

#[test]
fn call_sends_the_text_as_given_and_answers_with_the_raw_reply() {
    let mut m = module(&format!("{}{}", entry("echo", &["echo"], 2000), entry("err", &["error"], 2000)));
    // A request without "v" reaches the fixture as it is: it answers op form with a form.
    let form = r#"{"title":"Echo","submit":"Send","fields":[{"key":"title","label":"Title","required":true}]}"#;
    assert_eq!(call(&mut m, &["call", "echo", r#"{"op":"form"}"#]).unwrap(), form);
    // Not rewritten: the fixture finds no `"op":"` in a spaced request, so no op.
    let want = format!(
        r#"{{"v":1,"status":" done","items":[{{"key":"","title":"echo {}","subtitle":"/tmp/flick-test.sock"}}]}}"#,
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(call(&mut m, &["call", "echo", r#"{"op": "form"}"#]).unwrap(), want);
    // Not parsed: a reply proto refuses comes back as it is; a failed exit is the error.
    assert_eq!(call(&mut m, &["call", "err", "{}"]).unwrap(), r#"{"error":"no such view"}"#);
    let last = m.workers[1].last().unwrap();
    assert_eq!((last.op, last.error), ("call", None));
    assert_eq!(run(&mut m, &["call", "nope", "{}"]).unwrap_err(), "ext: no extension \"nope\"");
    for bad in ["[]", "not json", ""] {
        assert_eq!(run(&mut m, &["call", "echo", bad]).unwrap_err(), "ext call: the request must be a JSON object");
    }
    assert!(later::take().is_none());
    assert_eq!(run(&mut m, &["call", "echo"]).unwrap_err(), USAGE);
    assert!(m.inbox.len() == 0 && m.pending.is_empty());
    assert!(seen().is_empty());
}

#[test]
fn a_failed_raw_call_is_the_verb_error() {
    let mut m = module(&entry("broken", &["fail"], 2000));
    assert_eq!(call(&mut m, &["call", "broken", "{}"]), Err("broken: bad token".into()));
    let last = m.workers[0].last().unwrap();
    assert_eq!((last.op, last.error.as_deref()), ("call", Some("broken: bad token")));
    assert!(last.took < MAX_WAIT);
}

#[test]
fn unknown_verbs_and_the_verb_list() {
    let mut m = module("");
    assert_eq!(run(&mut m, &["nope"]).unwrap_err(), "ext: unknown command \"nope\"");
    assert_eq!(run(&mut m, &[]).unwrap_err(), "ext: missing command");
    assert_eq!(run(&mut m, &["refresh"]).unwrap_err(), USAGE);
    assert_eq!(run(&mut m, &["ls", "x"]).unwrap_err(), USAGE);
    assert_eq!(m.verbs(), VERBS);
    let out = crate::core::test_cx("", |cx| m.command(&["ls".into()], cx));
    assert!(out.unwrap().starts_with("ext: no extensions"));
}
