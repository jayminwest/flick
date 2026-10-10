use serde_json::{Value, json};

use super::*;
use crate::core::card::{Card, Origin, Style, parse, to_json};

fn do_of(v: &Value) -> Result<Do, String> {
    Do::parse(v)
}

fn checked(v: &Value, origin: Origin) -> Result<Do, String> {
    let d = Do::parse(v)?;
    d.check(origin)?;
    Ok(d)
}

fn words(w: &[&str]) -> Do {
    Do::Flick(w.iter().map(ToString::to_string).collect())
}

fn card_with(actions: &Value, origin: Origin) -> (Card, Vec<String>) {
    let json = json!({ "id": "c", "title": "t", "actions": actions }).to_string();
    let p = parse(&json, origin).unwrap();
    (p.card, p.warnings)
}

#[test]
fn every_do_parses_and_round_trips() {
    let cases = [
        (json!({ "open_url": "https://x.dev/a" }), Do::OpenUrl("https://x.dev/a".into())),
        (json!({ "open_app": "com.apple.Safari" }), Do::OpenApp("com.apple.Safari".into())),
        (json!({ "copy": "text" }), Do::Copy("text".into())),
        (
            json!({ "script": { "name": "deploy" } }),
            Do::Script { name: "deploy".into(), query: None },
        ),
        (
            json!({ "script": { "name": "d", "query": "prod" } }),
            Do::Script { name: "d".into(), query: Some("prod".into()) },
        ),
        (
            json!({ "script": { "name": "d", "query": null } }),
            Do::Script { name: "d".into(), query: None },
        ),
        (json!({ "flick": ["task", "start", "x"] }), words(&["task", "start", "x"])),
        (json!({ "shell": "make deploy" }), Do::Shell("make deploy".into())),
        (json!({ "dismiss": true }), Do::Dismiss),
        (json!("dismiss"), Do::Dismiss),
    ];
    for (v, want) in cases {
        let got = checked(&v, Origin::Local).unwrap();
        assert_eq!(got, want, "{v}");
        assert_eq!(Do::parse(&got.to_value()).unwrap(), want);
    }
    assert!(Do::Shell("ls".into()).needs_confirm());
    assert!(!Do::Dismiss.needs_confirm());
}

#[test]
fn malformed_do_is_refused() {
    let cases = [
        (json!(3), "do must be an object with one key"),
        (json!({}), "do must be an object with one key"),
        (json!({ "copy": "a", "shell": "b" }), "do must be an object with one key"),
        (json!("open_url"), "open_url needs a string"),
        (json!({ "open_app": 1 }), "open_app needs a string"),
        (json!({ "copy": null }), "copy needs a string"),
        (json!({ "shell": [] }), "shell needs a string"),
        (json!({ "script": "deploy" }), "script needs a name"),
        (json!({ "script": { "name": "d", "query": 1 } }), "script query is not a string"),
        (json!({ "flick": "task ls" }), "flick needs a list of words"),
        (json!({ "flick": ["task", 1] }), "flick words must be strings"),
        (json!({ "launch": "x" }), "unknown action \"launch\""),
    ];
    for (v, want) in cases {
        assert_eq!(do_of(&v).unwrap_err(), want, "{v}");
    }
}

#[test]
fn policy_refuses_unsafe_targets() {
    let refused = |d: Do, origin: Origin| d.check(origin).unwrap_err();
    for url in
        ["file:///etc/passwd", "javascript:alert(1)", "https://", "https://a b", "http://a\n"]
    {
        assert_eq!(
            refused(Do::OpenUrl(url.into()), Origin::Local),
            "open_url: only http and https links"
        );
    }
    let long = format!("https://x/{}", "a".repeat(URL_MAX));
    assert_eq!(refused(Do::OpenUrl(long), Origin::Local), "open_url: only http and https links");
    assert_eq!(Do::OpenUrl("HTTP://X".into()).check(Origin::Remote), Ok(()));
    for app in [" ", "a\u{7}", &"a".repeat(APP_MAX + 1)] {
        assert_eq!(
            refused(Do::OpenApp(app.to_string()), Origin::Local),
            "open_app: empty or invalid app name"
        );
    }
    assert_eq!(Do::Copy(String::new()).check(Origin::Remote), Ok(()));
    let script = Do::Script { name: " ".into(), query: None };
    assert_eq!(refused(script, Origin::Local), "script: empty name");
    assert_eq!(refused(Do::Shell(" ".into()), Origin::Local), "shell: empty command");
    assert_eq!(
        refused(Do::Shell("x".repeat(SHELL_MAX + 1)), Origin::Local),
        "shell: command over 2000 chars"
    );
    assert_eq!(Do::Shell("x".repeat(SHELL_MAX)).check(Origin::Remote), Ok(()));
}

#[test]
fn flick_words_are_checked_and_net_policy_applies_to_remote_cards() {
    let refused = |w: &[&str], origin: Origin| words(w).check(origin).unwrap_err();
    assert_eq!(refused(&[], Origin::Local), "flick: the first word must be a module");
    assert_eq!(refused(&["--host", "x"], Origin::Local), "flick: the first word must be a module");
    assert_eq!(refused(&["events"], Origin::Local), "flick: events is a stream, not an action");
    for flag in ["--host", "--json", "--remote", "--stdin", "--host=mac"] {
        assert_eq!(
            refused(&["task", "ls", flag], Origin::Local),
            format!("flick: {flag} is not allowed in an action")
        );
    }
    // Denied over the network: a remote card may not launder these through a button.
    assert_eq!(refused(&["reload"], Origin::Remote), "flick: reload: not allowed over the network");
    assert_eq!(
        refused(&["capture", "screen"], Origin::Remote),
        "flick: capture screen: not allowed over the network"
    );
    assert_eq!(
        refused(&["remote", "on"], Origin::Remote),
        "flick: remote on: not allowed over the network"
    );
    assert_eq!(words(&["reload"]).check(Origin::Local), Ok(()));
    assert_eq!(words(&["task", "start", "x"]).check(Origin::Remote), Ok(()));
}

