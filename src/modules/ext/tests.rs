use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::items::LOADING;
use super::proto::{Parsed, Reply};
use super::*;
use crate::core::scratch::Scratch;
use crate::core::test_cx;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ext-echo.sh");

thread_local! {
    /// What the hooks saw on this thread: `log <line>`, `notify <ext> | <error>`,
    /// `toast <ext> | <line>`, `card <do>` and `open <module> <key>`.
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}

fn log(line: &str) {
    SEEN.with(|s| s.borrow_mut().push(format!("log {line}")));
}

fn notify(ext: &str, error: &str) {
    SEEN.with(|s| s.borrow_mut().push(format!("notify {ext} | {error}")));
}

fn toast(ext: &str, line: &str) {
    SEEN.with(|s| s.borrow_mut().push(format!("toast {ext} | {line}")));
}

/// Records the card `do`; `open_app` of "Nope" fails.
fn card(run: &Do) -> Result<(), String> {
    SEEN.with(|s| s.borrow_mut().push(format!("card {}", run.to_value())));
    match run {
        Do::OpenApp(app) if app == "Nope" => Err("open_app: no app \"Nope\"".into()),
        _ => Ok(()),
    }
}

/// Answers a card `script` or `flick` with its words, or fails for `fail`.
fn flick(words: &[String]) -> Result<Parsed, String> {
    match words.first().map(String::as_str) {
        Some("fail") => Err("exit 1".into()),
        _ => Ok(Parsed { reply: Reply::Done(proto::Done { status: Some(words.join(" ")), refresh: false }), warnings: vec![] }),
    }
}

/// Records the hotkey `ext open` runs.
fn open(module: &str, key: &str) {
    SEEN.with(|s| s.borrow_mut().push(format!("open {module} {key}")));
}

pub(super) fn seen() -> Vec<String> {
    SEEN.with(RefCell::take)
}

pub(super) fn socket() -> PathBuf {
    "/tmp/flick-test.sock".into()
}

/// An `ext` over the real transport and the fixture, with recording hooks.
pub(super) fn module(table: &str) -> Ext {
    let job = Job { run: transport::call, socket, post: || {}, flick, raw: transport::raw };
    let mut m = Ext::with(Hooks { job, log, notify, toast, card, visible: || true, open });
    configure(&mut m, table).unwrap();
    m
}

pub(super) fn configure(m: &mut Ext, table: &str) -> Result<(), String> {
    let config = crate::config::parse(table)?;
    m.configure(&config.section("ext")?.unwrap_or_else(|| Section::empty("ext")))
}

/// `[[ext.extensions]]` for the fixture in `mode`.
pub(super) fn entry(name: &str, mode: &[&str], timeout_ms: u64) -> String {
    let argv: Vec<String> = std::iter::once(FIXTURE).chain(mode.iter().copied()).map(|a| format!("{a:?}")).collect();
    format!("[[ext.extensions]]\nname = \"{name}\"\nargv = [{}]\ntimeout_ms = {timeout_ms}\n", argv.join(", "))
}

/// Drain until every queued call was answered (bounded), as the main thread does on
/// `ModuleChanged`; true when a root item or view changed.
pub(super) fn settle(m: &mut Ext) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut stale = false;
    while !m.pending.is_empty() && Instant::now() < deadline {
        stale |= m.drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    stale
}

/// Wait (bounded) until `n` answers are in the inbox, then drain them; true when a root
/// item, view or form changed.
pub(super) fn answered(m: &mut Ext, n: usize) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while m.inbox.len() < n && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(m.inbox.len(), n);
    m.drain()
}

/// Drain until something was logged (bounded).
fn wait_seen(m: &mut Ext) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut got = vec![];
    while got.is_empty() && Instant::now() < deadline {
        assert!(!m.drain());
        got = seen();
        std::thread::sleep(Duration::from_millis(5));
    }
    got
}

