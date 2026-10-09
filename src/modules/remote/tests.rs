//! Module tests over fake `NetHooks`: a thread-local log of applies and a canned status. No
//! test opens a socket or runs the tailscale CLI.

use std::cell::RefCell;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use super::*;
use crate::config::parse;
use crate::core::Ranker;
use crate::core::store::Store;

/// 2026-10-09 18:31:01 UTC.
const TS: i64 = 1_791_570_661;
const TS_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(100, 101, 102, 103));

thread_local! {
    static APPLIED: RefCell<Vec<Option<NetSettings>>> = const { RefCell::new(Vec::new()) };
    static STATUS: RefCell<NetStatus> = RefCell::new(NetStatus::default());
    static FAIL: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn fake_apply(s: Option<NetSettings>) -> Result<NetStatus, String> {
    APPLIED.with(|a| a.borrow_mut().push(s.clone()));
    if let Some(e) = FAIL.with(|f| f.borrow().clone()) {
        return Err(e);
    }
    let status = match s {
        Some(s) => NetStatus {
            listening: vec![SocketAddr::new(TS_IP, s.port)],
            peers: s.peers,
            ..NetStatus::default()
        },
        None => NetStatus::default(),
    };
    STATUS.with(|st| *st.borrow_mut() = status.clone());
    Ok(status)
}

fn fake_status() -> NetStatus {
    STATUS.with(|st| st.borrow().clone())
}

const HOOKS: NetHooks = NetHooks { apply: fake_apply, status: fake_status };

/// Applies since the last call.
fn applied() -> Vec<Option<NetSettings>> {
    APPLIED.with(|a| std::mem::take(&mut *a.borrow_mut()))
}

fn store() -> Store {
    let s = Store::in_memory();
    s.migrate("remote", MIGRATIONS).unwrap();
    s
}

fn remote(config: &str) -> Remote {
    let mut r = Remote::new(HOOKS);
    r.now = || TS;
    configure(&mut r, config).unwrap();
    r
}

fn configure(r: &mut Remote, config: &str) -> Result<(), String> {
    r.configure(&parse(config).unwrap().section("remote").unwrap().unwrap())
}

fn with_cx<R>(store: &Store, json: bool, remote: bool, f: impl FnOnce(&mut Cx) -> R) -> R {
    let mut ranker = Ranker::new();
    f(&mut Cx { query: "", store, ranker: &mut ranker, hide: || {}, json, remote })
}

fn run(r: &mut Remote, s: &Store, words: &[&str]) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    with_cx(s, false, false, |cx| r.command(&args, cx))
}

fn event(r: &mut Remote, s: &Store, e: Event) {
    assert!(!with_cx(s, false, false, |cx| r.on_event(e, cx)));
}

fn net(peers: &[&str], port: u16, events: bool) -> NetSettings {
    NetSettings { peers: peers.iter().map(|p| (*p).to_string()).collect(), port, events }
}

const PEERS: &str = "[remote]\npeers = [\"mbp-server\", \" \"]";

#[test]
fn settings_default_and_validate() {
    let r = remote("");
    assert_eq!(r.settings, Settings::default());
    assert_eq!(r.settings.port, DEFAULT_PORT);
    let r = remote("[remote]\npeers = [\" a \", \"\"]\nport = 9000\nevents = true");
    assert_eq!(r.settings, Settings { peers: vec!["a".into()], port: 9000, events: true });
    let mut r = remote("");
    assert_eq!(
        configure(&mut r, "[remote]\nport = 0").unwrap_err(),
        "[remote] port: must be 1-65535"
    );
    assert!(configure(&mut r, "[remote]\nport = 70000").unwrap_err().starts_with("[remote]"));
    assert!(configure(&mut r, "[remote]\npeers = \"a\"").is_err());
    assert!(applied().is_empty(), "configure before Started applies nothing");
}

#[test]
fn off_by_default_and_started_stops() {
    let s = store();
    let mut r = remote(PEERS);
    event(&mut r, &s, Event::Wake);
    assert!(applied().is_empty(), "nothing before Started");
    event(&mut r, &s, Event::Started);
    assert_eq!(applied(), [None]);
    let text = run(&mut r, &s, &["status"]).unwrap();
    assert_eq!(
        text,
        "network access: off\nlistening: not listening\npeers: mbp-server\nlast connection: none"
    );
    // Off: Wake and LauncherOpened leave it alone.
    event(&mut r, &s, Event::Wake);
    event(&mut r, &s, Event::LauncherOpened);
    assert!(applied().is_empty());
    drop(r);
    assert!(applied().is_empty(), "nothing to stop");
}

