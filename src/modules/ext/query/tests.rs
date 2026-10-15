use super::*;
use crate::core::{ItemId, ListView, Module, test_cx};
use crate::modules::ext::proto::{self, Op};
use crate::modules::ext::tests::{answer, answered, configure, entry, module, seen};

fn query(q: &str) -> Request {
    Request::Query { view: "v".into(), query: q.into() }
}

fn keyword(q: &str) -> Request {
    Request::Keyword { keyword: "e".into(), query: q.into() }
}

/// View `v` of extension `echo` on the screen, searched by the extension; its own item `a`.
fn showing(m: &mut Ext) {
    let json = r#"{"filter": "ext", "items": [{"key": "a", "title": "A"}]}"#;
    let Reply::View(view) = proto::parse(Op::Open, json.as_bytes()).unwrap().reply else { panic!() };
    m.views.insert("echo/v".into(), view);
    m.showing = Some("echo/v".into());
}

/// The ids view `echo/v` shows for `q`.
fn shown(m: &mut Ext, q: &str) -> Vec<String> {
    let mut view = ListView::new("ext", "echo/v");
    test_cx(q, |cx| m.refresh(&mut view, cx));
    view.items.iter().map(|i| i.id.to_string()).collect()
}

fn direct(m: &mut Ext, q: &str) -> Vec<String> {
    test_cx(q, |cx| m.direct(cx)).iter().map(|i| i.id.to_string()).collect()
}

#[test]
fn an_ext_view_asks_for_each_text_and_shows_the_newest_answer() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    showing(&mut m);
    assert_eq!(shown(&mut m, ""), ["ext:echo/a"]);
    assert!(m.sent.is_empty());
    // Until the answer lands the view keeps its own items; the same text is asked once.
    assert_eq!(shown(&mut m, "x"), ["ext:echo/a"]);
    assert_eq!(shown(&mut m, "x"), ["ext:echo/a"]);
    assert_eq!(*m.sent_to("echo").unwrap(), query("x"));
    assert!(answered(&mut m, 1));
    assert_eq!(shown(&mut m, "x"), ["ext:echo/query"]);
    let found = ItemId::new("ext", "echo/query");
    assert_eq!(m.actions_of(&found).len(), 1);
    // A newer text shows the last answer until its own lands.
    assert_eq!(shown(&mut m, "xy"), ["ext:echo/query"]);
    assert!(answered(&mut m, 1));
    assert_eq!(m.found[0].1, query("xy"));
    // A blank text stops the query: the view's own items again.
    assert_eq!(shown(&mut m, " "), ["ext:echo/a"]);
    assert!(m.sent.is_empty());
    assert!(m.actions_of(&found).is_empty());
}

#[test]
fn only_an_answer_to_the_newest_query_counts_and_failures_are_not_posted() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    showing(&mut m);
    m.sent = vec![("echo".into(), query("new"))];
    m.inbox.push(answer("echo", query("old"), Ok(Reply::Items(vec![]))));
    assert!(!m.drain());
    assert!(m.found.is_empty());
    m.inbox.push(answer("echo", query("new"), Ok(Reply::Ack)));
    assert!(!m.drain());
    // A failure shows in the footer of the view on the screen; an answer clears it.
    m.inbox.push(answer("echo", query("new"), Err("echo: boom")));
    assert!(m.drain());
    assert_eq!(m.error, Some(("echo/v".into(), "echo: boom".into())));
    m.inbox.push(answer("echo", query("new"), Ok(Reply::Items(vec![]))));
    assert!(m.drain());
    assert_eq!((m.error.as_ref(), m.found[0].2.len()), (None, 0));
    // Off the screen, and for a keyword, it is only logged.
    m.showing = None;
    m.inbox.push(answer("echo", query("new"), Err("echo: gone")));
    assert!(!m.drain());
    m.sent = vec![("echo".into(), keyword("q"))];
    m.inbox.push(answer("echo", keyword("q"), Err("echo: kw")));
    assert!(!m.drain());
    assert_eq!(seen(), ["log echo: boom", "log echo: gone", "log echo: kw"]);
}

#[test]
fn a_keyword_and_text_ask_its_extension_for_direct_rows() {
    let table = format!("{}keywords = [\"e\"]\n{}", entry("echo", &["echo"], 2000), entry("other", &["echo"], 2000));
    let mut m = module(&table);
    for q in ["", "e", "e  ", "x foo", "echo foo"] {
        assert!(direct(&mut m, q).is_empty(), "{q}");
    }
    assert!(m.sent.is_empty());
    assert!(direct(&mut m, "e foo").is_empty());
    assert_eq!(*m.sent_to("echo").unwrap(), keyword("foo"));
    assert!(answered(&mut m, 1));
    assert_eq!(direct(&mut m, "e foo "), ["ext:echo/keyword"]);
    // Its rows run actions in root search.
    let row = ItemId::new("ext", "echo/keyword");
    assert_eq!(m.actions_of(&row).len(), 1);
    // Another text stops the query.
    assert!(direct(&mut m, "hello there").is_empty());
    assert!(m.sent.is_empty() && m.actions_of(&row).is_empty());
}

#[test]
fn leaving_the_view_or_a_reload_stops_the_query() {
    let table = entry("echo", &["echo"], 2000);
    let mut m = module(&table);
    showing(&mut m);
    shown(&mut m, "x");
    test_cx("", |cx| m.closed("echo/v", cx));
    assert!(m.sent.is_empty());
    showing(&mut m);
    shown(&mut m, "x");
    assert!(test_cx("", |cx| m.open("echo/v", cx)).is_some());
    assert!(m.sent.is_empty());
    m.sent = vec![("echo".into(), query("y"))];
    m.found = vec![("echo".into(), query("y"), vec![])];
    configure(&mut m, &table).unwrap();
    assert!(m.sent.is_empty() && m.found.is_empty());
    // An extension no longer configured is not asked.
    m.typed("gone", "v", "x");
    assert!(m.sent.is_empty());
    assert_eq!(seen(), ["log ext: no extension \"gone\""]);
}
