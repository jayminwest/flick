//! Module tests over an in-memory store, synthetic events and a fake clock.

use std::cell::{Cell, RefCell};

use super::*;
use crate::core::test_cx;

const T: i64 = 1_000_000;

thread_local! {
    static NOW: Cell<i64> = const { Cell::new(T) };
    /// Events posted and quit hooks asked for, in order.
    static SYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn at(t: i64) {
    NOW.with(|n| n.set(t));
}

/// The events and hooks since the last `calls()`.
fn calls() -> Vec<String> {
    SYS.with(RefCell::take)
}

fn tasks() -> Tasks {
    Tasks {
        env: Env {
            now: || NOW.with(Cell::get),
            utc_offset: |_| 0,
            post: |e| SYS.with(|s| s.borrow_mut().push(serde_json::to_string(&e).unwrap())),
            on_quit: |_| SYS.with(|s| s.borrow_mut().push("on_quit".into())),
        },
        ..Tasks::default()
    }
}

/// Run `f` with a migrated store; the fake clock starts at `T`.
fn with_cx(f: impl FnOnce(&mut Cx)) {
    at(T);
    calls();
    test_cx("", |cx| {
        cx.store.migrate("task", MIGRATIONS).unwrap();
        f(cx);
    });
}

fn run(t: &mut Tasks, cx: &mut Cx, words: &[&str]) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    t.command(&args, cx)
}

fn json(t: &mut Tasks, cx: &mut Cx, words: &[&str]) -> serde_json::Value {
    cx.json = true;
    let out = run(t, cx, words).unwrap();
    cx.json = false;
    serde_json::from_str(&out).unwrap()
}

/// Stored time rows as (start, end, task).
fn rows(cx: &Cx) -> Vec<(i64, i64, i64)> {
    cx.store.times(0, i64::MAX).into_iter().map(|s| (s.start, s.end, s.subject)).collect()
}

fn changed(task: Option<i64>) -> String {
    serde_json::to_string(&Event::TaskChanged { task }).unwrap()
}

#[test]
fn start_creates_runs_and_stop_ends_the_row() {
    with_cx(|cx| {
        let mut t = tasks();
        t.on_event(Event::Started, cx);
        assert_eq!(calls(), Vec::<String>::new());
        let out = run(&mut t, cx, &["start", "Write plan", "--project", "flick"]).unwrap();
        assert_eq!(out, "Started Write plan #flick");
        assert_eq!(calls(), ["on_quit".to_string(), changed(Some(1))]);
        assert_eq!(run(&mut t, cx, &["start", "write"]).unwrap(), "Already running: Write plan #flick");
        at(T + 300);
        t.on_event(Event::PasteboardChanged, cx); // any event advances the open row
        assert_eq!(rows(cx), [(T, T + 300, 1)]);
        at(T + 420);
        let ls = run(&mut t, cx, &["ls"]).unwrap();
        assert!(ls.starts_with("Running: Write plan (7m today)\n▶    1  doing"), "{ls}");
        assert_eq!(run(&mut t, cx, &["stop"]).unwrap(), "Stopped Write plan #flick");
        assert_eq!(calls(), [changed(None)]);
        assert_eq!(rows(cx), [(T, T + 420, 1)]);
        assert_eq!(run(&mut t, cx, &["stop"]).unwrap(), "No task running");
        at(T + 900);
        let report = run(&mut t, cx, &["report"]).unwrap();
        assert!(report.starts_with("Tasks today (1970-01-12): 7m\n       7m  1 Write plan  #flick"), "{report}");
        // Starting again after a stop counts from now.
        run(&mut t, cx, &["start", "1"]).unwrap();
        assert_eq!(rows(cx)[1], (T + 900, T + 900, 1));
    });
}

#[test]
fn starting_another_task_switches() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["start", "A"]).unwrap();
        at(T + 60);
        calls();
        assert_eq!(run(&mut t, cx, &["switch", "B"]).unwrap(), "Switched to B");
        assert_eq!(calls(), ["on_quit".to_string(), changed(Some(2))]);
        assert_eq!(rows(cx), [(T, T + 60, 1), (T + 60, T + 60, 2)]);
        // A switch within the flicker merge gives the time to the new task.
        at(T + 61);
        run(&mut t, cx, &["start", "A"]).unwrap();
        assert_eq!(rows(cx), [(T, T + 60, 1), (T + 60, T + 61, 1)]);
        let v = json(&mut t, cx, &["ls"]);
        assert_eq!((v["running"]["id"].as_i64(), v["running"]["since"].as_i64()), (Some(1), Some(T + 60)));
        assert_eq!(v["tasks"][0]["today_secs"], 61);
    });
}

