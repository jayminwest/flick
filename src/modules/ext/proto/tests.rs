use std::path::PathBuf;

use serde_json::{Value, json};

use super::item::{After, ExtAction, ExtItem, PATH_MAX, Run, SYMBOL_MAX, run_of};
use super::*;
use crate::core::card::Do;
use crate::core::{Icon, Tone};

fn reply(op: Op, v: &Value) -> Result<Parsed, String> {
    parse(op, v.to_string().as_bytes())
}

/// The items of a `root` reply with these items, and the warnings.
fn items(list: &Value) -> (Vec<ExtItem>, Vec<String>) {
    let p = reply(Op::Root, &json!({ "items": list })).unwrap();
    let Reply::Items(items) = p.reply else { panic!("not items: {:?}", p.reply) };
    (items, p.warnings)
}

/// The one warning parsing a single item `item` gives, and whether the item stayed.
fn item_warning(item: &Value) -> (bool, String) {
    let (items, w) = items(&json!([item]));
    assert_eq!(w.len(), 1, "{item}: {w:?}");
    (items.len() == 1, w[0].clone())
}

fn item_with(extra: &Value) -> Value {
    let mut item = json!({ "key": "k", "title": "T" });
    item.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    item
}

fn action_with(extra: &Value) -> Value {
    let mut a = json!({ "key": "a", "title": "A" });
    a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    item_with(&json!({ "actions": [a] }))
}

fn open() -> ExtAction {
    ExtAction {
        key: "open".into(),
        title: "Open".into(),
        icon: None,
        run: Run::Ext,
        after: After::Hide,
        confirm: None,
    }
}

#[test]
fn every_request_serializes() {
    let act = Request::Act { view: None, item: "i".into(), action: "a".into() };
    let submit = Request::Submit { form: "f".into(), values: vec![("t".into(), "x\ny".into())] };
    let cases = [
        (Request::Root, json!({ "op": "root" })),
        (Request::Open { view: "inbox".into() }, json!({ "op": "open", "view": "inbox" })),
        (
            Request::Query { view: "inbox".into(), query: "q".into() },
            json!({ "op": "query", "view": "inbox", "query": "q" }),
        ),
        (
            Request::Keyword { keyword: "gh".into(), query: "rest".into() },
            json!({ "op": "keyword", "keyword": "gh", "query": "rest" }),
        ),
        (act, json!({ "op": "act", "view": null, "item": "i", "action": "a" })),
        (
            Request::Act { view: Some("v".into()), item: "i".into(), action: "a".into() },
            json!({ "op": "act", "view": "v", "item": "i", "action": "a" }),
        ),
        (Request::Form { form: "new".into() }, json!({ "op": "form", "form": "new" })),
        (submit, json!({ "op": "submit", "form": "f", "values": { "t": "x\ny" } })),
    ];
    for (req, mut want) in cases {
        want["v"] = json!(1);
        let got: Value = serde_json::from_str(&req.to_json()).unwrap();
        assert_eq!(got, want);
        assert_eq!(req.op().name(), want["op"]);
    }
    let kinds = [
        (EventKind::Started, "started"),
        (EventKind::Reloaded, "reloaded"),
        (EventKind::Wake, "wake"),
        (EventKind::Unlocked, "unlocked"),
        (EventKind::LauncherOpened, "launcher_opened"),
    ];
    for (kind, name) in kinds {
        let got: Value = serde_json::from_str(&Request::Event(kind).to_json()).unwrap();
        assert_eq!(got, json!({ "v": 1, "op": "event", "event": name }));
    }
}