fn event(m: &mut Ext, event: Event) -> bool {
    test_cx("", |cx| m.on_event(event, cx))
}

pub(super) fn ids(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.id.as_str()).collect()
}

/// The answer of extension `ext` to `request`, as a worker leaves it.
pub(super) fn answer(ext: &str, request: Request, result: Result<Reply, &str>) -> Answer {
    let result = result.map(|reply| Parsed { reply, warnings: vec![] }).map_err(String::from);
    Answer { ext: ext.into(), request, result }
}

fn open_view(view: &str) -> Request {
    Request::Open { view: view.into() }
}

#[test]
fn root_items_are_cached_and_asked_again_when_the_launcher_opens() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    assert!(test_cx("", |cx| m.items(cx)).is_empty());
    // A second open while the first call is queued does not queue it again.
    assert!(!event(&mut m, Event::LauncherOpened));
    assert!(!event(&mut m, Event::LauncherOpened));
    assert_eq!(m.pending, [("echo".to_string(), Request::Root)]);
    assert_eq!(m.workers.len(), 1);
    assert!(settle(&mut m));
    let items = test_cx("", |cx| m.items(cx));
    assert_eq!(ids(&items), ["ext:echo/root"]);
    assert_eq!(items[0].subtitle, "/tmp/flick-test.sock");
    assert!(seen().is_empty());
}

#[test]
fn only_an_answer_is_stale_data_and_reload_asks_again() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    // Another module's change, an empty inbox or another event is not stale data for ext.
    assert!(!event(&mut m, Event::ModuleChanged { module: "ext" }));
    assert!(!event(&mut m, Event::ModuleChanged { module: "app" }));
    assert!(!event(&mut m, Event::Wake));
    assert!(!event(&mut m, Event::Reloaded));
    assert_eq!(m.pending.len(), 1);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut stale = false;
    while !stale && Instant::now() < deadline {
        stale = event(&mut m, Event::ModuleChanged { module: "ext" });
    }
    assert!(stale && m.pending.is_empty());
    assert_eq!(ids(&test_cx("", |cx| m.items(cx))), ["ext:echo/root"]);
}

#[test]
fn a_view_opens_from_its_cache_and_is_asked_again() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    let view = test_cx("", |cx| m.open("echo/v", cx)).unwrap();
    assert!(view.is("ext", "echo/v") && view.record_use);
    assert_eq!((view.empty.as_str(), m.showing.as_deref()), (LOADING, Some("echo/v")));
    assert_eq!(m.pending, [("echo".to_string(), open_view("v"))]);
    assert!(settle(&mut m));
    let mut view = test_cx("", |cx| m.open("echo/v", cx)).unwrap();
    assert_eq!(view.empty, "No Results");
    test_cx("", |cx| m.refresh(&mut view, cx));
    assert_eq!((ids(&view.items), view.footer.as_str()), (vec!["ext:echo/open"], "echo"));
    test_cx("zzz", |cx| m.refresh(&mut view, cx));
    assert!(view.items.is_empty());
    test_cx("", |cx| m.closed("echo/other", cx));
    assert!(m.showing.is_some());
    test_cx("", |cx| m.closed("echo/v", cx));
    assert!(m.showing.is_none());
    assert!(settle(&mut m));
}

#[test]
fn a_failed_view_call_shows_in_its_footer_while_it_is_on_the_screen() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    m.showing = Some("echo/v".into());
    m.inbox.push(answer("echo", open_view("v"), Err("echo: boom")));
    assert!(m.drain());
    assert_eq!(seen(), ["log echo: boom"]);
    let mut view = ListView::new("ext", "echo/v");
    test_cx("", |cx| m.refresh(&mut view, cx));
    assert_eq!((view.footer.as_str(), view.empty.as_str()), ("echo: boom", "No Results"));
    // A good answer clears it.
    m.inbox.push(answer("echo", open_view("v"), Ok(Reply::View(View::default()))));
    assert!(m.drain());
    test_cx("", |cx| m.refresh(&mut view, cx));
    assert_eq!(view.footer, "echo");
    // Hidden launcher, another view, or a root call: a notification.
    m.hooks.visible = || false;
    m.inbox.push(answer("echo", open_view("v"), Err("echo: hidden")));
    m.inbox.push(answer("echo", open_view("w"), Err("echo: other")));
    m.inbox.push(answer("echo", Request::Root, Err("echo: root")));
    assert!(!m.drain());
    let want = ["hidden", "other", "root"].map(|e| [format!("log echo: {e}"), format!("notify echo | echo: {e}")]);
    assert_eq!(seen(), want.concat());
    assert!(m.error.is_none());
}