#[test]
fn idle_sleep_and_lock_pause_the_running_task() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["start", "A"]).unwrap();
        at(T + 200);
        t.on_event(Event::Idle { secs: 60 }, cx);
        at(T + 300);
        t.on_event(Event::Active, cx);
        at(T + 400);
        t.on_event(Event::Sleep, cx);
        at(T + 500);
        t.on_event(Event::Wake, cx);
        at(T + 600);
        t.on_event(Event::Locked, cx);
        t.on_event(Event::Sleep, cx);
        at(T + 700);
        t.on_event(Event::Wake, cx); // still locked
        t.on_event(Event::Unlocked, cx);
        at(T + 800);
        t.on_event(Event::Unlocked, cx); // not locked: only advances the row
        assert_eq!(
            rows(cx),
            [(T, T + 140, 1), (T + 300, T + 400, 1), (T + 500, T + 600, 1), (T + 700, T + 800, 1)]
        );
    });
}

#[test]
fn a_task_started_while_away_waits_for_unlock() {
    with_cx(|cx| {
        let mut t = tasks();
        t.on_event(Event::Locked, cx);
        run(&mut t, cx, &["start", "A"]).unwrap();
        assert!(rows(cx).is_empty());
        at(T + 50);
        t.on_event(Event::Unlocked, cx);
        // Stopped, then started while asleep: waits for wake.
        at(T + 70);
        run(&mut t, cx, &["stop"]).unwrap();
        t.on_event(Event::Sleep, cx);
        run(&mut t, cx, &["start", "A"]).unwrap();
        at(T + 80);
        t.on_event(Event::Wake, cx);
        assert_eq!(rows(cx), [(T + 50, T + 70, 1), (T + 80, T + 80, 1)]);
        assert_eq!(json(&mut t, cx, &["ls"])["running"]["since"].as_i64(), Some(T + 80));
    });
}

#[test]
fn the_running_task_survives_a_restart() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["start", "A"]).unwrap();
        at(T + 100);
        t.on_event(Event::Active, cx);
        // Flick is down from T + 100 to T + 1000; the new instance takes over.
        at(T + 1000);
        calls();
        let mut fresh = tasks();
        assert!(!fresh.on_event(Event::Started, cx));
        assert_eq!(calls(), ["on_quit".to_string(), changed(Some(1))]);
        assert_eq!(rows(cx), [(T, T + 100, 1), (T + 1000, T + 1000, 1)]);
        assert_eq!(cx.store.open_row(), fresh.open);
        // Quit closed the row: the next start opens a new one; an empty row is dropped.
        cx.store.close_open(T + 1000);
        let mut again = tasks();
        again.on_event(Event::Started, cx);
        assert_eq!(rows(cx).len(), 3);
        cx.store.set_open_row(Some(99));
        cx.store.set_running(Some(42));
        tasks().on_event(Event::Started, cx);
        assert_eq!((cx.store.open_row(), rows(cx).len()), (None, 3));
    });
}

#[test]
fn done_stops_and_hides_the_task_but_reports_keep_it() {
    with_cx(|cx| {
        let mut t = tasks();
        assert_eq!(run(&mut t, cx, &["add", "Mail"]).unwrap(), "Added 1: Mail");
        assert!(run(&mut t, cx, &["add", "Mail"]).unwrap_err().contains("already"));
        run(&mut t, cx, &["start", "Review PR", "--project", "kota"]).unwrap();
        at(T + 120);
        calls();
        assert_eq!(run(&mut t, cx, &["done", "rev"]).unwrap(), "Done 2: Review PR #kota");
        assert_eq!(calls(), [changed(None)]);
        assert_eq!(run(&mut t, cx, &["done", "Mail"]).unwrap(), "Done 1: Mail");
        assert_eq!(run(&mut t, cx, &["ls"]).unwrap(), "No task running");
        assert_eq!(json(&mut t, cx, &["ls", "--all", "--project", "kota"])["tasks"][0]["status"], "done");
        let report = json(&mut t, cx, &["report", "week", "--project", "kota"]);
        assert_eq!((report["total_secs"].as_i64(), report["tasks"][0]["id"].as_i64()), (Some(120), Some(2)));
        assert!(run(&mut t, cx, &["done", "nope"]).unwrap_err().contains("no task matches"));
        assert!(run(&mut t, cx, &["report", "someday"]).unwrap_err().starts_with("task: bad range"));
    });
}

