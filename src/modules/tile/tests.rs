//! Module tests over an in-memory store and fake hooks: windows live in a thread-local list
//! that `place` and `hide` move, so no test touches a real window.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use super::*;
use crate::core::test_cx;
use crate::platform::winman::{AppInfo, Button, Focus};

/// Where the fake `hide` parks windows; the fake `stray` rescues any window there.
const PARK_X: f64 = 5000.0;
const RESCUED: Rect = Rect { x: 100.0, y: 100.0, w: 800.0, h: 600.0 };

thread_local! {
    static WINDOWS: RefCell<Vec<WinInfo>> = const { RefCell::new(Vec::new()) };
    /// Hook calls, in order.
    static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TRUSTED: Cell<bool> = const { Cell::new(true) };
    /// Windows the window server no longer has.
    static GONE: RefCell<HashSet<WinId>> = RefCell::new(HashSet::new());
    /// `place` returns a batch that never finishes.
    static HUNG: Cell<bool> = const { Cell::new(false) };
}

fn call(line: String) {
    CALLS.with(|c| c.borrow_mut().push(line));
}

/// The hook calls since the last `calls()`.
fn calls() -> Vec<String> {
    CALLS.with(RefCell::take)
}

fn moved(frames: &[(WinId, Rect)]) {
    WINDOWS.with(|ws| {
        for w in ws.borrow_mut().iter_mut() {
            if let Some(&(_, f)) = frames.iter().find(|(id, _)| *id == w.id) {
                w.frame = f;
            }
        }
    });
}

fn ids(frames: &[(WinId, Rect)]) -> String {
    frames.iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>().join(",")
}

const FAKE: Hooks = Hooks {
    start: || {
        call("start".into());
        TRUSTED.get()
    },
    stop: || call("stop".into()),
    drain: || call("drain".into()),
    snapshot: || WINDOWS.with(|w| w.borrow().clone()),
    place: |frames| {
        call(format!("place {}", ids(frames)));
        moved(frames);
        if HUNG.get() { Batch::pending() } else { Batch::done() }
    },
    hide: |windows| {
        call(format!("hide {}", ids(windows)));
        let parked: Vec<_> = windows.iter().map(|&(id, f)| (id, Rect { x: PARK_X, ..f })).collect();
        moved(&parked);
        Batch::done()
    },
    exists: |id| !GONE.with(|g| g.borrow().contains(&id)),
    stray: |f| (f.x == PARK_X).then_some(RESCUED),
    on_quit: |_| call("on_quit".into()),
    put_back: |db| call(format!("put_back {}", db.rsplit('/').next().unwrap_or(db))),
    log: |line| call(format!("log {line}")),
};

fn rect(x: f64) -> Rect {
    Rect { x, y: 25.0, w: 800.0, h: 600.0 }
}

fn win(id: WinId, x: f64) -> WinInfo {
    WinInfo {
        id,
        app: AppInfo { pid: 40 + id as i32, bundle_id: Some(format!("com.example.w{id}")), hidden: false },
        title: format!("Window {id}"),
        frame: rect(x),
        subrole: Some("AXStandardWindow".into()),
        minimized: false,
        fullscreen: false,
        fullscreen_button: Button::Enabled,
        title_buttons: true,
        focus: Focus::default(),
        level: Some(0),
    }
}

fn set_windows(windows: Vec<WinInfo>) {
    WINDOWS.with(|w| *w.borrow_mut() = windows);
}

fn x_of(id: WinId) -> Option<f64> {
    WINDOWS.with(|w| w.borrow().iter().find(|w| w.id == id).map(|w| w.frame.x))
}

/// A fresh module and fake world with `windows`; `f` runs with a migrated store.
fn with_tile(windows: Vec<WinInfo>, f: impl FnOnce(&mut Tile, &mut Cx)) {
    set_windows(windows);
    TRUSTED.set(true);
    HUNG.set(false);
    GONE.with(|g| g.borrow_mut().clear());
    calls();
    test_cx("", |cx| {
        cx.store.migrate(ID, MIGRATIONS).unwrap();
        let mut t = Tile::new(FAKE);
        f(&mut t, cx);
    });
}

fn run(t: &mut Tile, cx: &mut Cx, words: &[&str]) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    t.command(&args, cx)
}

fn changed(t: &mut Tile, cx: &mut Cx) {
    assert!(!t.on_event(Event::ModuleChanged { module: ID }, cx));
}

fn stored(cx: &Cx) -> Vec<WinId> {
    cx.store.parked_all().iter().map(|r| r.id).collect()
}

