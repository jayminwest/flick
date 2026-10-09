//! Module tests over an in-memory store, synthetic events and a fake clock and workspace.

use std::cell::{Cell, RefCell};

use super::*;
use crate::config::parse;
use crate::core::test_cx;

thread_local! {
    static NOW: Cell<i64> = const { Cell::new(1_000_000) };
    static FRONT: Cell<Option<i32>> = const { Cell::new(Some(1)) };
    static TITLE: RefCell<Option<String>> = const { RefCell::new(None) };
    static TRUSTED: Cell<bool> = const { Cell::new(true) };
    /// System calls made through `Env`, in order.
    static SYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// URL reads asked for and not answered yet (`tests::urls`).
    static ASKS: RefCell<Vec<super::urls::Ask>> = const { RefCell::new(Vec::new()) };
}

fn sys(call: String) {
    SYS.with(|s| s.borrow_mut().push(call));
}

/// The system calls made since the last `calls()`.
fn calls() -> Vec<String> {
    SYS.with(RefCell::take)
}

fn title(t: Option<&str>) {
    TITLE.with(|c| *c.borrow_mut() = t.map(String::from));
}

const OWN: i32 = 99;

fn at(t: i64) {
    NOW.with(|n| n.set(t));
}

fn identity(pid: i32) -> Option<(Option<String>, String)> {
    let (id, name) = match pid {
        1 => (Some("com.apple.Safari"), "Safari"),
        2 => (Some("com.apple.mail"), "Mail"),
        3 => (Some("com.1password.1password"), "1Password"),
        5 => (None, "NoBundle"),
        6 => (Some("com.brave.Browser"), "Brave Browser"),
        7 => (Some("com.google.Chrome"), "Google Chrome"),
        _ => return None,
    };
    Some((id.map(String::from), name.into()))
}

fn activity(table: &str) -> Activity {
    let mut a = Activity {
        env: Env {
            now: || NOW.with(Cell::get),
            utc_offset: |_| 0,
            identity,
            frontmost: || FRONT.with(Cell::get),
            own_pid: OWN,
            title: |pid| {
                sys(format!("title {pid}"));
                TITLE.with(|t| t.borrow().clone())
            },
            follow: |pid| sys(format!("follow {pid}")),
            unfollow: || sys("unfollow".into()),
            trusted: || TRUSTED.with(Cell::get),
            ask_url: |ask| {
                sys(format!("ask_url {} {}", ask.pid, ask.bundle));
                ASKS.with(|a| a.borrow_mut().push(ask));
            },
            indicator: |on| sys(format!("indicator {on}")),
            on_quit: |_| sys("on_quit".into()),
        },
        ..Activity::default()
    };
    a.configure(&parse(table).unwrap().section("activity").unwrap().unwrap()).unwrap();
    a
}

/// Run `f` with a migrated store; the fake clock starts at `T` with Safari in front.
fn with_cx(f: impl FnOnce(&mut Cx)) {
    at(1_000_000);
    FRONT.with(|f| f.set(Some(1)));
    title(None);
    TRUSTED.with(|t| t.set(true));
    calls();
    ASKS.with(RefCell::take);
    test_cx("", |cx| {
        cx.store.migrate("activity", MIGRATIONS).unwrap();
        f(cx);
    });
}

fn run(a: &mut Activity, cx: &mut Cx, words: &str) -> Result<String, String> {
    let args: Vec<String> = words.split(' ').map(String::from).collect();
    a.command(&args, cx)
}

/// Stored spans as (start, end, app name).
fn rows(cx: &Cx) -> Vec<(i64, i64, String)> {
    cx.store.spans(0, i64::MAX).into_iter().map(|s| (s.start, s.end, s.subject.name)).collect()
}

const T: i64 = 1_000_000;

mod remote;
mod tasks;
mod urls;