#[test]
fn on_off_apply_and_persist() {
    let s = store();
    let mut r = remote("[remote]\npeers = [\"mbp-server\"]\nport = 9000");
    event(&mut r, &s, Event::Started);
    applied();
    assert_eq!(
        run(&mut r, &s, &["on"]).unwrap(),
        "Network access on · listening on 100.101.102.103:9000"
    );
    assert_eq!(applied(), [Some(net(&["mbp-server"], 9000, false))]);
    assert!(s.network_on());
    let status = run(&mut r, &s, &["status"]).unwrap();
    assert!(
        status.starts_with("network access: on\nlistening: 100.101.102.103:9000\n"),
        "{status}"
    );

    // A restart reads the switch back.
    let mut again = remote("[remote]\npeers = [\"mbp-server\"]\nport = 9000");
    event(&mut again, &s, Event::Started);
    assert_eq!(applied(), [Some(net(&["mbp-server"], 9000, false))]);

    assert_eq!(run(&mut r, &s, &["off"]).unwrap(), "Network access off");
    assert_eq!(applied(), [None]);
    assert!(!s.network_on());
    drop(r);
    assert!(applied().is_empty());
    drop(again);
    assert_eq!(applied(), [None], "dropping a listening module stops the listener");
}

#[test]
fn on_without_peers_does_not_listen() {
    let s = store();
    let mut r = remote("");
    event(&mut r, &s, Event::Started);
    applied();
    assert_eq!(run(&mut r, &s, &["on"]).unwrap(), format!("Network access on · {NO_PEERS}"));
    assert_eq!(applied(), [None]);
    let status = run(&mut r, &s, &["status"]).unwrap();
    assert!(status.ends_with(&format!("peers: none\nlast connection: none\nerror: {NO_PEERS}")));
    event(&mut r, &s, Event::LauncherOpened);
    assert!(applied().is_empty());
}

#[test]
fn reload_applies_only_changes() {
    let s = store();
    s.set_network_on(true);
    let mut r = remote(PEERS);
    event(&mut r, &s, Event::Started);
    assert_eq!(applied(), [Some(net(&["mbp-server"], DEFAULT_PORT, false))]);
    configure(&mut r, PEERS).unwrap();
    assert!(applied().is_empty(), "same settings: no restart");
    configure(&mut r, "[remote]\npeers = [\"mbp-server\"]\nevents = true").unwrap();
    assert_eq!(applied(), [Some(net(&["mbp-server"], DEFAULT_PORT, true))]);
    configure(&mut r, "").unwrap();
    assert_eq!(applied(), [None]);
    assert!(configure(&mut r, "[remote]\nport = 0").is_err());
    assert!(applied().is_empty());
    // Off: a reload with peers starts nothing.
    run(&mut r, &s, &["off"]).unwrap();
    applied();
    configure(&mut r, PEERS).unwrap();
    assert!(applied().is_empty());
}

#[test]
fn apply_errors_show_and_retry_on_wake() {
    let s = store();
    s.set_network_on(true);
    let mut r = remote(PEERS);
    FAIL.with(|f| *f.borrow_mut() = Some("tailscale: not running".into()));
    event(&mut r, &s, Event::Started);
    applied();
    let status = run(&mut r, &s, &["status"]).unwrap();
    assert!(status.contains("listening: not listening\n"), "{status}");
    assert!(status.ends_with("error: tailscale: not running"), "{status}");
    assert_eq!(
        run(&mut r, &s, &["on"]).unwrap(),
        "Network access on · not listening: tailscale: not running"
    );
    applied();
    FAIL.with(|f| *f.borrow_mut() = None);
    event(&mut r, &s, Event::Wake);
    assert_eq!(applied(), [Some(net(&["mbp-server"], DEFAULT_PORT, false))]);
    assert!(!run(&mut r, &s, &["status"]).unwrap().contains("error:"));
    // Listening: LauncherOpened does not restart it.
    event(&mut r, &s, Event::LauncherOpened);
    event(&mut r, &s, Event::Idle { secs: 60 });
    assert!(applied().is_empty());
    // The transport lost its addresses: LauncherOpened retries.
    STATUS.with(|st| *st.borrow_mut() = NetStatus::default());
    event(&mut r, &s, Event::LauncherOpened);
    assert_eq!(applied().len(), 1);
}

#[test]
fn an_ok_apply_that_does_not_listen_says_so() {
    let s = store();
    let mut r = remote(PEERS);
    r.hooks = NetHooks { apply: |_| Ok(NetStatus::default()), status: fake_status };
    assert_eq!(run(&mut r, &s, &["on"]).unwrap(), "Network access on · not listening");
    r.applied = None;
}