fn parked(id: WinId, x: f64) -> Parked {
    Parked { id, pid: 1, bundle_id: None, frame: rect(x) }
}

#[test]
fn switching_parks_the_old_workspace_and_round_trips_every_window() {
    with_tile(vec![win(1, 0.0), win(2, 300.0)], |t, cx| {
        t.on_event(Event::Started, cx);
        assert_eq!(calls(), ["on_quit", "start"]);
        assert_eq!(run(t, cx, &["ws", "2"]).unwrap(), "workspace 2: 0 shown, 2 hidden");
        assert_eq!(stored(cx), [1, 2]);
        assert_eq!((x_of(1), x_of(2)), (Some(PARK_X), Some(PARK_X)));
        let log = calls();
        assert_eq!(log[..2], ["place ", "hide 1,2"]);
        assert!(log[2].starts_with("log ws 2: 2 windows in "), "{log:?}");
        // A new window joins the focused workspace, 2.
        WINDOWS.with(|w| w.borrow_mut().push(win(3, 600.0)));
        changed(t, cx);
        assert_eq!(calls(), ["drain"]);
        assert_eq!(run(t, cx, &["ws", "1"]).unwrap(), "workspace 1: 2 shown, 1 hidden");
        assert_eq!((x_of(1), x_of(2), x_of(3)), (Some(0.0), Some(300.0), Some(PARK_X)));
        assert_eq!(stored(cx), [3]);
        assert_eq!(calls()[..2], ["place 1,2", "hide 3"]);
        assert_eq!(run(t, cx, &["ws", "1"]).unwrap(), "workspace 1");
        assert_eq!(run(t, cx, &["ws", "2"]).unwrap(), "workspace 2: 1 shown, 2 hidden");
        assert_eq!(x_of(3), Some(600.0));
    });
}

#[test]
fn minimized_fullscreen_and_parked_windows_are_not_parked_again() {
    let minimized = WinInfo { minimized: true, ..win(2, 0.0) };
    let fullscreen = WinInfo { fullscreen: true, ..win(3, 0.0) };
    with_tile(vec![win(1, 0.0), minimized, fullscreen], |t, cx| {
        t.on_event(Event::Started, cx);
        run(t, cx, &["ws", "3"]).unwrap();
        assert_eq!(stored(cx), [1]);
        WINDOWS.with(|w| w.borrow_mut().push(win(4, 0.0)));
        changed(t, cx);
        assert_eq!(run(t, cx, &["ws", "1"]).unwrap(), "workspace 1: 1 shown, 1 hidden");
        assert_eq!(stored(cx), [4]);
        // A smaller config moves workspace 3's windows onto the focused workspace 1, where
        // the next sync puts them back.
        let table = |text: &str| crate::config::parse(text).unwrap().section(ID).unwrap().unwrap();
        t.configure(&table("[tile]\nworkspaces = 2")).unwrap();
        calls();
        changed(t, cx);
        assert_eq!(calls(), ["drain", "place 4"]);
        assert!(stored(cx).is_empty());
        assert_eq!(run(t, cx, &["ws", "3"]).unwrap_err(), "tile: ws: expected a workspace 1-2");
        assert_eq!(run(t, cx, &["ws", "2"]).unwrap(), "workspace 2: 0 shown, 2 hidden");
    });
}

#[test]
fn closed_windows_leave_their_workspace_and_their_row() {
    with_tile(vec![win(1, 0.0), win(2, 0.0)], |t, cx| {
        t.on_event(Event::Started, cx);
        run(t, cx, &["ws", "2"]).unwrap();
        set_windows(vec![win(2, PARK_X)]);
        changed(t, cx);
        assert_eq!(stored(cx), [2]);
        assert_eq!(t.spaces.of(1), None);
        // A closed window that comes back with the same id is new: parked, it is rescued.
        set_windows(vec![win(1, PARK_X), win(2, PARK_X)]);
        calls();
        changed(t, cx);
        assert_eq!(calls(), ["drain", "place 1"]);
        assert_eq!(x_of(1), Some(RESCUED.x));
    });
}

