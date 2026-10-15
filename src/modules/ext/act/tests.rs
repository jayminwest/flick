use super::*;
use crate::core::{Module, test_cx};
use crate::modules::ext::proto::{self, Op};
use crate::modules::ext::tests::{answer, answered, entry, module, seen};

/// A view `v` of extension `echo` with one item `n` and these actions, on the screen.
fn showing(m: &mut Ext, actions: &str) {
    let json = format!(r#"{{"items": [{{"key": "n", "title": "Note", "actions": [{actions}]}}]}}"#);
    let Reply::View(view) = proto::parse(Op::Open, json.as_bytes()).unwrap().reply else { panic!() };
    m.views.insert("echo/v".into(), view);
    m.showing = Some("echo/v".into());
}

fn id() -> ItemId {
    ItemId::new("ext", "echo/n")
}

fn act(m: &mut Ext, key: Option<&str>) -> Outcome {
    test_cx("", |cx| m.act_on(&id(), key, cx))
}

fn kind(o: &Outcome) -> String {
    match o {
        Outcome::Hide => "hide".into(),
        Outcome::Root => "root".into(),
        Outcome::Stay(s) => format!("stay {s:?}"),
        Outcome::Push(v) => format!("push {}", v.name),
        Outcome::Form { name, .. } => format!("form {name}"),
        Outcome::Confirm(c) => format!("confirm {} | {} | {}", c.token, c.title, c.label),
        Outcome::ReloadConfig => "reload".into(),
    }
}

fn footer(m: &mut Ext) -> String {
    let mut view = ListView::new("ext", "echo/v");
    test_cx("", |cx| m.refresh(&mut view, cx));
    view.footer
}

#[test]
fn enter_on_a_root_item_hides_at_once_and_its_status_toasts() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    m.fetch("echo", Request::Root);
    assert!(answered(&mut m, 1));
    let root = ItemId::new("ext", "echo/root");
    let actions = test_cx("", |cx| m.actions(&root, cx));
    assert_eq!(actions, [Action::new("open", "Open", Icon::symbol(ICON))]);
    assert_eq!(kind(&test_cx("", |cx| m.activate(&root, cx))), "hide");
    assert!(!answered(&mut m, 1));
    assert_eq!(seen(), ["toast echo | act done"]);
}

#[test]
fn stay_shows_the_action_running_then_its_status_in_the_footer() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    showing(&mut m, r#"{"key": "copy", "title": "Copy", "after": "stay", "icon": "doc"}, {"key": "x", "title": "X"}"#);
    let actions = m.actions_of(&id());
    assert_eq!(actions[0], Action::new("copy", "Copy", Icon::symbol("doc")));
    assert_eq!((&*actions[1].key, &actions[1].icon), ("x", &Icon::symbol(ICON)));
    assert_eq!(kind(&test_cx("", |cx| m.act(&id(), "copy", cx))), "stay None");
    assert_eq!(footer(&mut m), "Copy…");
    assert!(answered(&mut m, 1));
    assert_eq!((footer(&mut m), seen()), ("act done".to_string(), vec![]));
    // A reply with no status clears the footer; a reply that is not `Done` changes nothing.
    let act = || Request::Act { view: Some("v".into()), item: "n".into(), action: "copy".into() };
    m.inbox.push(answer("echo", act(), Ok(Reply::Done(Done::default()))));
    assert!(m.drain());
    assert_eq!(footer(&mut m), "echo");
    m.status = Some(("echo/v".into(), "x".into()));
    assert!(m.acted("echo", Some("v"), Ok(Reply::Ack)) && m.status.is_none());
}

#[test]
fn an_error_always_toasts_shows_in_the_footer_and_asks_again() {
    let mut m = module(&entry("echo", &["fail"], 2000));
    showing(&mut m, r#"{"key": "copy", "title": "Copy", "after": "stay"}"#);
    act(&mut m, None);
    assert!(answered(&mut m, 1));
    assert_eq!(seen(), ["log echo: bad token", "toast echo | echo: bad token"]);
    assert_eq!(footer(&mut m), "echo: bad token");
    let asked: Vec<_> = m.pending.iter().map(|(_, r)| r.clone()).collect();
    assert_eq!(asked, [Request::Root, Request::Open { view: "v".into() }]);
    // Off the screen: the status toasts, a refresh asks again.
    m.showing = None;
    let done = Done { status: Some("ok".into()), refresh: true };
    assert!(!m.acted("echo", Some("v"), Ok(Reply::Done(done))));
    assert!(!m.acted("echo", None, Ok(Reply::Done(Done::default()))));
    assert_eq!(seen(), ["toast echo | ok"]);
}

#[test]
fn after_root_and_navigation() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    showing(
        &mut m,
        r#"{"key": "r", "title": "R", "after": "root"}, {"key": "p", "title": "P", "do": {"push": "inbox"}},
           {"key": "f", "title": "F", "do": {"form": "new"}}, {"key": "d", "title": "D", "do": "dismiss"}"#,
    );
    let kinds: Vec<String> = ["r", "p", "f", "d"].iter().map(|k| kind(&act(&mut m, Some(k)))).collect();
    assert_eq!(kinds, ["root", "push echo/inbox", "form echo/new", "hide"]);
    // Root search replaced the view: the status toasts.
    test_cx("", |cx| m.closed("echo/v", cx));
    assert!(!answered(&mut m, 1));
    assert_eq!(seen(), ["toast echo | act done"]);
}

