use serde_json::{Value, json};

use super::*;

fn ok(v: &Value) -> Parsed {
    parse(&v.to_string(), Origin::Local).unwrap()
}

fn card(extra: &Value) -> Value {
    let mut v = json!({ "id": "c1", "title": "Hello" });
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

fn blocks(blocks: &Value) -> Parsed {
    ok(&card(&json!({ "blocks": blocks })))
}

fn md(b: &Block) -> &str {
    match b {
        Block::Text { md } => md,
        other => panic!("not text: {other:?}"),
    }
}

/// Parse, serialize, parse again: the card must come back equal with no new warnings.
fn round_trips(p: &Parsed, origin: Origin) {
    let again = parse(&to_json(&p.card), origin).unwrap();
    assert_eq!(again.card, p.card);
}

#[test]
fn a_minimal_card_gets_defaults() {
    let p = ok(&json!({ "id": "c1", "title": "  Hello  " }));
    assert!(p.warnings.is_empty());
    let c = &p.card;
    assert_eq!((c.v, c.id.as_str(), c.title.as_str(), c.state), (1, "c1", "Hello", State::Open));
    assert_eq!((c.thread.as_ref(), c.reply_to.as_ref()), (None, None));
    assert!(c.blocks.is_empty() && c.actions.is_empty());
    assert_eq!(to_json(c), r#"{"v":1,"id":"c1","title":"Hello","state":"open"}"#);
    assert_eq!(plain(c), "Hello");
    assert!(!c.waits_on_user());
}

#[test]
fn structural_errors_refuse_the_card() {
    let big = format!(r#"{{"id":"c","title":"{}"}}"#, "x".repeat(MAX_BYTES));
    assert!(parse(&big, Origin::Local).unwrap_err().contains("max 16384"));
    assert!(parse("{", Origin::Local).unwrap_err().starts_with("card is not JSON"));
    assert_eq!(parse("[1]", Origin::Local).unwrap_err(), "card must be a JSON object");
    let err = |v: Value| parse(&v.to_string(), Origin::Remote).unwrap_err();
    assert_eq!(err(json!({ "title": "t" })), "card has no id");
    assert!(err(json!({ "id": "a b", "title": "t" })).starts_with("id must be 1-64"));
    assert!(err(json!({ "id": 7, "title": "t" })).starts_with("id must be"));
    assert!(err(json!({ "id": "x".repeat(65), "title": "t" })).starts_with("id must be"));
    assert_eq!(err(json!({ "id": "a" })), "card needs a non-empty title string");
    assert_eq!(err(json!({ "id": "a", "title": "  " })), "card needs a non-empty title string");
    assert_eq!(err(json!({ "id": "a", "title": 3 })), "card needs a non-empty title string");
    for v in [json!("1"), json!(1.5), json!(0), json!(-1)] {
        assert_eq!(err(card(&json!({ "v": v }))), "v must be a positive integer");
    }
    for s in [json!("closed"), json!(1)] {
        assert_eq!(err(card(&json!({ "state": s }))), "state must be open, pending, done or error");
    }
}

#[test]
fn top_level_fields_parse_and_degrade() {
    let p = ok(&card(&json!({
        "v": 2, "state": "pending", "thread": "ops", "reply_to": "k-1", "extra": 1,
        "title": "t".repeat(130),
    })));
    let c = &p.card;
    assert_eq!((c.v, c.state), (2, State::Pending));
    assert_eq!((c.thread.as_deref(), c.reply_to.as_deref()), (Some("ops"), Some("k-1")));
    assert_eq!(c.title.chars().count(), TITLE_MAX);
    assert!(c.title.ends_with('…'));
    assert_eq!(
        p.warnings,
        [
            "title cut to 120 chars",
            "v 2 is newer than this Flick (v1); read as v1",
            "unknown key \"extra\" ignored",
        ]
    );
    round_trips(&p, Origin::Local);

    let p =
        ok(&card(&json!({ "thread": "bad id", "reply_to": null, "blocks": {}, "actions": null })));
    assert_eq!((p.card.thread.as_ref(), p.card.reply_to.as_ref()), (None, None));
    assert_eq!(
        p.warnings,
        ["thread is not a valid id; ignored", "blocks must be an array; ignored"]
    );
    for (s, want) in [("open", State::Open), ("done", State::Done), ("error", State::Error)] {
        assert_eq!(ok(&card(&json!({ "state": s }))).card.state, want);
    }
}

#[test]
fn every_block_type_parses_and_round_trips() {
    let p = blocks(&json!([
        { "type": "text", "md": "**hi**" },
        { "type": "kv", "items": [{ "key": "Due", "value": "Fri" }, { "key": "n", "value": 3 }] },
        { "type": "list", "items": ["a", true], "ordered": true },
        { "type": "list", "items": ["x"] },
        { "type": "progress", "value": 0.4, "label": "Up" },
        { "type": "progress" },
        { "type": "choice", "id": "env", "label": "Where",
          "options": ["prod", { "id": "stg", "label": "Staging" }, { "id": "dev" }], "selected": "stg" },
        { "type": "choice", "id": "tags", "options": ["a", "b", "c"], "multi": true, "selected": ["a", "c"] },
        { "type": "choice", "id": "none", "options": ["a", "b"] },
        { "type": "field", "id": "note", "label": "Note", "placeholder": "say", "value": "hi", "multiline": true },
        { "type": "field", "id": "bare" },
    ]));
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let b = &p.card.blocks;
    assert_eq!(b[0], Block::Text { md: "**hi**".into() });
    assert_eq!(
        b[1],
        Block::Kv {
            items: vec![
                Pair { key: "Due".into(), value: "Fri".into() },
                Pair { key: "n".into(), value: "3".into() },
            ]
        }
    );
    assert_eq!(b[2], Block::List { items: vec!["a".into(), "true".into()], ordered: true });
    assert_eq!(b[4], Block::Progress { value: Some(0.4), label: Some("Up".into()) });
    assert_eq!(b[5], Block::Progress { value: None, label: None });
    let Block::Choice { options, selected, multi: false, .. } = &b[6] else { panic!("{:?}", b[6]) };
    assert_eq!(options[1], Opt { id: "stg".into(), label: "Staging".into() });
    assert_eq!(options[2], Opt { id: "dev".into(), label: "dev".into() });
    assert_eq!(selected, &["stg"]);
    assert_eq!(
        p.card.inputs(),
        [
            ("env".to_string(), Input::One(Some("stg".into()))),
            ("tags".to_string(), Input::Many(vec!["a".into(), "c".into()])),
            ("none".to_string(), Input::One(None)),
            ("note".to_string(), Input::Text("hi".into())),
            ("bare".to_string(), Input::Text(String::new())),
        ]
    );
    assert_eq!(
        plain(&p.card),
        "Hello\n**hi**\nDue: Fri\nn: 3\n1. a\n2. true\n• x\nUp: 40%\nProgress: …\n\
         Where: Staging\ntags: a, c\nnone: a / b\nNote: hi\nbare:"
    );
    assert_eq!(
        action::values_json(&p.card.inputs()).unwrap(),
        r#"{"bare":"","env":"stg","none":null,"note":"hi","tags":["a","c"]}"#
    );
    round_trips(&p, Origin::Local);
}

#[test]
fn bad_blocks_degrade_to_text() {
    let p = blocks(&json!([
        "loose",
        { "type": "chart", "md": "sales up" },
        { "type": "chart2", "text": "plain" },
        { "data": [1, 2] },
        { "type": "text", "md": 5 },
        { "type": "kv", "items": [{ "key": "k" }] },
        { "type": "kv", "items": [{ "value": "v" }] },
        { "type": "list", "items": [[1]] },
        { "type": "list", "items": "nope" },
        { "type": "list", "items": [], "ordered": "yes" },
        { "type": "progress", "value": "half" },
        { "type": "progress", "label": 4 },
    ]));
    let mds: Vec<&str> = p.card.blocks.iter().map(md).collect();
    assert_eq!(
        mds,
        [
            "[block] \"loose\"",
            "[chart] sales up",
            "[chart2] plain",
            "[?] {\"data\":[1,2]}",
            "[text] {\"md\":5,\"type\":\"text\"}",
            "[kv] {\"items\":[{\"key\":\"k\"}],\"type\":\"kv\"}",
            "[kv] {\"items\":[{\"value\":\"v\"}],\"type\":\"kv\"}",
            "[list] {\"items\":[[1]],\"type\":\"list\"}",
            "[list] {\"items\":\"nope\",\"type\":\"list\"}",
            "[list] {\"items\":[],\"ordered\":\"yes\",\"type\":\"list\"}",
            "[progress] {\"type\":\"progress\",\"value\":\"half\"}",
            "[progress] {\"label\":4,\"type\":\"progress\"}",
        ]
    );
    assert_eq!(p.warnings[0], "blocks[0]: not an object; shown as text");
    assert_eq!(p.warnings[1], "blocks[1] (chart): unknown type; shown as text");
    assert_eq!(p.warnings[4], "blocks[4] (text): md is not a string; shown as text");
    assert_eq!(p.warnings[5], "blocks[5] (kv): item without a value; shown as text");
    assert_eq!(p.warnings[6], "blocks[6] (kv): item without a key string; shown as text");
    assert_eq!(p.warnings[8], "blocks[8] (list): items is not a list; shown as text");
    assert_eq!(p.warnings[9], "blocks[9] (list): ordered is not true or false; shown as text");
    assert_eq!(p.warnings[11], "blocks[11] (progress): label is not a string; shown as text");
    assert_eq!(p.warnings.len(), 12);
    round_trips(&p, Origin::Local);
}

#[test]
fn long_previews_and_text_are_cut() {
    let p = blocks(&json!([
        { "type": "chart", "data": "d".repeat(400) },
        { "type": "text", "md": "m".repeat(MD_MAX + 5) },
        { "type": "chart", "md": "m".repeat(MD_MAX + 5) },
    ]));
    let b = &p.card.blocks;
    assert_eq!(md(&b[0]).chars().count(), "[chart] ".len() + PREVIEW_MAX);
    assert!(md(&b[0]).ends_with('…'));
    assert_eq!(md(&b[1]).chars().count(), MD_MAX);
    assert_eq!(md(&b[2]).chars().count(), MD_MAX);
    assert_eq!(p.warnings[1], "blocks[1] (text): md cut to 4000 chars");
}

#[test]
fn bad_inputs_degrade_to_text() {
    let p = blocks(&json!([
        { "type": "choice", "id": "a", "options": [] },
        { "type": "choice", "options": ["x"] },
        { "type": "choice", "id": "a", "options": [3] },
        { "type": "choice", "id": "a", "options": [{ "label": "x" }] },
        { "type": "choice", "id": "a", "options": ["x", "x"] },
        { "type": "choice", "id": "a", "options": [""] },
        { "type": "choice", "id": "a", "options": "x" },
        { "type": "choice", "id": "a", "options": ["x"], "multi": 1 },
        { "type": "choice", "id": "a", "options": ["x"], "selected": 1 },
        { "type": "choice", "id": "a", "options": ["x"], "selected": [1] },
        { "type": "choice", "id": "a", "options": ["x"], "label": 1 },
        { "type": "field", "id": "f", "label": 1 },
        { "type": "field", "id": "f", "placeholder": 1 },
        { "type": "field", "id": "f", "value": 1 },
        { "type": "field", "id": "f", "multiline": 1 },
        { "type": "field", "id": "ok" },
        { "type": "field", "id": "ok" },
        { "type": "choice", "id": "ok", "options": ["x"] },
    ]));
    let degraded = p.card.blocks.iter().filter(|b| matches!(b, Block::Text { .. })).count();
    assert_eq!((p.card.blocks.len(), degraded), (18, 17));
    let whys: Vec<&str> = p.warnings.iter().map(|w| w.split(": ").nth(1).unwrap()).collect();
    assert_eq!(
        whys,
        [
            "no options; shown as text",
            "missing or invalid id; shown as text",
            "option is not a string or object; shown as text",
            "option without an id; shown as text",
            "empty or duplicate option id \"x\"; shown as text",
            "empty or duplicate option id \"\"; shown as text",
            "options is not a list; shown as text",
            "multi is not true or false; shown as text",
            "selected is not a string or list; shown as text",
            "selected is not a string; shown as text",
            "label is not a string; shown as text",
            "label is not a string; shown as text",
            "placeholder is not a string; shown as text",
            "value is not a string; shown as text",
            "multiline is not true or false; shown as text",
            "duplicate input id \"ok\"; shown as text",
            "duplicate input id \"ok\"; shown as text",
        ]
    );
}

#[test]
fn over_cap_blocks_are_truncated() {
    let many: Vec<String> = (0..25).map(|i| i.to_string()).collect();
    let options: Vec<String> = (0..14).map(|i| format!("o{i}")).collect();
    let p = blocks(&json!([
        { "type": "list", "items": many },
        { "type": "kv", "items": many.iter().map(|k| json!({ "key": k, "value": 1 })).collect::<Vec<_>>() },
        { "type": "progress", "value": 1.5 },
        { "type": "progress", "value": -1 },
        { "type": "choice", "id": "c", "options": options, "selected": ["o1", "o2", "zz"] },
        { "type": "choice", "id": "m", "options": ["a", "b"], "multi": true, "selected": ["b", "a"] },
        { "type": "field", "id": "f1", "value": "v".repeat(INPUT_MAX + 1) },
        { "type": "field", "id": "f2" }, { "type": "field", "id": "f3" },
        { "type": "field", "id": "f4" }, { "type": "field", "id": "f5" },
    ]));
    let b = &p.card.blocks;
    let Block::List { items, .. } = &b[0] else { panic!() };
    assert_eq!((items.len(), items[19].as_str()), (ITEMS_MAX, "…6 more"));
    let Block::Kv { items } = &b[1] else { panic!() };
    assert_eq!(
        (items.len(), items[19].key.as_str(), items[19].value.as_str()),
        (ITEMS_MAX, "…6 more", "")
    );
    assert_eq!(b[2], Block::Progress { value: Some(1.0), label: None });
    assert_eq!(b[3], Block::Progress { value: Some(0.0), label: None });
    let Block::Choice { options, selected, .. } = &b[4] else { panic!() };
    assert_eq!((options.len(), selected.as_slice()), (OPTIONS_MAX, ["o1".to_string()].as_slice()));
    let Block::Choice { selected, .. } = &b[5] else { panic!() };
    assert_eq!(selected, &["b", "a"]);
    let Block::Field { value, .. } = &b[6] else { panic!() };
    assert_eq!(value.chars().count(), INPUT_MAX);
    assert_eq!(b.len(), 10, "the fifth field is dropped");
    assert_eq!(
        p.warnings,
        [
            "blocks[0] (list): more than 20 items; last 6 folded into one",
            "blocks[1] (kv): more than 20 items; last 6 folded into one",
            "blocks[2] (progress): value 1.5 clamped to 0..1",
            "blocks[3] (progress): value -1 clamped to 0..1",
            "blocks[4] (choice): more than 12 options; last 2 dropped",
            "blocks[4] (choice): unknown or extra selected options dropped",
            "blocks[6] (field): value cut to 2000 chars",
            "blocks[10] (field): more than 4 fields; dropped",
        ]
    );
    round_trips(&p, Origin::Local);

    let texts: Vec<Value> =
        (0..30).map(|i| json!({ "type": "text", "md": i.to_string() })).collect();
    let p = blocks(&json!(texts));
    assert_eq!(p.card.blocks.len(), BLOCKS_MAX);
    assert_eq!(md(&p.card.blocks[23]), "…7 more");
    assert_eq!(p.warnings, ["more than 24 blocks; last 7 dropped"]);
}

#[test]
fn actions_drive_the_card_helpers() {
    let p = ok(&card(&json!({
        "actions": [
            { "id": "no", "label": "Skip" },
            { "id": "bad", "label": "Bad", "do": { "launch": "x" } },
            { "id": "go", "label": "Ship", "style": "primary", "do": { "copy": "x" }, "reply": true },
        ]
    })));
    let c = &p.card;
    assert!(c.waits_on_user());
    assert_eq!(c.primary().map(|a| a.id.as_str()), Some("go"));
    assert_eq!(plain(c), "Hello\n[Skip] [Bad] [Ship]");
    assert_eq!(
        to_json(c),
        r#"{"v":1,"id":"c1","title":"Hello","state":"open","actions":[{"id":"no","label":"Skip"},{"id":"bad","label":"Bad","do":{"launch":"x"}},{"id":"go","label":"Ship","style":"primary","do":{"copy":"x"},"reply":true}]}"#
    );
    round_trips(&p, Origin::Local);
    let done = Card { state: State::Done, ..c.clone() };
    assert!(!done.waits_on_user());
    let only_disabled = Card { actions: vec![c.actions[1].clone()], ..c.clone() };
    assert!(!only_disabled.waits_on_user());
    assert_eq!(only_disabled.primary(), None);
}

#[test]
fn ids_follow_the_message_id_space() {
    assert!(valid_id("a.B_9-z"));
    assert!(valid_id(&"x".repeat(ID_MAX)));
    assert!(!valid_id(""));
    assert!(!valid_id(&"x".repeat(ID_MAX + 1)));
    assert!(!valid_id("a/b"));
    assert!(!valid_id("é"));
}

#[test]
fn clip_counts_chars_not_bytes() {
    assert_eq!(clip("héllo", 5), ("héllo".to_string(), false));
    assert_eq!(clip("héllo!", 5), ("héll…".to_string(), true));
}
