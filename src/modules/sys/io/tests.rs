use super::*;
use crate::modules::sys::testkit::{HOOKS, SERVER};

fn wait_for(shared: &Shared, done: impl Fn(&State) -> bool) {
    assert!(shared.wait(Duration::from_secs(5), done), "timed out");
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
fn wait_gives_up_at_the_budget() {
    let shared = Shared::default();
    let start = Instant::now();
    assert!(!shared.wait(Duration::from_millis(50), |s| s.snapshot.is_some()));
    assert!(start.elapsed() >= Duration::from_millis(50));
    assert!(shared.wait(Duration::ZERO, |s| s.snapshot.is_none()));
}
