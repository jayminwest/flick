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
    let mut m = Snippets { texts: vec![], env: Env { now: pdt, type_text: record }, file: None };
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

thread_local! {
    static COPIED: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}

pub fn record_copy(text: &str) {
    COPIED.with(|c| c.borrow_mut().push(text.to_string()));
}

/// A module configured from `text`, writing to its own temp copy (never ~/.config).
struct Fixture {
    m: Snippets,
    path: std::path::PathBuf,
    /// Removes the temp copy when the test ends.
    _dir: crate::core::scratch::Scratch,
}

fn fixture(name: &str, text: &str) -> Fixture {
    let dir = crate::core::scratch::Scratch::new(&format!("snip-{name}"));
    let path = dir.join("config.toml");
    std::fs::write(&path, text).unwrap();
    let mut m = configured(text).unwrap();
    m.file = Some((path.clone(), None));
    Fixture { m, path, _dir: dir }
}

/// "name=text=hotkey" for each snippet a fresh load of `path` sees.
fn reloaded(path: &std::path::Path) -> Vec<String> {
    let m = configured(&std::fs::read_to_string(path).unwrap()).unwrap();
    m.texts.iter().map(|s| format!("{}={}={}", s.name, s.text, s.hotkey.clone().unwrap_or_default())).collect()
}

fn status(out: Outcome) -> Option<String> {
    let Outcome::Stay(status) = out else { panic!("{out:?}") };
    status
}

#[test]
fn browse_lists_snippets_then_create() {
    let Fixture { mut m, _dir, .. } = fixture("browse", DATE);
    let mut view = test_cx("", |cx| m.open("all", cx)).unwrap();
    assert!(test_cx("", |cx| m.open("other", cx)).is_none());
    test_cx("", |cx| m.refresh(&mut view, cx));
    let rows: Vec<_> = view.items.iter().map(|i| (i.id.as_str(), i.accessory.as_str())).collect();
    let hyper = "cmd+ctrl+alt+shift+KeyY";
    assert_eq!(rows, [("snippet:Date", hyper), ("snippet:Log Line", ""), ("snippet:", "")]);
    test_cx("log", |cx| m.refresh(&mut view, cx));
    let ids: Vec<_> = view.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["snippet:Log Line", "snippet:"]);
    let create = test_cx("", |cx| m.activate(&view.items[1].id, cx));
    assert!(matches!(create, Outcome::Form { module: "snippet", name } if name == "new"));
}

#[test]
fn create_form_appends_and_applies_at_once() {
    let Fixture { mut m, path, _dir } = fixture("create", DATE);
    let mut form = test_cx("", |cx| m.form("new", cx)).unwrap();
    let keys: Vec<_> = form.fields.iter().map(|f| &*f.key).collect();
    assert_eq!(keys, ["name", "text", "hotkey"]);
    assert!(form.fields[1].multiline);
    form.set_value(0, " Sig ");
    form.set_value(1, "Thanks,\nMe ");
    form.set_value(2, " cmd+ctrl+alt+shift+KeyS ");
    assert_eq!(test_cx("", |cx| m.submit(&form, cx)).unwrap(), "Saved snippet Sig");
    let sig = "Sig=Thanks,\nMe =cmd+ctrl+alt+shift+KeyS";
    assert_eq!(reloaded(&path)[2], sig);
    assert_eq!(m.hotkeys().len(), 2);
    assert!(std::fs::read_to_string(&path).unwrap().starts_with(DATE));
}

#[test]
fn edit_form_renames_in_place() {
    let Fixture { mut m, path, _dir } = fixture("edit", DATE);
    let mut form = test_cx("", |cx| m.form("edit/Date", cx)).unwrap();
    assert_eq!((form.value("text"), form.submit_label.as_str()), (Some("{date}"), "Save Snippet"));
    form.set_value(0, "Today");
    form.set_value(2, "");
    test_cx("", |cx| m.submit(&form, cx)).unwrap();
    assert_eq!(reloaded(&path), ["Today={date}=", "Log Line={date} {time} - ="]);
    assert!(m.hotkeys().is_empty());
    assert!(test_cx("", |cx| m.form("edit/Nope", cx)).is_none());
    assert!(test_cx("", |cx| m.form("other", cx)).is_none());
    // The snippet went away behind the open form.
    let gone = test_cx("", |cx| m.form("edit/Today", cx)).unwrap();
    m.texts.clear();
    assert_eq!(test_cx("", |cx| m.submit(&gone, cx)).unwrap_err(), "That snippet no longer exists");
}

