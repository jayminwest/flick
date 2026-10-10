use super::*;
use crate::modules::sys::check::Status;
use crate::modules::sys::settings::Target;
use crate::modules::sys::testkit::{HOOKS, SERVER};

fn svc(name: &str, kind: Kind, target: &str) -> Service {
    let target = match kind {
        Kind::Command => Target::Argv(target.split(' ').map(str::to_string).collect()),
        _ => Target::One(target.into()),
    };
    Service { name: name.into(), kind, target, warn: None, fail: None, log: None, restart: false, domain: Domain::Gui }
}

fn verdict(service: &Service) -> (Status, String) {
    let v = check_one(service, HOOKS);
    (v.status, v.reason)
}

fn wait_for(shared: &Shared, done: impl Fn(&State) -> bool) {
    assert!(shared.wait(Duration::from_secs(15), done), "timed out");
}

#[test]
fn each_kind_runs_its_check() {
    let ok = |r: &str| (Status::Ok, r.to_string());
    assert_eq!(verdict(&svc("a", Kind::Http, "http://ok/")), ok("HTTP 200 in 12 ms"));
    assert_eq!(verdict(&svc("a", Kind::Http, "http://gone/")), (Status::Fail, "HTTP 404".into()));
    assert_eq!(verdict(&svc("a", Kind::Http, "http://down/")).0, Status::Fail);
    assert_eq!(verdict(&svc("a", Kind::Tcp, "open:4")), ok("open in 4 ms"));
    assert_eq!(verdict(&svc("a", Kind::Tcp, "shut:1")), (Status::Fail, "shut:1: Connection refused".into()));
    assert_eq!(verdict(&svc("a", Kind::Launchd, "up")), ok("running (pid 919)"));
    assert_eq!(verdict(&svc("a", Kind::Launchd, "killed")).0, Status::Warn);
    assert_eq!(verdict(&svc("a", Kind::Launchd, "nope")), (Status::Fail, "not loaded in gui/501".into()));
    assert_eq!(verdict(&svc("a", Kind::Process, "syncthing")), ok("running (pid 917, 958)"));
    assert_eq!(verdict(&svc("a", Kind::Process, "nope")), (Status::Fail, "not running".into()));
    let mut backlog = svc("a", Kind::Command, "/fake/print 12");
    backlog.warn = Some(10.0);
    assert_eq!(verdict(&backlog), (Status::Warn, "12 (warn >= 10)".into()));
    let missing = svc("a", Kind::Command, "/nope");
    assert_eq!(verdict(&missing), (Status::Fail, "/nope: No such file or directory".into()));
    let no_uid = Hooks { uid: || None, ..HOOKS };
    let v = check_one(&svc("a", Kind::Launchd, "up"), no_uid);
    assert_eq!((v.status, v.reason.as_str()), (Status::Unknown, "no uid for the launchd domain"));
}

#[test]
fn the_probe_fills_the_snapshot_once_per_min_age() {
    let shared = Arc::new(Shared::default());
    assert!(refresh_probe(&shared, HOOKS));
    wait_for(&shared, |s| !s.probing);
    let snap = shared.lock().snapshot.clone().unwrap();
    assert_eq!(snap, probe::parse(SERVER, 1_000));
    // Fresh: no second run.
    assert!(!refresh_probe(&shared, HOOKS));
    let later = Hooks { now: || 1_000 + MIN_AGE, ..HOOKS };
    assert!(refresh_probe(&shared, later));
    // One at a time.
    assert!(!refresh_probe(&shared, later));
    wait_for(&shared, |s| !s.probing);
    assert_eq!(shared.lock().snapshot.as_ref().unwrap().at, 1_000 + MIN_AGE);
}