#[test]
fn json_answers_carry_the_task_and_running_id() {
    with_cx(|cx| {
        let mut t = tasks();
        let v = json(&mut t, cx, &["start", "A", "--project", "p"]);
        assert_eq!(v["message"], "Started A #p");
        assert_eq!((v["task"]["id"].as_i64(), v["running"].as_i64()), (Some(1), Some(1)));
        assert_eq!(v["task"]["status"], "doing");
        let v = json(&mut t, cx, &["stop"]);
        assert!(v["running"].is_null() && v["task"]["project"] == "p");
        let v = json(&mut t, cx, &["report", "today"]);
        assert_eq!((v["total_secs"].as_i64(), v["range"]["first"].as_str()), (Some(0), Some("1970-01-12")));
        assert!(run(&mut t, cx, &["start"]).is_err() && run(&mut t, cx, &["start", "9"]).is_err());
        assert!(t.verbs().starts_with("task start|switch"));
    });
}

fn item_ids(items: &[Item]) -> Vec<String> {
    items.iter().map(|i| i.id.to_string()).collect()
}

/// View `name` opened and refreshed for `query`.
fn list(t: &mut Tasks, cx: &mut Cx, name: &str, query: &str) -> Vec<Item> {
    let mut view = t.open(name, cx).unwrap();
    let mut cx = Cx { query, ranker: &mut *cx.ranker, ..*cx };
    t.refresh(&mut view, &mut cx);
    view.items
}

/// What an outcome shows: `hide`, `push task/<name>` or the status of `Stay`.
fn shown(outcome: Outcome) -> String {
    match outcome {
        Outcome::Hide => "hide".into(),
        Outcome::Push(v) => format!("push {}/{}", v.module, v.name),
        Outcome::Stay(text) => text.unwrap_or_default(),
        other => format!("{other:?}"),
    }
}

fn status(outcome: Outcome) -> Option<String> {
    match outcome {
        Outcome::Stay(text) => text,
        other => panic!("not Stay: {other:?}"),
    }
}

#[test]
fn the_launcher_starts_a_new_task_and_stops_it() {
    with_cx(|cx| {
        let mut t = tasks();
        assert_eq!(item_ids(&t.items(cx)), ["task:start", "task:list", "task:today"]);
        assert_eq!(shown(t.activate(&ItemId::new("task", "start"), cx)), "push task/pick");
        assert!(t.open("nope", cx).is_none());
        let items = list(&mut t, cx, "pick", "Review PR #kota");
        assert_eq!(item_ids(&items), ["task:new"]);
        assert_eq!(shown(t.activate(&items[0].id, cx)), "hide");
        assert_eq!(calls(), ["on_quit".to_string(), changed(Some(1))]);
        assert!(t.on_event(Event::TaskChanged { task: Some(1) }, cx));
        at(T + 185);
        let root = t.items(cx);
        assert_eq!(item_ids(&root), ["task:stop", "task:switch", "task:list", "task:today"]);
        assert_eq!(root[0].title, "Stop Task: Review PR · 0:03");
        assert_eq!(shown(t.activate(&root[1].id, cx)), "push task/pick");
        assert_eq!(status(t.activate(&root[0].id, cx)).as_deref(), Some("Stopped Review PR #kota"));
        assert_eq!(status(t.activate(&root[0].id, cx)).as_deref(), Some("No task running"));
    });
}

#[test]
fn launcher_rows_start_existing_tasks() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["add", "Review PR", "--project", "kota"]).unwrap();
        // The same title and project starts the existing task, even when it is done.
        cx.store.task_set_status(1, Status::Done, T);
        let new = ItemId::new("task", "new").with_arg("Review PR #kota");
        assert_eq!(shown(t.activate(&new, cx)), "hide");
        assert_eq!((t.running, cx.store.task_list(true).len()), (Some(1), 1));
        assert_eq!(status(t.activate(&ItemId::new("task", "run/9"), cx)).as_deref(), Some("task: no task 9"));
        assert_eq!(status(t.activate(&ItemId::new("task", "project/x"), cx)), None);
    });
    // Without the tables a new task cannot be added.
    test_cx("", |cx| {
        let new = ItemId::new("task", "new").with_arg("A");
        assert!(status(tasks().activate(&new, cx)).unwrap().starts_with("task: "));
    });
}