#[test]
fn answers_of_removed_extensions_and_to_events_change_nothing() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    m.inbox.push(answer("gone", Request::Root, Ok(Reply::Items(vec![]))));
    m.inbox.push(answer("gone", Request::Root, Err("gone: x")));
    m.inbox.push(answer("echo", Request::Event(proto::EventKind::Wake), Ok(Reply::Ack)));
    assert!(!m.drain());
    assert!(seen().is_empty() && m.root.is_empty());
}

#[test]
fn hotkeys_open_views() {
    let table = format!("{}hotkeys = {{ inbox = \"cmd+KeyI\", today = \"cmd+KeyT\" }}\n", entry("echo", &["echo"], 2000));
    let mut m = module(&table);
    let want = |view: &str, spec: &str| Binding { spec: spec.into(), key: Ok(format!("echo/{view}")) };
    assert_eq!(m.hotkeys(), [want("inbox", "cmd+KeyI"), want("today", "cmd+KeyT")]);
    let view = test_cx("", |cx| m.hotkey("echo/inbox", cx)).unwrap();
    assert!(view.is("ext", "echo/inbox"));
    assert!(test_cx("", |cx| m.hotkey("nope/inbox", cx)).is_none());
    assert!(test_cx("", |cx| m.hotkey("inbox", cx)).is_none());
    assert!(module("").hotkeys().is_empty());
}

#[test]
fn unknown_extensions_are_refused() {
    let mut m = module("");
    assert_eq!(m.call("nope", Request::Root).unwrap_err(), "ext: no extension \"nope\"");
    m.fetch("nope", Request::Root);
    assert_eq!(seen(), ["log ext: no extension \"nope\""]);
    assert!(m.workers.is_empty() && m.pending.is_empty());
    assert!(test_cx("", |cx| m.open("nope/v", cx)).is_none());
    assert!(test_cx("", |cx| m.open("nope", cx)).is_none());
}

#[test]
fn a_slow_extension_never_holds_the_caller_and_its_timeout_is_reported() {
    let dir = Scratch::new("ext-slow-module");
    let pids = dir.join("pids");
    let mut m = module(&entry("slow", &["sleep", "2", pids.to_str().unwrap()], 500));
    let started = Instant::now();
    let view = test_cx("", |cx| m.open("slow/v", cx)).unwrap();
    assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
    assert_eq!(view.empty, LOADING);
    assert!(!m.drain());
    m.hooks.visible = || false;
    let error = "slow: no reply in 500 ms; stopped";
    assert_eq!(wait_seen(&mut m), [format!("log {error}"), format!("notify slow | {error}")]);
}

#[test]
fn failures_are_logged_and_notified_with_the_name() {
    let mut m = module(&entry("broken", &["fail"], 2000));
    m.call("broken", Request::Root).unwrap();
    assert_eq!(wait_seen(&mut m), ["log broken: bad token", "notify broken | broken: bad token"]);
}