#[test]
fn unusable_replies_are_errors() {
    let big = format!("{{\"items\":[],\"pad\":\"{}\"}}", "x".repeat(REPLY_MAX));
    assert_eq!(
        parse(Op::Root, big.as_bytes()).unwrap_err(),
        format!("reply is {} bytes, max {REPLY_MAX}", big.len())
    );
    assert!(parse(Op::Root, b"not json").unwrap_err().starts_with("reply is not JSON: "));
    let cases = [
        (json!([]), "reply is not a JSON object"),
        (json!({ "error": "not logged in" }), "not logged in"),
        (json!({ "error": 3 }), "error is not a string"),
        (json!({ "error": "a\u{1b}[31m" }), "error has a control character"),
        (json!({ "items": {} }), "items is not a list"),
        (json!({ "title": 1 }), "title is not a string"),
    ];
    for (v, want) in cases {
        assert_eq!(reply(Op::Open, &v).unwrap_err(), want, "{v}");
    }
}

#[test]
fn blank_error_and_other_versions_still_parse() {
    let p = reply(Op::Event, &json!({ "error": " ", "v": 2 })).unwrap();
    assert_eq!(p.reply, Reply::Ack);
    assert_eq!(p.warnings, ["v 2 is not 1; read as v1"]);
    let p = reply(Op::Event, &json!({ "v": 1, "error": null })).unwrap();
    assert!(p.warnings.is_empty());
}

#[test]
fn a_full_item_parses() {
    let (got, w) = items(&json!([{
        "key": "PR #12", "title": "Fix it", "subtitle": "repo", "accessory": "2h",
        "icon": "star.fill", "keywords": ["bug", "ci"], "tone": "warn", "tab": "copy",
        "actions": [
            { "key": "open", "title": "Open", "do": { "open_url": "https://x.dev" },
              "after": "stay", "confirm": "Sure?", "icon": { "symbol": "safari" } },
            { "key": "copy", "title": "Copy", "do": { "copy": "12" }, "after": "root" },
            { "key": "log", "title": "Log", "after": "hide", "confirm": " " },
        ],
    }]));
    assert!(w.is_empty(), "{w:?}");
    let want = ExtItem {
        key: "PR #12".into(),
        title: "Fix it".into(),
        subtitle: "repo".into(),
        accessory: "2h".into(),
        icon: Some(Icon::symbol("star.fill")),
        keywords: vec!["bug".into(), "ci".into()],
        tone: Tone::Warn,
        actions: vec![
            ExtAction {
                key: "open".into(),
                title: "Open".into(),
                icon: Some(Icon::symbol("safari")),
                run: Run::Card(Do::OpenUrl("https://x.dev".into())),
                after: After::Stay,
                confirm: Some("Sure?".into()),
            },
            ExtAction {
                key: "copy".into(),
                title: "Copy".into(),
                run: Run::Card(Do::Copy("12".into())),
                after: After::Root,
                ..open()
            },
            ExtAction { key: "log".into(), title: "Log".into(), ..open() },
        ],
        tab: Some("copy".into()),
    };
    assert_eq!(got, [want]);
}

#[test]
fn a_bare_item_gets_defaults() {
    let (got, w) = items(&json!([{
        "key": "k", "title": "T", "subtitle": null, "icon": null, "keywords": null,
        "tone": null, "actions": null, "tab": null,
    }]));
    assert!(w.is_empty(), "{w:?}");
    let want = ExtItem {
        key: "k".into(),
        title: "T".into(),
        subtitle: String::new(),
        accessory: String::new(),
        icon: None,
        keywords: vec![],
        tone: Tone::Neutral,
        actions: vec![open()],
        tab: None,
    };
    assert_eq!(got, [want]);
    assert_eq!(items(&Value::Null), (vec![], vec![]));
}