#[test]
fn today_counts_the_running_task_up_to_now() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["start", "A"]).unwrap();
        at(T + 600);
        run(&mut t, cx, &["start", "B", "--project", "p"]).unwrap();
        at(T + 720); // no event since the switch: B's row still ends at T + 600
        let items = list(&mut t, cx, "today", "");
        assert_eq!(item_ids(&items), ["task:run/2", "task:run/1", "task:project/", "task:project/p"]);
        assert_eq!((items[0].accessory.as_str(), items[1].accessory.as_str()), ("Running · 0:02", "0:10"));
        assert_eq!(shown(t.activate(&items[1].id, cx)), "hide");
        assert_eq!(t.running, Some(1));
        let pick = list(&mut t, cx, "pick", "");
        assert_eq!(item_ids(&pick), ["task:run/1", "task:run/2"]);
        assert_eq!(pick[1].accessory, "0:02");
    });
}

#[test]
fn task_rows_have_start_done_and_stop_actions() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["add", "A"]).unwrap();
        let (row, root) = (ItemId::new("task", "run/1"), ItemId::new("task", "start"));
        let keys = |t: &mut Tasks, cx: &mut Cx| t.actions(&row, cx).into_iter().map(|a| a.key).collect::<Vec<_>>();
        assert_eq!(keys(&mut t, cx), ["start", "done", "rename", "delete"]);
        assert!(t.actions(&root, cx).is_empty());
        assert!(t.actions(&ItemId::new("task", "run/7"), cx).is_empty());
        assert_eq!(shown(t.act(&row, "start", cx)), "hide");
        assert_eq!(keys(&mut t, cx), ["stop", "done", "rename", "delete"]);
        at(T + 60);
        assert_eq!(status(t.act(&row, "stop", cx)).as_deref(), Some("Stopped A"));
        assert_eq!(status(t.act(&row, "done", cx)).as_deref(), Some("Done 1: A"));
        assert_eq!(keys(&mut t, cx), ["start", "reopen", "rename", "delete"]);
        assert_eq!(status(t.act(&ItemId::new("task", "run/7"), "done", cx)).as_deref(), Some("task: no task 7"));
        assert_eq!(status(t.act(&row, "nope", cx)), None);
        assert_eq!(status(t.act(&root, "done", cx)), None);
        assert!(list(&mut t, cx, "pick", "").is_empty());
    });
}

#[test]
fn reopen_puts_a_done_task_back_in_the_picker() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["add", "A"]).unwrap();
        run(&mut t, cx, &["done", "A"]).unwrap();
        let row = ItemId::new("task", "run/1");
        assert_eq!(status(t.act(&row, "reopen", cx)).as_deref(), Some("Reopened 1: A"));
        assert_eq!(item_ids(&list(&mut t, cx, "pick", "")), ["task:run/1"]);
        assert_eq!(status(t.act(&ItemId::new("task", "run/7"), "rename", cx)).as_deref(), Some("task: no task 7"));
    });
}

#[test]
fn the_tasks_view_lists_every_task_open_ones_first() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["add", "Mail"]).unwrap();
        run(&mut t, cx, &["add", "Review", "--project", "kota"]).unwrap();
        run(&mut t, cx, &["start", "Write"]).unwrap();
        run(&mut t, cx, &["done", "Mail"]).unwrap();
        at(T + 120);
        assert_eq!(shown(t.activate(&ItemId::new("task", "list"), cx)), "push task/list");
        let items = list(&mut t, cx, "list", "");
        assert_eq!(item_ids(&items), ["task:run/3", "task:run/2", "task:run/1"]);
        assert_eq!((items[0].accessory.as_str(), items[2].accessory.as_str()), ("Running · 0:02", "Done · 0:00"));
        assert_eq!(items[1].subtitle, "#kota");
        assert_eq!(item_ids(&list(&mut t, cx, "list", "mail")), ["task:run/1"]);
        // Enter on a done task starts it again.
        assert_eq!(shown(t.activate(&items[2].id, cx)), "hide");
        assert_eq!((t.running, cx.store.task_get(1).map(|t| t.status)), (Some(1), Some(Status::Doing)));
    });
}

