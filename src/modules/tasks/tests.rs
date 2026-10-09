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