#[test]
fn nothing_is_recorded_until_recording_is_on() {
    with_cx(|cx| {
        let mut a = activity("");
        assert!(!a.on_event(Event::Started, cx));
        assert!(!a.on_event(Event::AppActivated { pid: 2 }, cx));
        assert!(rows(cx).is_empty());
        assert_eq!(run(&mut a, cx, "status").unwrap(), "recording: off\ntitles: off\nurls: off\nopen span: none");
    });
}

#[test]
fn spans_follow_focus_idle_and_events() {
    with_cx(|cx| {
        let mut a = activity("");
        assert_eq!(run(&mut a, cx, "on").unwrap(), "Activity recording on");
        assert_eq!(run(&mut a, cx, "on").unwrap(), "Activity recording on");
        at(T + 60);
        assert!(a.on_event(Event::AppActivated { pid: 2 }, cx));
        at(T + 70);
        a.on_event(Event::PasteboardChanged, cx); // any event advances the open span
        assert_eq!(rows(cx), [(T, T + 60, "Safari".into()), (T + 60, T + 70, "Mail".into())]);
        assert!(run(&mut a, cx, "status").unwrap().ends_with("open span: Mail since 13:47"));
        // Flick itself and an app that quit leave the span alone.
        at(T + 80);
        a.on_event(Event::AppActivated { pid: OWN }, cx);
        a.on_event(Event::AppActivated { pid: 4 }, cx);
        assert_eq!(rows(cx)[1], (T + 60, T + 80, "Mail".into()));
        // Idle backdates the close; Active reopens on the same app.
        at(T + 200);
        a.on_event(Event::Idle { secs: 60 }, cx);
        assert_eq!(rows(cx)[1].1, T + 140);
        assert!(run(&mut a, cx, "status").unwrap().ends_with("open span: idle"));
        at(T + 300);
        a.on_event(Event::Active, cx);
        at(T + 330);
        assert_eq!(run(&mut a, cx, "off").unwrap(), "Activity recording off");
        assert_eq!(rows(cx)[2], (T + 300, T + 330, "Mail".into()));
        assert!(cx.store.open_span().is_none());
        at(T + 400);
        a.on_event(Event::AppActivated { pid: 1 }, cx);
        assert_eq!(rows(cx).len(), 3);
    });
}

#[test]
fn excluded_apps_leave_a_gap() {
    with_cx(|cx| {
        let mut a = activity("[activity]\nexclude = [\"com.1password.1password\", \"nobundle\"]");
        run(&mut a, cx, "on").unwrap();
        at(T + 30);
        a.on_event(Event::AppActivated { pid: 3 }, cx);
        assert!(run(&mut a, cx, "status").unwrap().ends_with("open span: excluded app in front"));
        at(T + 50);
        a.on_event(Event::AppActivated { pid: 5 }, cx); // excluded by name
        at(T + 90);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        at(T + 100);
        a.on_event(Event::LauncherOpened, cx);
        assert_eq!(rows(cx), [(T, T + 30, "Safari".into()), (T + 90, T + 100, "Mail".into())]);
        // Recording on with an excluded app in front opens nothing.
        run(&mut a, cx, "off").unwrap();
        FRONT.with(|f| f.set(Some(3)));
        run(&mut a, cx, "on").unwrap();
        assert_eq!(rows(cx).len(), 2);
    });
}

#[test]
fn flicker_and_bundleless_apps() {
    with_cx(|cx| {
        let mut a = activity("[activity]\nexclude = []");
        run(&mut a, cx, "on").unwrap();
        at(T + 100);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        at(T + 101);
        a.on_event(Event::AppActivated { pid: 5 }, cx); // a 1 s flicker on Mail
        at(T + 110);
        a.on_event(Event::Wake, cx); // closes at the last event, then reopens
        let spans = cx.store.spans(0, i64::MAX);
        let last = &spans[1];
        assert_eq!((last.start, last.end, last.subject.app.as_str()), (T + 100, T + 101, "NoBundle"));
        assert_eq!(rows(cx).len(), 3);
        assert_eq!(rows(cx)[2], (T + 110, T + 110, "Safari".into()));
    });
}