#[test]
fn rename_is_a_form() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["add", "Mail", "--project", "home"]).unwrap();
        run(&mut t, cx, &["add", "Write"]).unwrap();
        let row = ItemId::new("task", "run/1");
        let Outcome::Form { module: "task", name } = t.act(&row, "rename", cx) else { panic!("rename opens a form") };
        let mut form = t.form(&name, cx).unwrap();
        assert_eq!((form.title.as_str(), form.value("title"), form.value("project")), ("Rename Task", Some("Mail"), Some("home")));
        assert!(t.form("rename/9", cx).is_none() && t.form("nope", cx).is_none());
        form.set_value(0, "Write");
        form.set_value(1, "");
        assert!(t.submit(&form, cx).unwrap_err().contains("task 2 is already"));
        form.set_value(0, " Inbox ");
        form.set_value(1, "#admin");
        assert_eq!(t.submit(&form, cx).as_deref(), Ok("Renamed Inbox #admin"));
        assert!(t.submit(&Form::new("task", "other", "x"), cx).is_err());
    });
}

#[test]
fn delete_asks_first_then_drops_the_task_and_its_time() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["add", "Inbox", "--project", "admin"]).unwrap();
        let row = ItemId::new("task", "run/1");
        t.act(&row, "start", cx);
        at(T + 60);
        let Outcome::Confirm(c) = t.act(&row, "delete", cx) else { panic!("delete asks first") };
        assert!(c.destructive && c.token == "delete/1" && c.title == "Delete task \"Inbox\"?");
        assert_eq!(c.rows[0].title, "Inbox #admin");
        calls();
        assert_eq!(status(t.confirmed(&c.token, cx)).as_deref(), Some("Deleted 1: Inbox #admin"));
        assert_eq!(calls(), [changed(None)]);
        assert_eq!((t.running, cx.store.task_get(1), rows(cx).len()), (None, None, 0));
        assert_eq!(status(t.confirmed(&c.token, cx)).as_deref(), Some("task: no task 1"));
        assert_eq!(status(t.confirmed("nope", cx)), None);
    });
}

#[test]
fn cli_reopens_renames_and_removes() {
    with_cx(|cx| {
        let mut t = tasks();
        run(&mut t, cx, &["add", "Mail", "--project", "home"]).unwrap();
        run(&mut t, cx, &["done", "1"]).unwrap();
        assert_eq!(run(&mut t, cx, &["reopen", "Mail"]).unwrap(), "Reopened 1: Mail #home");
        assert_eq!(run(&mut t, cx, &["rename", "1", "Inbox"]).unwrap(), "Renamed 1: Inbox #home");
        let v = json(&mut t, cx, &["rename", "Inbox", "Inbox", "--project", ""]);
        assert!(v["task"]["project"].is_null() && v["message"] == "Renamed");
        assert!(run(&mut t, cx, &["reopen", "nope"]).unwrap_err().contains("no task matches"));
        let v = json(&mut t, cx, &["rm", "1"]);
        assert!(v["task"].is_null() && v["message"] == "Deleted 1: Inbox");
        assert!(run(&mut t, cx, &["rm", "1"]).is_err());
    });
}

#[test]
fn the_hotkey_opens_the_pick_view() {
    let section = |text: &str| crate::config::parse(text).unwrap().section("task").unwrap().unwrap();
    let mut t = tasks();
    assert!(t.hotkeys().is_empty());
    t.configure(&section("[task]\nhotkey = \"cmd+shift+T\"")).unwrap();
    let bindings = t.hotkeys();
    assert_eq!((bindings[0].spec.as_str(), bindings[0].key.as_deref()), ("cmd+shift+T", Ok("pick")));
    test_cx("", |cx| {
        assert!(t.hotkey("pick", cx).is_some_and(|v| v.is("task", "pick")));
        assert!(t.hotkey("other", cx).is_none());
    });
    t.configure(&section("[task]\nhotkey = \" \"")).unwrap();
    assert!(t.hotkeys().is_empty());
    assert!(t.configure(&section("[task]\nhotky = \"x\"")).is_err());
}