#[test]
fn a_start_after_a_crash_puts_back_what_the_last_run_parked() {
    with_tile(vec![win(1, PARK_X), win(5, PARK_X), win(6, 10.0)], |t, cx| {
        // Rows a killed Flick left: 1 is on screen, 2 closed meanwhile, 4's app has not been
        // scanned yet, 7 closes before it is.
        let rows = [parked(1, 11.0), parked(2, 22.0), parked(4, 44.0), parked(7, 77.0)];
        cx.store.parked_put(&rows).unwrap();
        GONE.with(|g| g.borrow_mut().insert(2));
        t.on_event(Event::Started, cx);
        let log = calls();
        assert_eq!(log[..3], ["on_quit", "log putting back 3 windows a previous run left parked", "start"]);
        // 1 goes back to its saved frame, 5 sat parked without a row: centred.
        assert_eq!(log[3..], ["place 1,5"]);
        assert_eq!((x_of(1), x_of(5), x_of(6)), (Some(11.0), Some(RESCUED.x), Some(10.0)));
        assert_eq!(stored(cx), [4, 7]);
        GONE.with(|g| g.borrow_mut().insert(7));
        WINDOWS.with(|w| w.borrow_mut().push(win(4, PARK_X)));
        changed(t, cx);
        assert_eq!(calls(), ["drain", "place 4"]);
        assert_eq!(x_of(4), Some(44.0));
        assert!(stored(cx).is_empty());
        // Every window joined workspace 1.
        assert_eq!(t.spaces.windows(0), [1, 5, 6, 4]);
        // A second Started does nothing.
        t.on_event(Event::Started, cx);
        assert!(calls().is_empty());
    });
}

#[test]
fn without_accessibility_nothing_runs() {
    with_tile(vec![win(1, PARK_X)], |t, cx| {
        TRUSTED.set(false);
        cx.store.parked_put(&[parked(1, 0.0)]).unwrap();
        t.on_event(Event::Started, cx);
        let log = calls();
        assert_eq!(log[2..], ["start", "log needs Accessibility permission; windows stay where they are"]);
        changed(t, cx);
        assert!(calls().is_empty());
        for verb in [&["ls"][..], &["ws", "2"], &["restore"]] {
            assert_eq!(run(t, cx, verb).unwrap_err(), NOT_RUNNING);
        }
        // The rows stay for a start with permission.
        assert_eq!(stored(cx), [1]);
        assert_eq!(run(t, cx, &["ws", "x"]).unwrap_err(), "tile: ws: expected a workspace 1-8");
        assert_eq!(run(t, cx, &["ws"]).unwrap_err(), "tile: unknown command \"ws\"");
        assert_eq!(run(t, cx, &[]).unwrap_err(), "tile: missing command");
    });
}

#[test]
fn restore_puts_every_window_back_on_the_focused_workspace() {
    with_tile(vec![win(1, 0.0), win(2, 300.0)], |t, cx| {
        t.on_event(Event::Started, cx);
        run(t, cx, &["ws", "3"]).unwrap();
        calls();
        assert_eq!(run(t, cx, &["restore"]).unwrap(), "put back 2 windows");
        assert_eq!(calls(), ["place 1,2"]);
        assert_eq!((x_of(1), x_of(2)), (Some(0.0), Some(300.0)));
        assert!(stored(cx).is_empty());
        assert_eq!(t.spaces.windows(2), [1, 2]);
    });
}

#[test]
fn a_failed_row_write_parks_nothing() {
    set_windows(vec![win(1, 0.0)]);
    TRUSTED.set(true);
    test_cx("", |cx| {
        // No migration: the write fails.
        let mut t = Tile::new(FAKE);
        t.on_event(Event::Started, cx);
        calls();
        let err = run(&mut t, cx, &["ws", "2"]).unwrap_err();
        assert!(err.starts_with("tile: no such table"), "{err}");
        assert_eq!(t.spaces.focused(), 0);
        assert!(calls().is_empty());
    });
}

#[test]
fn a_switch_is_logged_only_once_every_app_answered() {
    with_tile(vec![win(1, 0.0)], |t, cx| {
        t.on_event(Event::Started, cx);
        run(t, cx, &["ws", "2"]).unwrap();
        HUNG.set(true);
        calls();
        run(t, cx, &["ws", "1"]).unwrap();
        changed(t, cx);
        assert_eq!(calls(), ["place 1", "hide ", "drain"]);
    });
}