#[test]
fn bad_items_are_skipped_with_a_warning() {
    let long_key = "k".repeat(KEY_MAX + 1);
    let cases = [
        (json!(3), "not an object"),
        (json!({ "title": "T" }), "key must be 1-128 chars, no control chars"),
        (json!({ "key": " ", "title": "T" }), "key must be 1-128 chars, no control chars"),
        (json!({ "key": long_key, "title": "T" }), "key must be 1-128 chars, no control chars"),
        (json!({ "key": "a\u{202e}b", "title": "T" }), "key must be 1-128 chars, no control chars"),
        (json!({ "key": "k" }), "no title"),
        (json!({ "key": "k", "title": "  " }), "no title"),
        (json!({ "key": "k", "title": "a\nb" }), "title has a control character"),
        (item_with(&json!({ "subtitle": 1 })), "subtitle is not a string"),
        (item_with(&json!({ "accessory": "\u{7}" })), "accessory has a control character"),
        (item_with(&json!({ "keywords": "a b" })), "keywords is not a list"),
        (item_with(&json!({ "keywords": ["a", 1] })), "keywords must be strings"),
        (
            item_with(&json!({ "keywords": ["a\tb"] })),
            "keywords must be up to 80 chars, no control chars",
        ),
        (
            item_with(&json!({ "keywords": ["k".repeat(SHORT_MAX + 1)] })),
            "keywords must be up to 80 chars, no control chars",
        ),
        (item_with(&json!({ "actions": {} })), "items[0].actions is not a list"),
        (item_with(&json!({ "actions": [] })), "no valid action"),
        (item_with(&json!({ "tab": 1 })), "tab is not a string"),
    ];
    for (item, why) in cases {
        let (kept, w) = item_warning(&item);
        assert!(!kept, "{item}");
        assert_eq!(w, format!("items[0]: {why}; skipped"), "{item}");
    }
}

#[test]
fn items_degrade_with_a_warning() {
    let many_words: Vec<String> = (0..=KEYWORDS_MAX).map(|i| i.to_string()).collect();
    let cases = [
        (
            item_with(&json!({ "title": "t".repeat(TITLE_MAX + 1) })),
            "items[0]: title cut to 200 chars",
        ),
        (
            item_with(&json!({ "keywords": many_words })),
            "items[0]: more than 16 keywords; the rest dropped",
        ),
        (item_with(&json!({ "tone": "pink" })), "items[0]: unknown tone \"pink\"; neutral used"),
        (item_with(&json!({ "tab": "nope" })), "items[0]: tab names no action \"nope\"; ignored"),
        (
            item_with(&json!({ "icon": 3 })),
            "items[0]: icon 3 is not a symbol name or an image path; ignored",
        ),
        (
            action_with(&json!({ "after": "later" })),
            "items[0].actions[0]: unknown after \"later\"; hide used",
        ),
    ];
    for (item, want) in cases {
        let (kept, w) = item_warning(&item);
        assert!(kept, "{item}");
        assert_eq!(w, want, "{item}");
    }
    let (got, _) = items(&json!([item_with(&json!({ "title": "t".repeat(TITLE_MAX + 1) }))]));
    assert_eq!(got[0].title.chars().count(), TITLE_MAX);
    assert!(got[0].title.ends_with('…'));
    let (got, _) = items(&json!([item_with(&json!({ "keywords": (0..20).collect::<Vec<_>>() }))]));
    assert!(got.is_empty());
}

#[test]
fn every_tone_parses() {
    let tones = [
        ("neutral", Tone::Neutral),
        ("ok", Tone::Ok),
        ("warn", Tone::Warn),
        ("error", Tone::Error),
    ];
    for (name, want) in tones {
        let (got, w) = items(&json!([item_with(&json!({ "tone": name }))]));
        assert_eq!((got[0].tone, w.len()), (want, 0), "{name}");
    }
}