#[test]
fn a_restart_closes_the_leftover_span() {
    with_cx(|cx| {
        let mut a = activity("");
        run(&mut a, cx, "on").unwrap();
        at(T + 50);
        a.on_event(Event::DisplaysChanged, cx);
        // A new process: the recording flag persists, the open row ends at its last event.
        let mut b = activity("");
        at(T + 500);
        FRONT.with(|f| f.set(Some(2)));
        assert!(b.on_event(Event::Started, cx));
        assert_eq!(rows(cx), [(T, T + 50, "Safari".into()), (T + 500, T + 500, "Mail".into())]);
        FRONT.with(|f| f.set(None));
        b.on_event(Event::Wake, cx);
        assert!(cx.store.open_span().is_none());
    });
}

#[test]
fn reports_are_text_or_json() {
    with_cx(|cx| {
        let mut a =
            activity("[[activity.rules]]\napp = \"safari\"\nproject = \"web\"\ncategory = \"browse\"");
        // T is 13:46 UTC on day 11; the report starts at midnight.
        run(&mut a, cx, "on").unwrap();
        at(T + 600);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        at(T + 900);
        let today = run(&mut a, cx, "today").unwrap();
        assert!(today.starts_with("Activity today (recording on)\nRecorded 15m, gaps 0s\nCategories"), "{today}");
        assert!(today.contains("\nProjects\n       10m   67%  web"), "{today}");
        assert!(run(&mut a, cx, "week").unwrap().starts_with("Activity week"));
        cx.json = true;
        let json: serde_json::Value = serde_json::from_str(&run(&mut a, cx, "today").unwrap()).unwrap();
        assert_eq!(json["by_app"][1]["name"], "Mail");
        assert_eq!(json["recorded_secs"], 900);
        let spans: serde_json::Value = serde_json::from_str(&run(&mut a, cx, "spans").unwrap()).unwrap();
        assert_eq!(spans[0]["category"], "browse");
        assert_eq!(spans[1]["end"], T + 900);
        cx.json = false;
        let text = run(&mut a, cx, "spans --since 1970-01-12").unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(run(&mut a, cx, "spans --since 1970-01-13").unwrap().is_empty());
        assert_eq!(
            run(&mut a, cx, "spans --since soon").unwrap_err(),
            "activity: bad date \"soon\" (today, week or YYYY-MM-DD)"
        );
        assert!(run(&mut a, cx, "spans today").unwrap_err().starts_with("activity: usage:"));
        assert_eq!(run(&mut a, cx, "dance").unwrap_err(), "activity: unknown command \"dance\"");
    });
}

#[test]
fn forget_needs_yes() {
    with_cx(|cx| {
        let mut a = activity("");
        run(&mut a, cx, "on").unwrap();
        at(T + 60);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        at(T + 70);
        let need = "activity: forget deletes spans; add --yes";
        assert_eq!(run(&mut a, cx, "forget today").unwrap_err(), need);
        assert_eq!(run(&mut a, cx, "forget app com.apple.mail").unwrap_err(), need);
        assert!(run(&mut a, cx, "forget yesterday --yes").unwrap_err().starts_with("activity: usage:"));
        assert_eq!(run(&mut a, cx, "forget app com.apple.mail --yes").unwrap(), "Deleted 1 spans");
        // Recording goes on from the frontmost app.
        assert_eq!(rows(cx), [(T, T + 60, "Safari".into()), (T + 70, T + 70, "Safari".into())]);
        assert_eq!(run(&mut a, cx, "forget today --yes").unwrap(), "Deleted 2 spans");
        assert_eq!(rows(cx).len(), 1);
        assert_eq!(run(&mut a, cx, "forget all --yes").unwrap(), "Deleted 1 spans");
        assert!(rows(cx).is_empty() && !cx.store.recording());
    });
}