#[test]
fn ls_lists_windows_by_workspace() {
    let dialog = WinInfo { subrole: Some("AXDialog".into()), title: String::new(), ..win(9, 5.0) };
    let anonymous = WinInfo { app: AppInfo { pid: 77, bundle_id: None, hidden: false }, ..win(3, 0.0) };
    with_tile(vec![win(1, 0.0), dialog, win(2, 300.0)], |t, cx| {
        assert_eq!(run(t, cx, &["ls"]).unwrap_err(), NOT_RUNNING);
        t.on_event(Event::Started, cx);
        run(t, cx, &["ws", "2"]).unwrap();
        WINDOWS.with(|w| w.borrow_mut().push(anonymous));
        changed(t, cx);
        assert_eq!(
            run(t, cx, &["ls"]).unwrap(),
            "1       1  com.example.w1  5000,25 800x600 parked  Window 1\n\
             1       2  com.example.w2  5000,25 800x600 parked  Window 2\n\
             2*      3  pid 77  0,25 800x600  Window 3\n\
             -       9  com.example.w9  5,25 800x600  "
        );
        cx.json = true;
        let rows: serde_json::Value = serde_json::from_str(&run(t, cx, &["ls"]).unwrap()).unwrap();
        assert_eq!(
            rows[0],
            json!({
                "workspace": 1, "id": 1, "pid": 41, "app": "com.example.w1", "title": "Window 1",
                "frame": { "x": 5000.0, "y": 25.0, "w": 800.0, "h": 600.0 }, "parked": true,
            })
        );
        assert_eq!((rows[2]["app"].clone(), rows[3]["workspace"].clone()), (json!(null), json!(null)));
        set_windows(vec![]);
        cx.json = false;
        assert_eq!(run(t, cx, &["ls"]).unwrap(), "no windows");
    });
}

#[test]
fn put_back_waits_for_the_apps_before_forgetting_rows() {
    with_tile(vec![win(1, PARK_X)], |_, cx| {
        cx.store.parked_put(&[parked(1, 11.0), parked(9, 99.0)]).unwrap();
        HUNG.set(true);
        put_back(cx.store, &FAKE, Duration::ZERO);
        assert_eq!(calls(), ["place 1", "log an app did not answer; its windows go back at the next start"]);
        assert_eq!(stored(cx), [1, 9]);
        HUNG.set(false);
        put_back(cx.store, &FAKE, Duration::ZERO);
        // 9 is a window winman does not know: its row waits for the next start.
        assert_eq!(stored(cx), [9]);
        assert_eq!(x_of(1), Some(11.0));
        calls();
        set_windows(vec![]);
        put_back(cx.store, &FAKE, Duration::ZERO);
        assert!(calls().is_empty());
    });
}

#[test]
fn dropping_a_started_module_puts_windows_back_and_stops() {
    with_tile(vec![], |t, cx| {
        drop(std::mem::replace(t, Tile::new(FAKE)));
        assert!(calls().is_empty());
        t.on_event(Event::Started, cx);
        calls();
        drop(std::mem::replace(t, Tile::new(FAKE)));
        // An in-memory store has no file to reopen.
        assert_eq!(calls(), ["stop"]);
    });
    let path = std::env::temp_dir().join(format!("flick-tile-{}.db", std::process::id()));
    let store = Store::open(&path).unwrap();
    let mut ranker = crate::core::Ranker::new();
    let mut cx = Cx { query: "", store: &store, ranker: &mut ranker, hide: || {}, json: false, remote: false };
    let mut t = Tile::new(FAKE);
    t.on_event(Event::Started, &mut cx);
    calls();
    drop(t);
    assert_eq!(calls(), [format!("put_back flick-tile-{}.db", std::process::id()), "stop".into()]);
    drop(store);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn settings_take_one_to_nine_workspaces() {
    let table = |text: &str| crate::config::parse(text).unwrap().section(ID).unwrap().unwrap();
    let mut t = Tile::new(FAKE);
    assert!(t.configure(&table("")).is_ok());
    assert_eq!(t.spaces.count(), 8);
    assert!(t.configure(&table("[tile]\nworkspaces = 3")).is_ok());
    assert_eq!(t.spaces.count(), 3);
    for bad in ["[tile]\nworkspaces = 0", "[tile]\nworkspaces = 10"] {
        assert_eq!(t.configure(&table(bad)).unwrap_err(), "[tile] workspaces: must be 1-9");
    }
    assert!(t.configure(&table("[tile]\nwork = 1")).unwrap_err().starts_with("[tile]: unknown field"));
    assert_eq!(t.id(), ID);
    assert_eq!(t.migrations(), MIGRATIONS);
    assert!(t.verbs().starts_with("tile ls"));
    crate::config::example::assert_documents::<Settings>(ID);
}

#[test]
fn other_events_do_nothing() {
    with_tile(vec![win(1, 0.0)], |t, cx| {
        t.on_event(Event::Started, cx);
        calls();
        assert!(!t.on_event(Event::Wake, cx));
        assert!(!t.on_event(Event::ModuleChanged { module: "kota" }, cx));
        assert!(calls().is_empty());
    });
}