#[test]
fn status_reports_the_last_connection_and_errors() {
    let s = store();
    let mut r = remote(PEERS);
    let last = |name: Option<&str>, at: i64, allowed: bool, reason: Option<&str>| LastConn {
        name: name.map(String::from),
        ip: TS_IP,
        at,
        allowed,
        reason: reason.map(String::from),
    };
    let cases = [
        (
            last(Some("mbp-server"), TS - 5, true, None),
            "mbp-server (100.101.102.103) allowed 5s ago",
        ),
        (
            last(Some("phone"), TS - 120, false, Some("not in peers")),
            "phone (100.101.102.103) refused 2m ago: not in peers",
        ),
        (last(None, TS - 7200, false, None), "unknown (100.101.102.103) refused 2h ago"),
        (last(None, TS - 172_800, false, None), "unknown (100.101.102.103) refused 2d ago"),
        (last(None, TS + 9, true, None), "unknown (100.101.102.103) allowed 0s ago"),
    ];
    for (conn, want) in cases {
        STATUS.with(|st| {
            *st.borrow_mut() = NetStatus {
                last: Some(conn),
                error: Some("no Tailscale address".into()),
                ..NetStatus::default()
            };
        });
        let text = run(&mut r, &s, &["status"]).unwrap();
        assert!(
            text.ends_with(&format!("last connection: {want}\nerror: no Tailscale address")),
            "{text}"
        );
    }
}

#[test]
fn status_json() {
    let s = store();
    let mut r = remote("[remote]\npeers = [\"mbp-server\"]\nevents = true");
    event(&mut r, &s, Event::Started);
    with_cx(&s, false, false, |cx| r.command(&["on".into()], cx)).unwrap();
    let text = with_cx(&s, true, true, |cx| r.command(&["status".into()], cx)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v,
        serde_json::json!({
            "on": true, "port": DEFAULT_PORT, "events": true, "peers": ["mbp-server"],
            "listening": [format!("100.101.102.103:{DEFAULT_PORT}")], "last": null, "error": null,
        })
    );
    run(&mut r, &s, &["off"]).unwrap();
}

#[test]
fn remote_callers_only_read_status() {
    let s = store();
    let mut r = remote(PEERS);
    event(&mut r, &s, Event::Started);
    applied();
    for verb in ["on", "off"] {
        let reply = with_cx(&s, false, true, |cx| r.command(&[verb.into()], cx));
        assert_eq!(reply.unwrap_err(), REFUSED);
    }
    assert!(applied().is_empty());
    assert!(!s.network_on());
    assert!(with_cx(&s, false, true, |cx| r.command(&["status".into()], cx)).is_ok());
}

#[test]
fn bad_verbs() {
    let s = store();
    let mut r = remote("");
    for words in [&["status", "x"][..], &["on", "now"]] {
        assert_eq!(run(&mut r, &s, words).unwrap_err(), USAGE);
    }
    assert_eq!(run(&mut r, &s, &["zap"]).unwrap_err(), unknown_verb(ID, &["zap".into()]));
    assert_eq!(run(&mut r, &s, &[]).unwrap_err(), unknown_verb(ID, &[]));
    assert_eq!(r.verbs(), "remote status|on|off");
    assert_eq!(r.id(), "remote");
    assert_eq!(r.migrations(), MIGRATIONS);
}

#[test]
fn the_root_item_toggles() {
    let s = store();
    let mut r = remote(PEERS);
    event(&mut r, &s, Event::Started);
    applied();
    let item = |r: &mut Remote| with_cx(&s, false, false, |cx| r.items(cx)).remove(0);
    let off = item(&mut r);
    assert_eq!(off.id.to_string(), "remote:network");
    assert_eq!(off.title, "Turn On Network Access");
    let toggle = |r: &mut Remote| with_cx(&s, false, false, |cx| r.activate(&off.id, cx));
    assert!(matches!(toggle(&mut r), Outcome::Stay(Some(t)) if t.starts_with("Network access on")));
    assert_eq!(item(&mut r).title, "Turn Off Network Access");
    assert!(matches!(toggle(&mut r), Outcome::Stay(Some(t)) if t == "Network access off"));
    assert_eq!(applied(), [Some(net(&["mbp-server"], DEFAULT_PORT, false)), None]);
    let other = ItemId::new(ID, "other");
    assert!(matches!(with_cx(&s, false, false, |cx| r.activate(&other, cx)), Outcome::Stay(None)));
}