#[test]
fn icons_are_symbols_or_absolute_image_paths() {
    let good = [
        (json!("star.fill"), Icon::symbol("star.fill")),
        (json!({ "symbol": "1.circle" }), Icon::symbol("1.circle")),
        (json!({ "file": "/tmp/a.PNG" }), Icon::File(PathBuf::from("/tmp/a.PNG"))),
        (json!({ "file": "/x/y.icns" }), Icon::File(PathBuf::from("/x/y.icns"))),
    ];
    for (icon, want) in good {
        let (got, w) = items(&json!([item_with(&json!({ "icon": icon }))]));
        assert_eq!((got[0].icon.clone(), w.len()), (Some(want), 0), "{icon}");
    }
    let long = format!("/{}.png", "a".repeat(PATH_MAX));
    let bad = [
        json!(""),
        json!("Star"),
        json!("star fill"),
        json!("s".repeat(SYMBOL_MAX + 1)),
        json!({ "symbol": 1 }),
        json!({ "symbol": "a", "file": "/a.png" }),
        json!({ "url": "https://x.dev/a.png" }),
        json!({ "file": "a.png" }),
        json!({ "file": "/a.txt" }),
        json!({ "file": "/noext" }),
        json!({ "file": "/a\u{0}.png" }),
        json!({ "file": long }),
        json!([]),
    ];
    for icon in bad {
        let (kept, w) = item_warning(&item_with(&json!({ "icon": icon })));
        assert!(kept && w.ends_with("not a symbol name or an image path; ignored"), "{icon}");
    }
}

#[test]
fn do_is_the_card_vocabulary_without_shell_plus_ext_push_form() {
    let good = [
        (json!("ext"), Run::Ext),
        (json!({ "ext": true }), Run::Ext),
        (json!({ "push": "inbox" }), Run::Push("inbox".into())),
        (json!({ "form": "new-task" }), Run::Form("new-task".into())),
        (json!({ "copy": "x" }), Run::Card(Do::Copy("x".into()))),
        (json!("dismiss"), Run::Card(Do::Dismiss)),
        (
            json!({ "flick": ["task", "ls"] }),
            Run::Card(Do::Flick(vec!["task".into(), "ls".into()])),
        ),
    ];
    for (v, want) in good {
        assert_eq!(run_of(&v).unwrap(), want, "{v}");
    }
    let bad = [
        (json!({ "shell": "make" }), "shell is not allowed: an extension runs commands in act"),
        (json!("shell"), "shell is not allowed: an extension runs commands in act"),
        (json!({ "push": "a b" }), "push needs a name of 1-64 of A-Z a-z 0-9 . _ -"),
        (json!({ "form": 1 }), "form needs a name of 1-64 of A-Z a-z 0-9 . _ -"),
        (json!({ "open_url": "file:///etc" }), "open_url: only http and https links"),
        (json!({ "flick": ["config"] }), "flick: config is not a module"),
        (json!({ "launch": "x" }), "unknown action \"launch\""),
        (json!(1), "do must be an object with one key"),
    ];
    for (v, want) in bad {
        assert_eq!(run_of(&v).unwrap_err(), want, "{v}");
    }
}

#[test]
fn bad_actions_are_skipped_with_a_warning() {
    let cases = [
        (json!(1), "not an object"),
        (json!({ "title": "A" }), "key must be 1-64 of A-Z a-z 0-9 . _ -"),
        (json!({ "key": "a b", "title": "A" }), "key must be 1-64 of A-Z a-z 0-9 . _ -"),
        (json!({ "key": "a" }), "no title"),
        (
            json!({ "key": "a", "title": "A", "do": { "shell": "rm" } }),
            "shell is not allowed: an extension runs commands in act",
        ),
        (
            json!({ "key": "a", "title": "A", "confirm": "\u{85}" }),
            "confirm has a control character",
        ),
    ];
    for (a, why) in cases {
        let item = item_with(&json!({ "actions": [a, { "key": "ok", "title": "Ok" }] }));
        let (got, w) = items(&json!([item]));
        assert_eq!(w, [format!("items[0].actions[0]: {why}; skipped")], "{a}");
        assert_eq!(got[0].actions.len(), 1);
    }
    let dup = item_with(
        &json!({ "actions": [{ "key": "a", "title": "A" }, { "key": "a", "title": "B" }] }),
    );
    let (got, w) = items(&json!([dup]));
    assert_eq!(w, ["items[0].actions[1]: duplicate key \"a\"; skipped"]);
    assert_eq!(got[0].actions[0].title, "A");
    let many: Vec<Value> =
        (0..=ACTIONS_MAX).map(|i| json!({ "key": format!("a{i}"), "title": "A" })).collect();
    let (got, w) = items(&json!([item_with(&json!({ "actions": many }))]));
    assert_eq!(got[0].actions.len(), ACTIONS_MAX);
    assert_eq!(w, ["more than 16 items[0].actions; items[0].actions[16] and later dropped"]);
    let (got, _) = items(&json!([action_with(&json!({ "do": null, "after": null }))]));
    assert_eq!((got[0].actions[0].run.clone(), got[0].actions[0].after), (Run::Ext, After::Hide));
}

