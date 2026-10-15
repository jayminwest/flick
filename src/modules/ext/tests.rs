use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::proto::Reply;
use super::*;
use crate::core::scratch::Scratch;
use crate::core::test_cx;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ext-echo.sh");

thread_local! {
    /// What the hooks saw on this thread: `log <line>` and `notify <ext>: <error>`.
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}

fn log(line: &str) {
    SEEN.with(|s| s.borrow_mut().push(format!("log {line}")));
}

fn notify(ext: &str, error: &str) {
    SEEN.with(|s| s.borrow_mut().push(format!("notify {ext} | {error}")));
}

fn seen() -> Vec<String> {
    SEEN.with(RefCell::take)
}

fn socket() -> PathBuf {
    "/tmp/flick-test.sock".into()
}

/// An `ext` over the real transport and the fixture, with recording hooks.
fn module(table: &str) -> Ext {
    let job = Job { run: transport::call, socket, post: || {} };
    let mut m = Ext::with(Hooks { job, log, notify });
    configure(&mut m, table).unwrap();
    m
}

fn configure(m: &mut Ext, table: &str) -> Result<(), String> {
    let config = crate::config::parse(table)?;
    m.configure(&config.section("ext")?.unwrap_or_else(|| Section::empty("ext")))
}

/// `[[ext.extensions]]` for the fixture in `mode`.
fn entry(name: &str, mode: &[&str], timeout_ms: u64) -> String {
    let argv: Vec<String> = std::iter::once(FIXTURE).chain(mode.iter().copied()).map(|a| format!("{a:?}")).collect();
    format!("[[ext.extensions]]\nname = \"{name}\"\nargv = [{}]\ntimeout_ms = {timeout_ms}\n", argv.join(", "))
}

/// Drain until `n` answers arrived (bounded), as the main thread does on `ModuleChanged`.
fn answers(m: &mut Ext, n: usize) -> Vec<(String, Request, Parsed)> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut got = vec![];
    while got.len() < n && Instant::now() < deadline {
        got.extend(m.drain());
        std::thread::sleep(Duration::from_millis(5));
    }
    got
}

/// Drain until something was logged (bounded).
fn wait_seen(m: &mut Ext) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut got = vec![];
    while got.is_empty() && Instant::now() < deadline {
        assert!(m.drain().is_empty());
        got = seen();
        std::thread::sleep(Duration::from_millis(5));
    }
    got
}

#[test]
fn a_call_queues_on_the_worker_and_the_answer_arrives_with_module_changed() {
    let mut m = module(&entry("echo", &["echo"], 2000));
    m.call("echo", Request::Root).unwrap();
    m.call("echo", Request::Open { view: "v".into() }).unwrap();
    assert_eq!(m.workers.len(), 1);
    let got = answers(&mut m, 2);
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].0.as_str(), &got[0].1), ("echo", &Request::Root));
    let Reply::Items(items) = &got[0].2.reply else { panic!("{:?}", got[0].2.reply) };
    assert_eq!(items[0].subtitle, "/tmp/flick-test.sock");
    assert_eq!(got[1].1, Request::Open { view: "v".into() });
    assert!(seen().is_empty());
    // Another module's change, or an empty inbox, is not stale data for ext.
    assert!(!test_cx("", |cx| m.on_event(Event::ModuleChanged { module: "ext" }, cx)));
    m.call("echo", Request::Root).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut stale = false;
    while !stale && Instant::now() < deadline {
        assert!(!test_cx("", |cx| m.on_event(Event::ModuleChanged { module: "app" }, cx)));
        stale = test_cx("", |cx| m.on_event(Event::ModuleChanged { module: "ext" }, cx));
    }
    assert!(stale);
}

#[test]
fn unknown_extensions_are_refused() {
    let mut m = module("");
    assert_eq!(m.call("nope", Request::Root).unwrap_err(), "ext: no extension \"nope\"");
    assert!(m.workers.is_empty());
}

#[test]
fn a_slow_extension_never_holds_the_caller_and_its_timeout_is_reported() {
    let dir = Scratch::new("ext-slow-module");
    let pids = dir.join("pids");
    let mut m = module(&entry("slow", &["sleep", "2", pids.to_str().unwrap()], 500));
    let started = Instant::now();
    m.call("slow", Request::Root).unwrap();
    assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
    assert!(m.drain().is_empty());
    let error = "slow: no reply in 500 ms; stopped";
    assert_eq!(wait_seen(&mut m), [format!("log {error}"), format!("notify slow | {error}")]);
}

#[test]
fn failures_are_logged_and_notified_with_the_name() {
    let mut m = module(&entry("broken", &["fail"], 2000));
    m.call("broken", Request::Form { form: "f".into() }).unwrap();
    assert_eq!(wait_seen(&mut m), ["log broken: bad token", "notify broken | broken: bad token"]);
}

#[test]
fn warnings_are_logged_with_a_cap() {
    let mut m = module(&entry("bad", &["bad", "20"], 2000));
    m.call("bad", Request::Root).unwrap();
    let got = answers(&mut m, 1);
    assert_eq!(got[0].2.warnings.len(), 20);
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
    m.call("a", Request::Root).unwrap();
    m.call("b", Request::Root).unwrap();
    assert_eq!(answers(&mut m, 2).len(), 2);
    let changed = format!("{}{}", entry("a", &["echo"], 2000), entry("b", &["echo"], 900));
    configure(&mut m, &changed).unwrap();
    let names: Vec<&str> = m.workers.iter().map(|w| w.spec().name.as_str()).collect();
    assert_eq!(names, ["a"]);
    assert!(configure(&mut m, &entry("Bad Name", &[], 1)).is_err());
    assert_eq!(m.specs.len(), 2);
    configure(&mut m, "").unwrap();
    assert!(m.workers.is_empty() && m.specs.is_empty());
    assert!(configure(&mut m, "[ext]\nextensions = 1").unwrap_err().starts_with("[ext]"));
}

#[test]
fn a_worker_whose_thread_died_is_started_again() {
    fn panics_on_open(spec: &Spec, request: &Request, socket: &std::path::Path) -> Result<Parsed, String> {
        assert_ne!(request.op(), proto::Op::Open, "extension call panicked");
        transport::call(spec, request, socket)
    }
    let mut m = module(&entry("echo", &["echo"], 2000));
    m.hooks.job.run = panics_on_open;
    m.call("echo", Request::Open { view: "v".into() }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while m.workers[0].send(Request::Event(proto::EventKind::Wake)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    m.call("echo", Request::Root).unwrap();
    assert_eq!(m.workers.len(), 1);
    let got = answers(&mut m, 1);
    assert_eq!(got.last().map(|a| a.1.clone()), Some(Request::Root));
}

#[test]
fn the_real_hooks_are_installed() {
    let m = Ext::new(socket);
    assert_eq!(m.id(), "ext");
    assert_eq!((m.hooks.job.socket)(), socket());
}
