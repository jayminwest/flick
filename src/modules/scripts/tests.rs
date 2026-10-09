use std::cell::RefCell;

use super::*;
use crate::config::parse;
use crate::core::test_cx;

thread_local! {
    /// (name, command line) of every run, in order.
    static RUNS: RefCell<Vec<(String, String)>> = const { RefCell::new(vec![]) };
}

fn record(name: &str, cmd: &str) {
    RUNS.with(|r| r.borrow_mut().push((name.into(), cmd.into())));
}

fn runs() -> Vec<(String, String)> {
    RUNS.with(|r| r.borrow_mut().drain(..).collect())
}

const KOTA: &str = "[[script.commands]]\nname = \"Ask KOTA\"\nkeyword = \"k\"\n\
                    shell = \"printf %s {query} | ssh mbp-server kota-ask\"\n\
                    [[script.commands]]\nname = \"Lock\"\nshell = \"pmset displaysleepnow\"\n";

fn configured(text: &str) -> Result<Scripts, String> {
    let mut m = Scripts { run: record, ..Scripts::default() };
    m.configure(&parse(text)?.section("script")?.ok_or("disabled")?)?;
    Ok(m)
}

fn activate(m: &mut Scripts, id: &ItemId) -> Outcome {
    test_cx("", |cx| m.activate(id, cx))
}

#[test]
fn quote_makes_one_shell_word() {
    assert_eq!(quote("hi there"), "'hi there'");
    assert_eq!(quote("it's $HOME `x` \"y\""), r#"'it'\''s $HOME `x` "y"'"#);
    assert_eq!(quote(""), "''");
}

#[test]
fn keyword_and_text_runs_the_command_with_the_text_quoted() {
    let mut m = configured(KOTA).unwrap();
    let items = test_cx("k  what's up; rm -rf ~ ", |cx| m.direct(cx));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id.as_str(), "script:Ask KOTA");
    assert_eq!(items[0].subtitle, "“what's up; rm -rf ~”");
    assert!(matches!(activate(&mut m, &items[0].id), Outcome::Hide));
    let cmd = r"printf %s 'what'\''s up; rm -rf ~' | ssh mbp-server kota-ask";
    assert_eq!(runs(), [("Ask KOTA".to_string(), cmd.to_string())]);
}

#[test]
fn direct_needs_a_known_keyword_text_and_a_query_command() {
    let mut m = configured(KOTA).unwrap();
    for q in ["k", "k ", "k   ", "x hello", "Lock now", "kk hi"] {
        assert!(test_cx(q, |cx| m.direct(cx)).is_empty(), "{q}");
    }
    let lock = "[[script.commands]]\nname = \"Lock\"\nkeyword = \"l\"\nshell = \"true\"\n";
    let mut m = configured(lock).unwrap();
    assert!(test_cx("l now", |cx| m.direct(cx)).is_empty());
}

#[test]
fn root_items_run_at_once_or_ask_for_the_argument() {
    let mut m = configured(KOTA).unwrap();
    let items = test_cx("", |cx| m.items(cx));
    let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["script:Ask KOTA", "script:Lock"]);
    assert!(items[0].tab && items[0].id.arg().is_none());
    assert_eq!((items[0].subtitle.as_str(), items[0].keywords.as_slice()), ("k", &["k".to_string()][..]));
    assert!(!items[1].tab);
    assert!(matches!(activate(&mut m, &items[1].id), Outcome::Hide));
    assert_eq!(runs(), [("Lock".to_string(), "pmset displaysleepnow".to_string())]);
    match activate(&mut m, &items[0].id) {
        Outcome::Push(view) => assert_eq!((view.module, view.name.as_str()), ("script", "Ask KOTA")),
        _ => panic!("expected the argument view"),
    }
    assert_eq!(runs().len(), 0);
}

#[test]
fn argument_view_previews_the_command_and_runs_it() {
    let mut m = configured(KOTA).unwrap();
    let mut view = test_cx("", |cx| m.open("Ask KOTA", cx)).unwrap();
    assert!(view.record_use);
    assert!(test_cx("", |cx| m.open("Nope", cx)).is_none());
    test_cx("", |cx| m.refresh(&mut view, cx));
    assert_eq!(view.items[0].subtitle, "Type an argument");
    // Enter on an empty argument does nothing.
    assert!(matches!(activate(&mut m, &view.items[0].id), Outcome::Stay(None)));
    test_cx("hi ", |cx| m.refresh(&mut view, cx));
    assert_eq!(view.items[0].subtitle, "printf %s 'hi' | ssh mbp-server kota-ask");
    assert!(matches!(activate(&mut m, &view.items[0].id), Outcome::Hide));
    assert_eq!(runs().len(), 1);
    view.name = "Gone".into();
    test_cx("hi", |cx| m.refresh(&mut view, cx));
    assert!(view.items.is_empty());
}

#[test]
fn unknown_or_oversized_runs_nothing() {
    let mut m = configured(KOTA).unwrap();
    let gone = ItemId::new("script", "Gone").with_arg("x");
    assert!(matches!(activate(&mut m, &gone), Outcome::Stay(None)));
    let long = ItemId::new("script", "Ask KOTA").with_arg("x".repeat(MAX_QUERY + 1));
    let msg = format!("Ask KOTA: argument over {MAX_QUERY} bytes");
    assert!(matches!(activate(&mut m, &long), Outcome::Stay(Some(s)) if s == msg));
    assert_eq!(runs().len(), 0);
}

#[test]
fn bad_tables_name_the_command() {
    let cmd = |name: &str, kw: &str, shell: &str| {
        format!("[[script.commands]]\nname = \"{name}\"\nkeyword = \"{kw}\"\nshell = \"{shell}\"\n")
    };
    let err = |text: &str| configured(text).err().unwrap();
    assert_eq!(err(&cmd(" ", "a", "true")), "[script] commands: a command has no name");
    assert_eq!(err(&cmd("A", "a", " ")), "[script] commands: \"A\": shell is empty");
    assert_eq!(
        err(&(cmd("A", "a", "true") + &cmd("A", "b", "true"))),
        "[script] commands: \"A\": duplicate name; keep one"
    );
    assert_eq!(err(&cmd("A", "a b", "true")), "[script] commands: \"A\": keyword must be one word");
    assert_eq!(err(&cmd("A", "", "true")), "[script] commands: \"A\": keyword must be one word");
    assert_eq!(
        err(&(cmd("A", "a", "true") + &cmd("B", "a", "true"))),
        "[script] commands: \"B\": keyword \"a\" is used twice"
    );
    assert!(err("[[script.commands]]\nname = \"A\"\n").starts_with("[script]: missing field `shell`"));
    assert_eq!(configured("").unwrap().commands.len(), 0);
    assert_eq!(Scripts::default().id(), "script");
}