#[test]
fn item_lists_are_capped_and_deduplicated() {
    let many: Vec<Value> =
        (0..=ITEMS_MAX).map(|i| json!({ "key": i.to_string(), "title": "T" })).collect();
    let (got, w) = items(&json!(many));
    assert_eq!(got.len(), ITEMS_MAX);
    assert_eq!(w, ["more than 500 items; items[500] and later dropped"]);
    let (got, w) = items(&json!([{ "key": "k", "title": "A" }, { "key": "k", "title": "B" }]));
    assert_eq!((got.len(), got[0].title.as_str()), (1, "A"));
    assert_eq!(w, ["items[1]: duplicate key \"k\"; skipped"]);
    for op in [Op::Query, Op::Keyword] {
        let p = reply(op, &json!({ "items": [{ "key": "k", "title": "T" }] })).unwrap();
        assert!(matches!(p.reply, Reply::Items(ref i) if i.len() == 1));
    }
}

#[test]
fn open_replies_a_view() {
    let p = reply(Op::Open, &json!({})).unwrap();
    assert_eq!((p.reply, p.warnings.len()), (Reply::View(View::default()), 0));
    let v = json!({ "title": "Inbox", "placeholder": "Search mail", "filter": "ext",
                    "items": [{ "key": "k", "title": "T" }] });
    let Reply::View(view) = reply(Op::Open, &v).unwrap().reply else { panic!("not a view") };
    assert_eq!(view.title.as_deref(), Some("Inbox"));
    assert_eq!(view.placeholder.as_deref(), Some("Search mail"));
    assert_eq!((view.filter, view.items.len()), (Filter::Ext, 1));
    let p = reply(Op::Open, &json!({ "filter": "flick" })).unwrap();
    assert_eq!(p.reply, Reply::View(View::default()));
    let p = reply(Op::Open, &json!({ "filter": "fuzzy" })).unwrap();
    assert_eq!(p.reply, Reply::View(View::default()));
    assert_eq!(p.warnings, ["unknown filter \"fuzzy\"; flick used"]);
}

#[test]
fn act_and_submit_reply_done() {
    for op in [Op::Act, Op::Submit] {
        let p = reply(op, &json!({ "status": "Saved", "refresh": true })).unwrap();
        assert_eq!(p.reply, Reply::Done(Done { status: Some("Saved".into()), refresh: true }));
        let p = reply(op, &json!({ "refresh": null })).unwrap();
        assert_eq!(p.reply, Reply::Done(Done::default()));
    }
    let p = reply(Op::Act, &json!({ "refresh": "yes" })).unwrap();
    assert_eq!(p.reply, Reply::Done(Done::default()));
    assert_eq!(p.warnings, ["reply: refresh is not true or false; false used"]);
    let long = "s".repeat(TEXT_MAX + 1);
    let p = reply(Op::Act, &json!({ "status": long })).unwrap();
    assert_eq!(p.warnings, ["reply: status cut to 500 chars"]);
}

#[test]
fn hidden_chars() {
    for c in [
        '\0', '\n', '\u{7f}', '\u{9b}', '\u{200b}', '\u{200f}', '\u{2028}', '\u{202e}', '\u{2066}',
        '\u{2069}',
    ] {
        assert!(hidden(c), "{c:?}");
    }
    for c in ['a', ' ', 'é', '…', '\u{2070}', '\u{2030}'] {
        assert!(!hidden(c), "{c:?}");
    }
}