#[test]
fn actions_parse_with_styles_reply_and_disabled_do() {
    let (card, warnings) = card_with(
        &json!([
            { "id": "a", "label": "Reply", "reply": true },
            { "id": "b", "label": "Open", "style": "destructive", "do": { "open_url": "https://x" } },
            { "id": "c", "label": "Both", "style": "default", "do": "dismiss", "reply": true },
            { "id": "d", "label": "Odd", "style": "loud", "reply": "yes", "do": null },
            { "id": "e", "label": "Num", "style": 3 },
            { "id": "f", "label": "Reload", "do": { "flick": ["reload"] }, "reply": true },
        ]),
        Origin::Remote,
    );
    let a = &card.actions;
    assert_eq!(a.len(), 6);
    assert_eq!((a[0].kind.clone(), a[0].replies(), a[0].enabled()), (Kind::Reply, true, true));
    assert_eq!(a[1].style, Style::Destructive);
    assert_eq!(a[1].kind, Kind::Local { run: Do::OpenUrl("https://x".into()), reply: false });
    assert!(!a[1].replies() && a[1].enabled());
    assert_eq!(a[2].kind, Kind::Local { run: Do::Dismiss, reply: true });
    assert!(a[2].replies());
    assert_eq!((a[3].style, a[3].kind.clone()), (Style::Default, Kind::Reply));
    assert_eq!(a[4].style, Style::Default);
    let reason = "flick: reload: not allowed over the network".to_string();
    assert_eq!(
        a[5].kind,
        Kind::Disabled { raw: json!({ "flick": ["reload"] }), reply: true, reason }
    );
    assert!(!a[5].enabled() && !a[5].replies());
    assert_eq!(
        warnings,
        [
            "actions[3]: unknown style \"loud\"; default used",
            "actions[3]: reply is not true or false; false used",
            "actions[4]: unknown style 3; default used",
            "actions[5]: flick: reload: not allowed over the network; shown disabled",
        ]
    );
    // The normalized JSON keeps the refused `do`, so the stored card re-parses the same way.
    let again = parse(&to_json(&card), Origin::Remote).unwrap();
    assert_eq!(again.card, card);
    // The same card from this Mac allows the verb.
    let local = parse(&to_json(&card), Origin::Local).unwrap();
    assert_eq!(local.card.actions[5].kind, Kind::Local { run: words(&["reload"]), reply: true });
}

#[test]
fn bad_actions_are_dropped() {
    let mut raw = vec![
        json!("go"),
        json!({ "label": "No id" }),
        json!({ "id": "bad id", "label": "x" }),
        json!({ "id": "n" }),
        json!({ "id": "n", "label": "  " }),
        json!({ "id": "dup", "label": "One" }),
        json!({ "id": "dup", "label": "Two" }),
        json!({ "id": "long", "label": "l".repeat(LABEL_MAX + 3) }),
    ];
    raw.extend((0..6).map(|i| json!({ "id": format!("x{i}"), "label": "X" })));
    let (card, warnings) = card_with(&json!(raw), Origin::Local);
    let ids: Vec<&str> = card.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["dup", "long", "x0", "x1", "x2", "x3"]);
    assert_eq!(card.actions[1].label.chars().count(), LABEL_MAX);
    assert_eq!(
        warnings,
        [
            "actions[0]: not an object; dropped",
            "actions[1]: id must be 1-64 of A-Z a-z 0-9 . _ -; dropped",
            "actions[2]: id must be 1-64 of A-Z a-z 0-9 . _ -; dropped",
            "actions[3]: no label; dropped",
            "actions[4]: no label; dropped",
            "actions[6]: duplicate id \"dup\"; dropped",
            "actions[7]: label cut to 40 chars",
            "more than 6 actions; actions[12] and later dropped",
        ]
    );
}

#[test]
fn values_json_shapes_and_cap() {
    let inputs = vec![
        ("note".to_string(), Input::Text("ship it".into())),
        ("env".to_string(), Input::One(Some("prod".into()))),
        ("pick".to_string(), Input::One(None)),
        ("tags".to_string(), Input::Many(vec!["a".into(), "b".into()])),
    ];
    assert_eq!(
        values_json(&inputs).unwrap(),
        r#"{"env":"prod","note":"ship it","pick":null,"tags":["a","b"]}"#
    );
    assert_eq!(values_json(&[]).unwrap(), "{}");
    let long = vec![("f".to_string(), Input::Text("x".repeat(INPUT_MAX + 50)))];
    let json = values_json(&long).unwrap();
    assert_eq!(json.len(), r#"{"f":""}"#.len() + INPUT_MAX, "a field is cut to INPUT_MAX");
    let over: Vec<(String, Input)> =
        (0..3).map(|i| (format!("f{i}"), Input::Text("x".repeat(INPUT_MAX)))).collect();
    assert_eq!(values_json(&over).unwrap_err(), "values are 6025 chars, max 4000");
}