#[test]
fn a_confirm_asks_first_and_runs_on_its_token() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    showing(&mut m, r#"{"key": "del", "title": "Delete", "confirm": "Delete it?", "after": "stay"}"#);
    let asked = act(&mut m, None);
    assert_eq!(kind(&asked), "confirm echo/n#del | Delete it? | Delete");
    assert_eq!(kind(&test_cx("", |cx| m.confirmed("other", cx))), "stay None");
    assert!(m.asked.is_none());
    act(&mut m, None);
    assert_eq!(kind(&test_cx("", |cx| m.confirmed("echo/n#del", cx))), "stay None");
    assert_eq!(footer(&mut m), "Delete…");
    assert!(answered(&mut m, 1));
}

#[test]
fn card_actions_run_in_flick() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    showing(
        &mut m,
        r#"{"key": "c", "title": "C", "do": {"copy": "hi"}}, {"key": "o", "title": "O", "do": {"open_app": "Nope"}, "after": "stay"},
           {"key": "s", "title": "S", "do": {"script": {"name": "deploy", "query": "prod"}}},
           {"key": "f", "title": "F", "do": {"flick": ["fail"]}, "after": "stay"}"#,
    );
    assert_eq!(kind(&act(&mut m, Some("c"))), "hide");
    assert_eq!(kind(&act(&mut m, Some("o"))), "stay None");
    let e = "echo: open_app: no app \"Nope\"";
    let want = [r#"card {"copy":"hi"}"#, r#"card {"open_app":"Nope"}"#, &format!("log {e}"), &format!("toast echo | {e}")];
    assert_eq!(seen(), want);
    assert_eq!(footer(&mut m), "echo");
    assert_eq!(kind(&act(&mut m, Some("s"))), "hide");
    // Hidden: the status toasts.
    m.hooks.visible = || false;
    assert!(!answered(&mut m, 1));
    assert_eq!(seen(), ["toast echo | script run deploy prod"]);
    m.hooks.visible = || true;
    assert_eq!(kind(&act(&mut m, Some("f"))), "stay None");
    assert!(answered(&mut m, 1));
    assert_eq!(footer(&mut m), "echo: flick fail: exit 1");
}

#[test]
fn a_gone_item_says_so_and_asks_for_the_root_items_again() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    assert_eq!(kind(&act(&mut m, None)), "stay Some(\"echo: that item is gone; try again\")");
    assert_eq!(m.pending, [("echo".to_string(), Request::Root)]);
    showing(&mut m, r#"{"key": "a", "title": "A"}"#);
    assert_eq!(kind(&act(&mut m, Some("nope"))), "stay Some(\"echo: that item is gone; try again\")");
    assert!(m.actions_of(&ItemId::new("ext", "noslash")).is_empty());
    assert_eq!(kind(&test_cx("", |cx| m.act_on(&ItemId::new("ext", "x"), None, cx))), "stay Some(\"ext: that item is gone; try again\")");
    assert!(answered(&mut m, 1));
    // A root item of another extension while a view shows is found among its root items.
    m.root.insert("other".into(), m.views["echo/v"].items.clone());
    assert_eq!(m.actions_of(&ItemId::new("ext", "other/n")).len(), 1);
}

#[test]
fn an_action_of_a_dead_extension_reports_why() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    showing(&mut m, r#"{"key": "a", "title": "A"}"#);
    m.specs.clear();
    let e = "ext: no extension \"echo\"";
    assert_eq!(kind(&act(&mut m, None)), format!("stay Some({e:?})"));
    assert_eq!(seen(), [format!("log {e}"), format!("toast echo | {e}")]);
}