#[test]
fn a_failed_probe_keeps_the_last_snapshot() {
    let shared = Arc::new(Shared::default());
    refresh_probe(&shared, HOOKS);
    wait_for(&shared, |s| !s.probing);
    let fails = Hooks { now: || 2_000, run: |_, _| Err("timed out after 2 s".into()), ..HOOKS };
    refresh_probe(&shared, fails);
    wait_for(&shared, |s| !s.probing);
    {
        let st = shared.lock();
        assert_eq!(st.probe_error.as_deref(), Some("probe: timed out after 2 s"));
        assert_eq!(st.snapshot.as_ref().unwrap().at, 1_000);
    }
    let silent = Hooks { now: || 3_000, run: |_, _| Ok(Exit { code: Some(0), ..Exit::default() }), ..HOOKS };
    refresh_probe(&shared, silent);
    wait_for(&shared, |s| !s.probing);
    assert_eq!(shared.lock().probe_error.as_deref(), Some("probe: no output"));
    // A good run clears the error.
    refresh_probe(&shared, Hooks { now: || 4_000, ..HOOKS });
    wait_for(&shared, |s| !s.probing);
    assert_eq!(shared.lock().probe_error, None);
}

#[test]
fn a_round_checks_every_service_in_parallel() {
    let shared = Arc::new(Shared::default());
    assert!(!refresh_services(&shared, HOOKS), "no services, no round");
    let services = [svc("web", Kind::Http, "http://ok/"), svc("sync", Kind::Process, "syncthing"), svc("db", Kind::Tcp, "shut:5432")];
    shared.set_services(&services);
    assert!(refresh_services(&shared, HOOKS));
    assert!(!refresh_services(&shared, HOOKS), "one round at a time");
    wait_for(&shared, |s| !s.checking);
    let st = shared.lock();
    let got: Vec<(Status, Option<u64>)> = st.services.iter().map(|e| (e.verdict.as_ref().unwrap().status, e.checked_at)).collect();
    assert_eq!(got, [(Status::Ok, Some(1_000)), (Status::Ok, Some(1_000)), (Status::Fail, Some(1_000))]);
    drop(st);
    // Rate limit, then a later ask runs again.
    assert!(!refresh_services(&shared, HOOKS));
    assert!(refresh_services(&shared, Hooks { now: || 1_000 + MIN_AGE, ..HOOKS }));
    wait_for(&shared, |s| !s.checking);
}

#[test]
fn a_reload_keeps_unchanged_verdicts_and_drops_late_ones() {
    let shared = Arc::new(Shared::default());
    let web = svc("web", Kind::Http, "http://ok/");
    shared.set_services(std::slice::from_ref(&web));
    refresh_services(&shared, HOOKS);
    wait_for(&shared, |s| !s.checking);
    let slow = svc("slow", Kind::Command, "/fake/sleep 300");
    shared.set_services(&[web.clone(), slow.clone()]);
    {
        let st = shared.lock();
        assert!(st.services[0].verdict.is_some(), "unchanged service keeps its verdict");
        assert!(st.services[1].verdict.is_none());
    }
    assert!(refresh_services(&shared, HOOKS), "a reload resets the rate limit");
    // Reload while `slow` runs: its result belongs to the old list and is dropped.
    shared.set_services(&[slow]);
    assert!(!shared.lock().checking);
    // The check threads hold clones of `shared` until they end.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while Arc::strong_count(&shared) > 1 {
        assert!(std::time::Instant::now() < deadline, "the check threads did not end");
        std::thread::sleep(Duration::from_millis(5));
    }
    let st = shared.lock();
    assert_eq!(st.services.len(), 1);
    assert_eq!(st.services[0].verdict, None);
    assert!(!st.checking);
}

#[test]
fn wait_gives_up_at_the_budget() {
    let shared = Shared::default();
    let start = Instant::now();
    assert!(!shared.wait(Duration::from_millis(50), |s| s.snapshot.is_some()));
    assert!(start.elapsed() >= Duration::from_millis(50));
    assert!(shared.wait(Duration::ZERO, |s| s.snapshot.is_none()));
}