#[test]
fn saves_are_checked() {
    let Fixture { mut m, path, _dir } = fixture("check", DATE);
    let add = |m: &mut Snippets, args: &[&str]| run(m, &[&["add"], args].concat()).unwrap_err();
    assert_eq!(add(&mut m, &["date", "x"]), "A snippet named \"date\" already exists");
    assert_eq!(add(&mut m, &[" ", "x"]), "Name is required");
    assert_eq!(add(&mut m, &["X", ""]), "Text is required");
    let used = add(&mut m, &["X", "x", "--hotkey", "cmd+alt+ctrl+shift+KeyY"]);
    assert_eq!(used, "Hotkey \"cmd+alt+ctrl+shift+KeyY\" is used by Date");
    assert!(add(&mut m, &["X", "x", "--hotkey", "cmd+Nope"]).starts_with("Hotkey \"cmd+Nope\": "));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), DATE);
    // Editing a snippet keeps its own name and hotkey.
    let form = test_cx("", |cx| m.form("edit/Date", cx)).unwrap();
    assert!(test_cx("", |cx| m.submit(&form, cx)).is_ok());
}

#[test]
fn list_add_remove_verbs() {
    let Fixture { mut m, path, _dir } = fixture("verbs", DATE);
    let list = run(&mut m, &["list"]).unwrap();
    assert_eq!(list, "Date\t{date}\tcmd+ctrl+alt+shift+KeyY\nLog Line\t{date} {time} - \t");
    assert_eq!(run(&mut m, &["add", "Hi", "hello"]).unwrap(), "Added snippet Hi");
    assert_eq!(run(&mut m, &["remove", "Date"]).unwrap(), "Removed snippet Date");
    assert_eq!(run(&mut m, &["remove", "Date"]).unwrap_err(), "No snippet named \"Date\"");
    assert_eq!(reloaded(&path), ["Log Line={date} {time} - =", "Hi=hello="]);
    for bad in [&["add", "X"][..], &["add", "X", "y", "--hot", "k"], &["remove"]] {
        assert!(run(&mut m, bad).unwrap_err().starts_with("usage: flick snippet add"));
    }
    assert!(m.verbs().contains("snippet add <name> <text>"));
}

#[test]
fn actions_type_copy_edit_delete() {
    let Fixture { mut m, _dir, .. } = fixture("actions", DATE);
    let id = ItemId::new("snippet", "Date");
    let keys: Vec<_> = test_cx("", |cx| m.actions(&id, cx)).into_iter().map(|a| a.key).collect();
    assert_eq!(keys, ["type", "copy", "edit", "delete"]);
    let nope = ItemId::new("snippet", "Nope");
    assert!(test_cx("", |cx| m.actions(&nope, cx)).is_empty());
    assert_eq!(status(test_cx("", |cx| m.act(&nope, "copy", cx))), None);
    assert_eq!(status(test_cx("", |cx| m.act(&id, "zap", cx))), None);
    assert!(matches!(test_cx("", |cx| m.act(&id, "type", cx)), Outcome::Hide));
    assert_eq!(typed(), ["2026-10-14"]);
    assert_eq!(status(test_cx("", |cx| m.act(&id, "copy", cx))).as_deref(), Some("Copied Date"));
    assert_eq!(COPIED.with(RefCell::take), ["2026-10-14"]);
    let edit = test_cx("", |cx| m.act(&id, "edit", cx));
    assert!(matches!(edit, Outcome::Form { module: "snippet", name } if name == "edit/Date"));
}

#[test]
fn delete_asks_first_then_removes() {
    let Fixture { mut m, path, _dir } = fixture("delete", DATE);
    let id = ItemId::new("snippet", "Date");
    let Outcome::Confirm(c) = test_cx("", |cx| m.act(&id, "delete", cx)) else { panic!() };
    assert_eq!((c.token.as_str(), c.rows[0].subtitle.as_str()), ("delete/Date", "{date}"));
    let done = status(test_cx("", |cx| m.confirmed(&c.token, cx)));
    assert_eq!(done.as_deref(), Some("Deleted snippet Date"));
    assert_eq!(reloaded(&path), ["Log Line={date} {time} - ="]);
    let again = status(test_cx("", |cx| m.confirmed(&c.token, cx)));
    assert_eq!(again.as_deref(), Some("No snippet named \"Date\""));
    assert_eq!(status(test_cx("", |cx| m.confirmed("other", cx))), None);
}
