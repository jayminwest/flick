use super::*;
use crate::core::{Module, test_cx};
use crate::modules::ext::proto::{self, Op};
use crate::modules::ext::tests::{answered, entry, module, seen};

fn form_reply() -> Reply {
    let json = r#"{"title": "New Task", "submit": "Create", "fields": [
        {"key": "title", "label": "Title", "required": true, "placeholder": "What"},
        {"key": "notes", "label": "Notes", "value": "a", "multiline": true}]}"#;
    proto::parse(Op::Form, json.as_bytes()).unwrap().reply
}

fn open(m: &mut Ext) -> Form {
    test_cx("", |cx| m.form("echo/new", cx)).unwrap()
}

fn refresh(m: &mut Ext, form: &mut Form) -> Option<String> {
    test_cx("", |cx| m.refresh_form(form, cx))
}

/// The fixture's form `echo/new`, loaded and on the screen.
fn loaded(m: &mut Ext) -> Form {
    let mut form = open(m);
    // False when it was cached already: the answer only updates the cache.
    answered(m, 1);
    assert_eq!(refresh(m, &mut form), None);
    form
}

#[test]
fn a_form_loads_then_opens_from_its_cache() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    assert!(test_cx("", |cx| m.form("nope/new", cx)).is_none());
    assert!(test_cx("", |cx| m.form("echo", cx)).is_none());
    let mut form = open(&mut m);
    assert!(form.is("ext", "echo/new") && form.background && form.fields.is_empty());
    assert_eq!((form.title.as_str(), form.pending.as_deref()), ("echo", Some(LOADING)));
    // A submit is refused while it loads.
    assert_eq!(m.submit_form(&form).unwrap_err(), "echo: new is not ready");
    assert!(answered(&mut m, 1));
    assert_eq!(refresh(&mut m, &mut form), None);
    assert_eq!((form.title.as_str(), form.submit_label.as_str(), form.pending.as_deref()), ("Echo", "Send", None));
    assert!(form.fields[0].required && form.background && form.fields.len() == 1);
}

#[test]
fn a_cached_form_opens_filled_and_a_later_answer_only_updates_the_cache() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    loaded(&mut m);
    assert!(!m.loaded("echo", "new", Ok(form_reply())));
    let again = open(&mut m);
    assert_eq!((again.fields.len(), again.pending.as_deref()), (2, None));
    let keys: Vec<&str> = again.fields.iter().map(|f| &*f.key).collect();
    assert_eq!(keys, ["title", "notes"]);
    assert!(again.fields[1].multiline);
    assert_eq!((again.fields[0].placeholder.as_str(), again.fields[1].value.as_str()), ("What", "a"));
    assert!(!answered(&mut m, 1));
    assert!(!m.loaded("echo", "new", Ok(Reply::Ack)));
    assert!(seen().is_empty());
}

#[test]
fn a_failed_load_toasts_and_shows_in_the_form() {
    let mut m = module(&entry("echo", &["fail"], 2000));
    let mut form = open(&mut m);
    assert!(answered(&mut m, 1));
    assert_eq!(seen(), ["log echo: bad token", "toast echo | echo: bad token"]);
    assert_eq!(refresh(&mut m, &mut form), None);
    assert_eq!((form.pending.as_deref(), form.error.as_deref()), (None, Some("echo: bad token")));
    assert_eq!(refresh(&mut m, &mut form), None);
    assert!(m.submit_form(&form).is_err());
}

#[test]
fn a_submit_stays_pending_until_the_reply_closes_it() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    let mut form = loaded(&mut m);
    form.set_value(0, "Buy milk");
    assert_eq!(test_cx("", |cx| m.submit(&form, cx)).as_deref(), Ok(SAVING));
    form.pending = Some(SAVING.into());
    assert_eq!(refresh(&mut m, &mut form), None);
    assert!(answered(&mut m, 1));
    assert_eq!(refresh(&mut m, &mut form).as_deref(), Some("submit done"));
    assert!(m.form.is_none() && seen().is_empty());
    // A form not on the screen does not refresh.
    assert_eq!(refresh(&mut m, &mut form), None);
}

#[test]
fn a_failed_submit_shows_in_the_form_and_toasts() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    let mut form = loaded(&mut m);
    m.hooks.job.run = |_, _, _| Err("echo: bad token".into());
    m.workers.clear();
    test_cx("", |cx| m.submit(&form, cx)).unwrap();
    assert!(answered(&mut m, 1));
    assert_eq!(seen(), ["log echo: bad token", "toast echo | echo: bad token"]);
    assert_eq!(refresh(&mut m, &mut form), None);
    assert_eq!(form.error.as_deref(), Some("echo: bad token"));
    // It may be submitted again.
    assert!(m.submit_form(&form).is_ok());
}

#[test]
fn a_reply_after_the_form_left_or_the_launcher_hid_toasts() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    let form = loaded(&mut m);
    m.submit_form(&form).unwrap();
    test_cx("", |cx| m.form_closed(&Form::new("ext", "echo/other", "x"), cx));
    assert!(m.form.is_some());
    test_cx("", |cx| m.form_closed(&form, cx));
    assert!(!answered(&mut m, 1));
    assert_eq!(seen(), ["toast echo | submit done"]);
    // Hidden: the status toasts, and a refresh asks for the root items again.
    let mut form = loaded(&mut m);
    m.submit_form(&form).unwrap();
    assert!(answered(&mut m, 1));
    m.form = Some(("echo/new".into(), Phase::Saving));
    m.hooks.visible = || false;
    let done = Done { status: None, refresh: true };
    assert!(m.saved("echo", "new", Ok(Reply::Done(done))));
    assert_eq!(seen(), [format!("toast echo | {SAVED}")]);
    assert_eq!(m.pending, [("echo".to_string(), Request::Root)]);
    assert_eq!(refresh(&mut m, &mut form).as_deref(), Some(SAVED));
    assert!(!m.saved("echo", "new", Ok(Reply::Ack)));
    assert_eq!(seen(), [format!("toast echo | {SAVED}")]);
    assert!(!m.saved("echo", "new", Err("echo: late".into())));
    assert_eq!(seen(), ["log echo: late", "toast echo | echo: late"]);
}

#[test]
fn an_extension_form_needs_its_extension() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    assert_eq!(m.submit_form(&Form::new("ext", "bare", "x")).unwrap_err(), "ext: not an extension form");
    let form = loaded(&mut m);
    m.specs.clear();
    assert_eq!(m.submit_form(&form).unwrap_err(), "ext: no extension \"echo\"");
}
