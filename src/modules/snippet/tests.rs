use std::cell::RefCell;

use super::*;
use crate::config::parse;
use crate::core::test_cx;

thread_local! {
    /// Every text typed, in order.
    static TYPED: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}

fn record(text: String) {
    TYPED.with(|t| t.borrow_mut().push(text));
}

fn typed() -> Vec<String> {
    TYPED.with(|t| t.borrow_mut().drain(..).collect())
}

/// 2026-10-14 23:30 UTC, which is 16:30 PDT.
fn pdt() -> (i64, i32) {
    (1_792_020_600, -25_200)
}

const DATE: &str = "[[snippet.texts]]\nname = \"Date\"\ntext = \"{date}\"\n\
                    hotkey = \"cmd+ctrl+alt+shift+KeyY\"\n\
                    [[snippet.texts]]\nname = \"Log Line\"\ntext = \"{date} {time} - \"\n";

fn configured(text: &str) -> Result<Snippets, String> {
    let mut m = Snippets { texts: vec![], env: Env { now: pdt, type_text: record } };
    m.configure(&parse(text)?.section("snippet")?.ok_or("disabled")?)?;
    Ok(m)
}

fn run(m: &mut Snippets, args: &[&str]) -> Result<String, String> {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    test_cx("", |cx| m.command(&args, cx))
}

#[test]
fn render_fills_local_date_and_time() {
    let (ts, offset) = pdt();
    assert_eq!(render("{date}", ts, offset), "2026-10-14");
    assert_eq!(render("{date}", ts, 0), "2026-10-14");
    assert_eq!(render("{date}", ts + 3600, 0), "2026-10-15");
    assert_eq!(render("notes-{date}.md at {time}", ts, offset), "notes-2026-10-14.md at 16:30");
    assert_eq!(render("{other} {Date}", ts, offset), "{other} {Date}");
}

#[test]
fn items_show_the_text_and_enter_types_it() {
    let mut m = configured(DATE).unwrap();
    let items = test_cx("", |cx| m.items(cx));
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["snippet:Date", "snippet:Log Line"]);
    assert_eq!(items[0].subtitle, "2026-10-14");
    assert_eq!(items[1].subtitle, "2026-10-14 16:30 - ");
    assert_eq!(items[0].keywords, ["snippet"]);
    assert!(matches!(test_cx("", |cx| m.activate(&items[0].id, cx)), Outcome::Hide));
    assert_eq!(typed(), ["2026-10-14"]);
    let gone = ItemId::new("snippet", "Gone");
    assert!(matches!(test_cx("", |cx| m.activate(&gone, cx)), Outcome::Stay(None)));
    assert!(typed().is_empty());
}

#[test]
fn hotkeys_type_without_opening_the_launcher() {
    let mut m = configured(DATE).unwrap();
    let bound = m.hotkeys();
    assert_eq!(
        bound,
        [Binding { spec: "cmd+ctrl+alt+shift+KeyY".into(), key: Ok("Date".into()) }]
    );
    assert!(test_cx("", |cx| m.hotkey("Date", cx)).is_none());
    assert!(test_cx("", |cx| m.hotkey("Gone", cx)).is_none());
    assert_eq!(typed(), ["2026-10-14"]);
    let blank = "[[snippet.texts]]\nname = \"D\"\ntext = \"x\"\nhotkey = \" \"\n";
    assert!(configured(blank).unwrap().hotkeys().is_empty());
}

#[test]
fn verbs_type_or_show_a_snippet_by_name() {
    let mut m = configured(DATE).unwrap();
    assert_eq!(run(&mut m, &["show", "Log", "Line"]).unwrap(), "2026-10-14 16:30 - ");
    assert!(typed().is_empty());
    assert_eq!(run(&mut m, &["type", "Date"]).unwrap(), "typed Date: 2026-10-14");
    assert_eq!(typed(), ["2026-10-14"]);
    let err = run(&mut m, &["type", "Nope"]).unwrap_err();
    assert_eq!(err, "snippet: no snippet \"Nope\" (snippets: Date, Log Line)");
    let mut empty = configured("[snippet]").unwrap();
    let err = run(&mut empty, &["show", "Date"]).unwrap_err();
    assert_eq!(err, "snippet: no snippet \"Date\" (none in [[snippet.texts]])");
    assert_eq!(run(&mut m, &["type"]).unwrap_err(), "usage: snippet type <name>");
    assert_eq!(run(&mut m, &["paste", "Date"]).unwrap_err(), "snippet: unknown command \"paste\"");
    assert_eq!(run(&mut m, &[]).unwrap_err(), "snippet: missing command");
    assert_eq!(m.id(), "snippet");
    assert!(m.verbs().starts_with("snippet type"));
}

#[test]
fn config_needs_unique_names_and_text() {
    let one = |name: &str, text: &str| format!("[[snippet.texts]]\nname = \"{name}\"\ntext = \"{text}\"\n");
    let err = |text: &str| configured(text).err().unwrap();
    assert_eq!(err(&one(" ", "x")), "[snippet] texts: a snippet has no name");
    assert_eq!(err(&one("D", "")), "[snippet] texts: \"D\": text is empty");
    let twice = format!("{}{}", one("D", "a"), one("D", "b"));
    assert_eq!(err(&twice), "[snippet] texts: \"D\": duplicate name; keep one");
    assert!(configured("[snippet]\ntexts = 1").is_err());
}

#[test]
fn the_example_documents_every_key() {
    crate::config::example::assert_documents::<Settings>("snippet");
    crate::config::example::assert_documents::<Snippet>("snippet.texts");
}
