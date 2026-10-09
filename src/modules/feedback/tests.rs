//! Module tests over a temp checkout per test, a fake clock and a fake front app. They never
//! touch the real checkout's feedback.jsonl.

use std::cell::RefCell;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::config::parse;
use crate::core::test_cx;

/// 2026-10-09 18:31:01 UTC.
const TS: i64 = 1_791_570_661;
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

thread_local! {
    static COPIED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// A fresh temp checkout; each test gets its own (mulch mx-231c5f).
fn checkout() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("flick-feedback-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn feedback(source: Option<&Path>, config: &str) -> Feedback {
    let mut f = Feedback {
        env: Env {
            now: || TS,
            utc_offset: |_| -25_200,
            frontmost: || Some((Some("com.apple.Safari".into()), "Safari".into())),
            copy: |text| COPIED.with(|c| c.borrow_mut().push(text.into())),
            source: source.map(Path::to_path_buf),
            home: Some(PathBuf::from("/Users/me")),
            build: SHA.into(),
        },
        settings: Settings::default(),
        front: None,
        query: None,
    };
    f.configure(&parse(config).unwrap().section("feedback").unwrap().unwrap()).unwrap();
    f
}

fn lines(path: &Path) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(path).unwrap();
    text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn run(f: &mut Feedback, json: bool, words: &[&str]) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    test_cx("", |cx| {
        cx.json = json;
        f.command(&args, cx)
    })
}