#[test]
fn root_items_view_and_hotkey() {
    with_cx(|cx| {
        let mut a = activity("");
        let titles = |a: &mut Activity, cx: &mut Cx| a.items(cx).into_iter().map(|i| i.title).collect::<Vec<_>>();
        assert_eq!(
            titles(&mut a, cx),
            ["Start Activity Recording", "Activity Today", "Allow Agents to Read Activity (1 h)"]
        );
        let mut view = a.open("today", cx).unwrap();
        assert!(a.open("week", cx).is_none());
        a.refresh(&mut view, cx);
        assert_eq!((view.items.len(), view.empty.as_str()), (0, "Activity recording is off"));

        let record = ItemId::new("activity", "record");
        let on = a.activate(&record, cx);
        assert!(matches!(on, Outcome::Stay(Some(s)) if s == "Activity recording on"));
        assert_eq!(titles(&mut a, cx)[0], "Stop Activity Recording");
        at(T + 120);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        at(T + 160);
        a.refresh(&mut view, cx);
        let shown = |v: &ListView| v.items.iter().map(|i| (i.title.clone(), i.accessory.clone())).collect::<Vec<_>>();
        assert_eq!(shown(&view), [("Safari".into(), "2m  ·  75%".into()), ("Mail".into(), "40s  ·  25%".into())]);
        assert_eq!(view.items[0].subtitle, "App");
        assert!(matches!(a.activate(&ItemId::new("activity", "today"), cx), Outcome::Push(v) if v.is("activity", "today")));
        assert!(matches!(a.activate(&view.items[0].id, cx), Outcome::Stay(None)));
    });
}

#[test]
fn the_hotkey_toggles_recording() {
    with_cx(|cx| {
        let mut a = activity("[activity]\nhotkey = \"cmd+F9\"");
        assert_eq!(a.hotkeys(), [Binding { spec: "cmd+F9".into(), key: Ok("toggle".into()) }]);
        assert!(activity("").hotkeys().is_empty());
        assert!(a.hotkey("toggle", cx).is_none());
        assert!(cx.store.recording());
        a.hotkey("other", cx);
        assert!(cx.store.recording());
        a.hotkey("toggle", cx);
        assert!(!cx.store.recording());
    });
}

#[test]
fn rules_show_categories_and_projects_in_the_view() {
    with_cx(|cx| {
        let mut a = activity("[[activity.rules]]\napp = \"mail\"\nproject = \"inbox\"\ncategory = \"comms\"");
        run(&mut a, cx, "on").unwrap();
        at(T + 30);
        let mut view = a.open("today", cx).unwrap();
        a.refresh(&mut view, cx);
        assert_eq!(view.empty, "Nothing recorded today yet");
        let kinds = view.items.iter().map(|i| (i.subtitle.as_str(), i.title.as_str())).collect::<Vec<_>>();
        assert_eq!(kinds, [("App", "Safari")]);
        at(T + 40);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        at(T + 70);
        a.refresh(&mut view, cx);
        let kinds = view.items.iter().map(|i| (i.subtitle.as_str(), i.title.as_str())).collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [("Category", "Uncategorized"), ("Category", "comms"), ("Project", "inbox"), ("App", "Safari"), ("App", "Mail")]
        );
    });
}

/// Stored spans as (start, end, title).
fn titled(cx: &Cx) -> Vec<(i64, i64, Option<String>)> {
    cx.store.spans(0, i64::MAX).into_iter().map(|s| (s.start, s.end, s.subject.title)).collect()
}

#[test]
fn titles_off_reads_no_title_and_follows_nothing() {
    with_cx(|cx| {
        let mut a = activity("");
        title(Some("secret"));
        run(&mut a, cx, "on").unwrap();
        at(T + 30);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        a.on_event(Event::WindowChanged { pid: 2 }, cx);
        assert_eq!(calls(), ["indicator true", "on_quit"]);
        assert!(titled(cx).iter().all(|(_, _, t)| t.is_none()));
        run(&mut a, cx, "off").unwrap();
        assert_eq!(calls(), ["indicator false"]);
    });
}

