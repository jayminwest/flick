use std::cell::RefCell;

use super::*;
use crate::core::test_cx;

thread_local! {
    static OPENED: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
    static COPIED: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}

fn help() -> Help {
    Help {
        hooks: Hooks {
            open: |url| OPENED.with_borrow_mut(|o| o.push(url.into())),
            copy: |text| COPIED.with_borrow_mut(|c| c.push(text.into())),
        },
        off: vec![],
    }
}

fn id(key: &str) -> ItemId {
    ItemId::new(ID, key)
}

fn view(h: &mut Help, name: &str, query: &str) -> Vec<Item> {
    test_cx(query, |cx| {
        let mut v = h.open(name, cx).unwrap();
        h.refresh(&mut v, cx);
        v.items
    })
}

#[test]
fn the_root_item_opens_the_topics() {
    let mut h = help();
    let items = test_cx("", |cx| h.items(cx));
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].id.as_str(), items[0].title.as_str()), ("help:index", "Flick Help"));
    assert!(items[0].keywords[1].contains("Draw on Screen"));
    let out = test_cx("", |cx| h.activate(&id("index"), cx));
    assert!(matches!(out, Outcome::Push(v) if v.module == "help" && v.name == "topics"));
}

#[test]
fn topics_list_every_feature_in_order_and_search() {
    let mut h = help();
    let all = view(&mut h, "topics", "");
    assert_eq!(all.len(), TOPICS.len());
    assert_eq!(all[0].id.as_str(), "help:topic/launcher");
    assert_eq!(all[8].id.as_str(), "help:topic/draw");
    assert_eq!(all[8].verb, "Show Keys");
    assert_eq!(all[0].verb, "Open Docs");
    let draw = view(&mut h, "topics", "draw");
    assert_eq!(draw[0].title, "Draw on Screen");
    assert!(test_cx("", |cx| h.open("nope", cx)).is_none());
    assert!(test_cx("", |cx| h.open("keys/launcher", cx)).is_none());
    assert!(test_cx("", |cx| h.open("keys/nope", cx)).is_none());
}

#[test]
fn draw_shows_its_keys_and_key_rows_open_the_docs() {
    let mut h = help();
    let out = test_cx("", |cx| h.activate(&id("topic/draw"), cx));
    assert!(matches!(&out, Outcome::Push(v) if v.name == "keys/draw"));
    let keys = view(&mut h, "keys/draw", "");
    assert_eq!(keys[0].id.as_str(), "help:key/draw/0");
    assert_eq!(keys[0].accessory, "A");
    assert!(keys.iter().any(|k| k.accessory == "1-5"));
    let undo = view(&mut h, "keys/draw", "undo");
    assert_eq!(undo[0].accessory, "⌘Z");
    let out = test_cx("", |cx| h.activate(&undo[0].id, cx));
    assert!(matches!(out, Outcome::Hide));
    assert_eq!(OPENED.with_borrow(|o| o.last().cloned()).unwrap(), format!("{}capture.md", topics::DOCS));
}

#[test]
fn a_topic_without_keys_opens_its_docs_page() {
    let mut h = help();
    let out = test_cx("", |cx| h.activate(&id("topic/feedback"), cx));
    assert!(matches!(out, Outcome::Hide));
    assert!(OPENED.with_borrow(|o| o.last().is_some_and(|u| u.ends_with("docs/feedback.md"))));
    assert!(matches!(test_cx("", |cx| h.activate(&id("topic/nope"), cx)), Outcome::Stay(None)));
}

#[test]
fn actions_open_docs_or_copy_the_how_to() {
    let mut h = help();
    let keys = |h: &mut Help, k: &str| {
        test_cx("", |cx| h.actions(&id(k), cx)).into_iter().map(|a| a.key).collect::<Vec<_>>()
    };
    assert_eq!(keys(&mut h, "topic/draw"), ["docs", "copy"]);
    assert_eq!(keys(&mut h, "key/draw/3"), ["docs", "copy"]);
    assert!(keys(&mut h, "index").is_empty());
    let out = test_cx("", |cx| h.act(&id("topic/draw"), "copy", cx));
    assert!(matches!(out, Outcome::Stay(Some(s)) if s == "Copied Draw on Screen"));
    assert!(COPIED.with_borrow(|c| c.last().is_some_and(|t| t.contains("⌘Z  Undo"))));
    let out = test_cx("", |cx| h.act(&id("topic/draw"), "docs", cx));
    assert!(matches!(out, Outcome::Hide));
    assert!(matches!(test_cx("", |cx| h.act(&id("topic/draw"), "x", cx)), Outcome::Stay(None)));
    assert!(matches!(test_cx("", |cx| h.act(&id("index"), "docs", cx)), Outcome::Stay(None)));
}

#[test]
fn unknown_rows_rank_last() {
    let item = Item::new(id("other"), "x", "Open", Icon::symbol("x"));
    assert!(items_pos(&item, 3) < items_pos(&Item::new(id("key/draw/2"), "y", "Open", Icon::symbol("x")), 3));
    assert!(help().topic_of("key/").is_none());
}

#[test]
fn topics_of_modules_that_are_off_are_hidden() {
    let mut h = Help { off: vec!["kota", "llm", "dictation"], ..help() };
    let slugs = |items: &[Item]| items.iter().map(|i| i.id.key().trim_start_matches(TOPIC).to_string()).collect::<Vec<_>>();
    let shown = slugs(&view(&mut h, "topics", ""));
    for gone in ["kota", "chat", "llm", "dictation"] {
        assert!(!shown.iter().any(|s| s == gone), "{gone} in {shown:?}");
    }
    assert!(shown.iter().any(|s| s == "message") && shown.len() == TOPICS.len() - 4);
    let keywords = test_cx("", |cx| h.items(cx))[0].keywords.join(" ");
    assert!(!keywords.contains("KOTA") && !keywords.contains("Local Model"), "{keywords}");
    assert!(test_cx("", |cx| h.actions(&id("topic/kota"), cx)).is_empty());
    // On by default: `Help::new` with nothing off lists every topic.
    assert_eq!(slugs(&view(&mut Help::new(vec![]), "topics", "")).len(), TOPICS.len());
}