#[test]
fn root_items_and_the_keyword_row() {
    let dir = checkout();
    let mut f = feedback(Some(&dir), "");
    let items = test_cx("", |cx| f.items(cx));
    let ids: Vec<_> = items.iter().map(|i| i.id.to_string()).collect();
    assert_eq!(ids, ["feedback:new", "feedback:list"]);
    assert_eq!(items[0].title, "Add Feedback…");
    assert_eq!((items[0].subtitle.as_str(), items[0].accessory.as_str()), ("Feedback", "Command"));
    assert!(test_cx("fb", |cx| f.direct(cx)).is_empty());
    assert!(test_cx("fb   ", |cx| f.direct(cx)).is_empty());
    assert!(test_cx("fbx hi", |cx| f.direct(cx)).is_empty());
    let row = test_cx("fb  slow  start ", |cx| f.direct(cx));
    assert_eq!(row.len(), 1);
    assert_eq!((row[0].id.as_str(), row[0].id.arg()), ("feedback:save", Some("slow  start")));
    assert_eq!(row[0].title, "Save feedback: slow  start");
    let mut custom = feedback(Some(&dir), "[feedback]\nkeyword = \"note\"");
    assert!(test_cx("fb hi", |cx| custom.direct(cx)).is_empty());
    assert_eq!(test_cx("note hi", |cx| custom.direct(cx)).len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_keyword_row_saves_with_the_front_app_and_hides() {
    let dir = checkout();
    let mut f = feedback(Some(&dir), "");
    test_cx("", |cx| assert!(!f.on_event(Event::LauncherOpened, cx)));
    let id = ItemId::new("feedback", "save").with_arg("the switcher is slow");
    assert!(matches!(test_cx("fb the switcher is slow", |cx| f.activate(&id, cx)), Outcome::Hide));
    let got = lines(&dir.join("feedback.jsonl"));
    let want = serde_json::json!({
        "ts": "2026-10-09T11:31:01-07:00",
        "text": "the switcher is slow",
        "build": SHA,
        "app": "Safari",
        "bundle_id": "com.apple.Safari",
    });
    assert_eq!(got, [want]);
    let blank = ItemId::new("feedback", "save").with_arg(" ");
    let out = test_cx("", |cx| f.activate(&blank, cx));
    assert!(matches!(out, Outcome::Stay(Some(s)) if s == "Feedback is empty"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_form_saves_with_the_root_query() {
    let dir = checkout();
    let mut f = feedback(Some(&dir), "");
    let out = test_cx("add fee", |cx| f.activate(&ItemId::new("feedback", "new"), cx));
    assert!(matches!(out, Outcome::Form { module: "feedback", ref name } if name == "new"));
    assert!(test_cx("", |cx| f.form("other", cx)).is_none());
    let mut form = test_cx("", |cx| f.form("new", cx)).unwrap();
    assert_eq!((form.title.as_str(), form.fields.len()), ("Add Feedback", 1));
    assert!(form.fields[0].required && form.fields[0].multiline);
    form.set_value(0, "Tab should complete");
    assert_eq!(test_cx("", |cx| f.submit(&form, cx)), Ok("Feedback saved".into()));
    // The query is used once; the CLI adds no app.
    assert_eq!(run(&mut f, false, &["add", "from", "cli"]).unwrap(), format!("Feedback saved to {}", dir.join("feedback.jsonl").display()));
    let got = lines(&dir.join("feedback.jsonl"));
    assert_eq!(got[0]["query"], "add fee");
    assert_eq!(got[0]["text"], "Tab should complete");
    assert_eq!(got[0].get("app"), None, "no LauncherOpened yet");
    assert_eq!(got[1]["text"], "from cli");
    assert_eq!(got[1].get("query"), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn without_a_checkout_or_file_saving_refuses() {
    let mut f = feedback(None, "");
    let err = run(&mut f, false, &["add", "hi"]).unwrap_err();
    assert!(err.contains("Set [feedback] file"), "{err}");
    assert!(run(&mut f, false, &["path"]).is_err());
    let items = test_cx("", |cx| f.items(cx));
    assert!(items[0].subtitle.contains("[feedback] file"));
    let mut view = test_cx("", |cx| f.open("recent", cx)).unwrap();
    test_cx("", |cx| f.refresh(&mut view, cx));
    assert!(view.items.is_empty() && view.empty.contains("[feedback] file"));
    // An explicit file needs no checkout and gets its folder made.
    let dir = checkout();
    let file = dir.join("deep/notes.jsonl");
    let mut f = feedback(None, &format!("[feedback]\nfile = \"{}\"", file.display()));
    assert_eq!(run(&mut f, false, &["path"]).unwrap(), file.display().to_string());
    run(&mut f, false, &["add", "hi"]).unwrap();
    assert_eq!(lines(&file).len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ls_lists_newest_first_as_text_or_json() {
    let dir = checkout();
    let mut f = feedback(Some(&dir), "");
    assert_eq!(run(&mut f, false, &["ls"]).unwrap(), "");
    for text in ["one", "two\nlines", "three"] {
        run(&mut f, false, &["add", text]).unwrap();
    }
    let ts = "2026-10-09T11:31:01-07:00";
    assert_eq!(run(&mut f, false, &["ls", "--limit", "2"]).unwrap(), format!("{ts}\tthree\n{ts}\ttwo lines"));
    let all: serde_json::Value = serde_json::from_str(&run(&mut f, true, &["ls"]).unwrap()).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 3);
    assert_eq!(all[2]["text"], "one");
    let saved: serde_json::Value = serde_json::from_str(&run(&mut f, true, &["add", "four"]).unwrap()).unwrap();
    assert_eq!(saved["entry"]["text"], "four");
    assert!(saved["path"].as_str().unwrap().ends_with("feedback.jsonl"));
    assert!(run(&mut f, false, &["ls", "--limit", "x"]).unwrap_err().contains("not a number"));
    assert!(run(&mut f, false, &["ls", "x"]).unwrap_err().starts_with("usage"));
    assert!(run(&mut f, false, &["add"]).unwrap_err().starts_with("usage"));
    assert!(run(&mut f, false, &["nope"]).is_err());
    assert!(f.verbs().starts_with("feedback add"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn recent_feedback_lists_and_copies() {
    let dir = checkout();
    let mut f = feedback(Some(&dir), "");
    test_cx("", |cx| f.on_event(Event::LauncherOpened, cx));
    for text in ["alpha", "beta"] {
        f.save(text, true, None).unwrap();
    }
    let out = test_cx("", |cx| f.activate(&ItemId::new("feedback", "list"), cx));
    assert!(matches!(out, Outcome::Push(ref v) if v.is("feedback", "recent")));
    assert!(test_cx("", |cx| f.open("nope", cx)).is_none());
    let mut view = test_cx("", |cx| f.open("recent", cx)).unwrap();
    test_cx("", |cx| f.refresh(&mut view, cx));
    let titles: Vec<_> = view.items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["beta", "alpha"]);
    assert_eq!(view.items[0].subtitle, "2026-10-09T11:31:01-07:00  ·  Safari");
    test_cx("alp", |cx| f.refresh(&mut view, cx));
    assert_eq!(view.items.len(), 1);
    let out = test_cx("", |cx| f.activate(&view.items[0].id, cx));
    assert!(matches!(out, Outcome::Stay(Some(s)) if s == "Copied feedback"));
    assert_eq!(COPIED.with(RefCell::take), ["alpha"]);
    let out = test_cx("", |cx| f.activate(&ItemId::new("feedback", "entry/0"), cx));
    assert!(matches!(out, Outcome::Stay(None)));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_hotkey_opens_the_write_view() {
    let dir = checkout();
    assert!(feedback(Some(&dir), "").hotkeys().is_empty());
    let mut f = feedback(Some(&dir), "[feedback]\nhotkey = \"cmd+ctrl+alt+shift+KeyF\"");
    assert_eq!(f.hotkeys()[0].key, Ok("write".into()));
    assert!(test_cx("", |cx| f.hotkey("other", cx)).is_none());
    let mut view = test_cx("", |cx| f.hotkey("write", cx)).unwrap();
    assert!(view.escape_hides);
    assert_eq!(f.front.as_ref().map(|a| a.1.as_str()), Some("Safari"));
    test_cx(" ", |cx| f.refresh(&mut view, cx));
    assert!(view.items.is_empty());
    test_cx("crash on wake", |cx| f.refresh(&mut view, cx));
    assert_eq!(view.items[0].id.arg(), Some("crash on wake"));
    assert!(test_cx("", |cx| f.open("write", cx)).is_some());
    let mut other = ListView::new("feedback", "other");
    test_cx("x", |cx| f.refresh(&mut other, cx));
    assert!(other.items.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn settings_are_checked() {
    let table = |text: &str| parse(text).unwrap().section("feedback").unwrap().unwrap();
    let mut f = Feedback::default();
    assert!(f.configure(&table("[feedback]\nkeyword = \"\"")).unwrap_err().contains("one word"));
    assert!(f.configure(&table("[feedback]\nkeyword = \"a b\"")).is_err());
    assert!(f.configure(&table("[feedback]\nnope = 1")).is_err());
    assert_eq!(f.id(), "feedback");
}