#[test]
fn warnings_are_logged_with_a_cap() {
    let mut m = module(&entry("bad", &["bad", "20"], 2000));
    assert!(!event(&mut m, Event::LauncherOpened));
    assert!(settle(&mut m));
    assert!(test_cx("", |cx| m.items(cx)).is_empty());
    let lines = seen();
    assert_eq!(lines.len(), WARNINGS_LOGGED + 1, "{lines:?}");
    assert_eq!(lines[0], "log bad: root: items[0]: no title; skipped");
    assert_eq!(lines[WARNINGS_LOGGED], "log bad: root: 15 more warnings");
    m.log_warnings("bad", &Request::Root, &["one".into()]);
    assert_eq!(seen(), ["log bad: root: one"]);
}

#[test]
fn a_reload_keeps_only_the_workers_of_unchanged_extensions() {
    let both = format!("{}{}", entry("a", &["echo"], 2000), entry("b", &["echo"], 2000));
    let mut m = module(&both);
    event(&mut m, Event::Reloaded);
    test_cx("", |cx| m.open("b/v", cx));
    assert!(settle(&mut m));
    assert_eq!(ids(&test_cx("", |cx| m.items(cx))), ["ext:a/root", "ext:b/root"]);
    let changed = format!("{}{}", entry("a", &["echo"], 2000), entry("b", &["echo"], 900));
    configure(&mut m, &changed).unwrap();
    let names: Vec<&str> = m.workers.iter().map(|w| w.spec().name.as_str()).collect();
    assert_eq!(names, ["a"]);
    assert_eq!((m.root.len(), m.views.len()), (2, 1));
    m.error = Some(("b/v".into(), "b: x".into()));
    m.status = Some(("b/v".into(), "Copy…".into()));
    let form = proto::form::ExtForm { title: "F".into(), submit: "S".into(), fields: vec![] };
    m.forms.insert("b/f".into(), form);
    m.form = Some(("b/f".into(), forms::Phase::Open));
    let ran = act::Ran { ext: "b".into(), view: None, item: "i".into() };
    let action = proto::item::ExtAction {
        key: "a".into(),
        title: "A".into(),
        icon: None,
        run: proto::item::Run::Ext,
        after: proto::item::After::Hide,
        confirm: None,
    };
    m.asked = Some(Asked { token: "t".into(), run: ran, action });
    configure(&mut m, &entry("a", &["echo"], 2000)).unwrap();
    assert_eq!((m.root.len(), m.views.len(), m.error.is_some()), (1, 0, false));
    assert!(m.status.is_none() && m.forms.is_empty() && m.form.is_none() && m.asked.is_none());
    assert!(configure(&mut m, &entry("Bad Name", &[], 1)).is_err());
    assert_eq!(m.specs.len(), 1);
    configure(&mut m, "").unwrap();
    assert!(m.workers.is_empty() && m.specs.is_empty());
    assert!(configure(&mut m, "[ext]\nextensions = 1").unwrap_err().starts_with("[ext]"));
}

#[test]
fn a_worker_whose_thread_died_is_started_again() {
    fn panics_on_open(spec: &Spec, request: &Request, socket: &std::path::Path, stop: &dyn Fn() -> bool) -> Result<Parsed, String> {
        assert_ne!(request.op(), proto::Op::Open, "extension call panicked");
        transport::call(spec, request, socket, stop)
    }
    let mut m = module(&entry("echo", &["echo"], 2000));
    m.hooks.job.run = panics_on_open;
    m.call("echo", Request::Open { view: "v".into() }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while m.workers[0].send(Request::Event(proto::EventKind::Wake)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    // The dead worker's queued call is never answered; asking again does not wait on it.
    m.pending.push(("echo".into(), Request::Root));
    m.fetch("echo", Request::Root);
    assert_eq!(m.workers.len(), 1);
    assert!(settle(&mut m));
    assert_eq!(ids(&test_cx("", |cx| m.items(cx))), ["ext:echo/root"]);
}

#[test]
fn the_real_hooks_are_installed() {
    let m = Ext::new(socket, open);
    assert_eq!(m.id(), "ext");
    assert_eq!((m.hooks.job.socket)(), socket());
    assert!(m.root.is_empty() && m.showing.is_none());
}