#[test]
fn titles_on_follow_the_front_app_while_recording() {
    with_cx(|cx| {
        let mut a = activity("[activity]\ntitles = true");
        a.on_event(Event::Started, cx);
        assert!(calls().is_empty(), "nothing is followed while recording is off");
        title(Some("⠋ inbox"));
        run(&mut a, cx, "on").unwrap();
        assert_eq!(calls(), ["title 1", "indicator true", "on_quit", "follow 1"]);
        // A title change on the followed app is a new subject; one on another pid is not.
        at(T + 30);
        title(Some("⠙ news"));
        a.on_event(Event::WindowChanged { pid: 1 }, cx);
        a.on_event(Event::WindowChanged { pid: 2 }, cx);
        assert_eq!(calls(), ["title 1"]);
        at(T + 40);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        // Flick itself keeps the follow; an excluded app drops it.
        a.on_event(Event::AppActivated { pid: OWN }, cx);
        assert_eq!(calls(), ["title 2", "follow 2"]);
        at(T + 50);
        a.on_event(Event::AppActivated { pid: 3 }, cx);
        assert_eq!(calls(), ["unfollow"]);
        assert_eq!(
            titled(cx),
            [
                (T, T + 30, Some("inbox".into())),
                (T + 30, T + 40, Some("news".into())),
                (T + 40, T + 50, Some("news".into())),
            ]
        );
        at(T + 60);
        a.on_event(Event::AppActivated { pid: 1 }, cx);
        run(&mut a, cx, "off").unwrap();
        assert_eq!(calls(), ["title 1", "follow 1", "indicator false", "unfollow"]);
        TRUSTED.with(|t| t.set(false));
        assert!(run(&mut a, cx, "status").unwrap().contains("titles: no Accessibility permission"));
    });
}

#[test]
fn lock_and_sleep_pause_until_unlock_and_wake() {
    with_cx(|cx| {
        let mut a = activity("[activity]\ntitles = true");
        run(&mut a, cx, "on").unwrap();
        at(T + 10);
        a.on_event(Event::Locked, cx);
        a.on_event(Event::Locked, cx);
        assert!(run(&mut a, cx, "status").unwrap().ends_with("open span: screen locked or asleep"));
        at(T + 20);
        a.on_event(Event::Sleep, cx);
        at(T + 100);
        a.on_event(Event::Wake, cx); // still locked
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        assert_eq!(rows(cx), [(T, T + 10, "Safari".into())]);
        at(T + 120);
        a.on_event(Event::Unlocked, cx);
        a.on_event(Event::Unlocked, cx); // a second notice only advances the span
        at(T + 130);
        a.on_event(Event::Sleep, cx);
        at(T + 200);
        a.on_event(Event::Wake, cx);
        assert_eq!(rows(cx)[1..], [(T + 120, T + 130, "Safari".into()), (T + 200, T + 200, "Safari".into())]);
        let follows: Vec<String> = calls().into_iter().filter(|c| !c.starts_with("title")).collect();
        assert_eq!(follows, ["indicator true", "on_quit", "follow 1", "unfollow", "follow 1", "unfollow", "follow 1"]);
    });
}

#[test]
fn the_indicator_menu_stops_recording() {
    with_cx(|cx| {
        let mut a = activity("");
        run(&mut a, cx, "on").unwrap();
        at(T + 30);
        // Another module's progress, or no pending stop, leaves recording on.
        a.on_event(Event::ModuleChanged { module: "flick" }, cx);
        a.on_event(Event::ModuleChanged { module: "activity" }, cx);
        assert!(cx.store.recording());
        wire::STOP.set(true);
        assert!(a.on_event(Event::ModuleChanged { module: "activity" }, cx));
        assert!(!cx.store.recording() && !wire::take_stop());
        assert_eq!(rows(cx), [(T, T + 30, "Safari".into())]);
        assert_eq!(calls(), ["indicator true", "on_quit", "indicator false"]);
        // A restart with recording on shows the indicator again; forget all hides it.
        run(&mut a, cx, "on").unwrap();
        let mut b = activity("");
        b.on_event(Event::Started, cx);
        calls();
        run(&mut b, cx, "forget all --yes").unwrap();
        assert_eq!(calls(), ["indicator false"]);
    });
}

