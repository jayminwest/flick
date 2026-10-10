//! Loopback tests: a fake tailnet, listeners on 127.0.0.1 through `Net::loopback` only.

use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::time::Instant;

/// A tailnet with fixed addresses whose whois gives every peer the same answer.
struct Fake {
    ips: Vec<&'static str>,
    whois: Result<Vec<&'static str>, &'static str>,
}

impl Tailnet for Fake {
    fn ips(&self) -> Result<Vec<IpAddr>, String> {
        Ok(self.ips.iter().map(|ip| ip.parse().unwrap()).collect())
    }
    fn whois(&self, _: SocketAddr) -> Result<Vec<String>, String> {
        self.whois
            .clone()
            .map(|n| n.iter().map(|s| (*s).to_string()).collect())
            .map_err(String::from)
    }
}

/// Echoes the words and whether the request ran as a remote caller.
#[expect(clippy::needless_pass_by_value, reason = "a gate run function owns its words")]
fn echo(words: Vec<String>, flags: Flags) -> Reply {
    Reply::Ok(format!("{} remote={} json={}", words.join(" "), flags.remote, flags.json).into())
}

fn handler(words: Vec<String>) -> Reply {
    gate(words, echo)
}

fn net(whois: Result<Vec<&'static str>, &'static str>, hub: &'static Hub) -> &'static Net {
    Net::loopback(Box::new(Fake { ips: vec!["127.0.0.1"], whois }), handler, hub)
}

fn settings(peers: &[&str], events: bool) -> NetSettings {
    NetSettings { peers: peers.iter().map(|p| (*p).to_string()).collect(), port: 0, events }
}

fn connect(status: &NetStatus) -> BufReader<TcpStream> {
    let stream = TcpStream::connect(status.listening[0]).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    BufReader::new(stream)
}

fn ask(stream: &mut BufReader<TcpStream>, request: &str) -> String {
    writeln!(stream.get_mut(), "{request}").unwrap();
    let mut line = String::new();
    stream.read_line(&mut line).unwrap();
    line
}

/// Whether the server closed `stream` without a reply.
fn closed_unanswered(stream: &mut BufReader<TcpStream>) -> bool {
    let _ = writeln!(stream.get_mut(), r#"["task","ls"]"#);
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).map_or(true, |_| rest.is_empty())
}

/// Whether no listener of `net` answers at `addr`. A stopped listener's port is free, so on a
/// busy machine another listener (another test, or another test process) can take it at
/// once: then the connection succeeds, but `net` never tracks it.
fn not_listening(net: &Net, addr: SocketAddr) -> bool {
    let Ok(stream) = TcpStream::connect(addr) else { return true };
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut stream = BufReader::new(stream);
    // A listener of `net` tracks a connection before it answers on it.
    let _ = writeln!(stream.get_mut(), r#"["x"]"#);
    let _ = stream.read_line(&mut String::new());
    lock(&net.conns).is_empty()
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

impl Net {
    /// `apply`, waiting for the outcome.
    fn sync(&'static self, settings: Option<NetSettings>) -> Result<NetStatus, String> {
        self.apply_now(self.wanted.fetch_add(1, Ordering::SeqCst) + 1, settings)
    }
}

fn words(w: &[&str]) -> Vec<String> {
    w.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn network_requests_always_run_as_remote_callers_and_pass_the_policy() {
    let ok = |w: &[&str]| match gate(words(w), echo) {
        Reply::Ok(v) => v.as_str().unwrap().to_string(),
        Reply::Error(e) => format!("error: {e}"),
    };
    assert_eq!(ok(&["task", "ls"]), "task ls remote=true json=false");
    assert_eq!(ok(&["task", "ls", "--remote", "--json"]), "task ls remote=true json=true");
    assert_eq!(ok(&["reload"]), "error: reload: not allowed over the network");
    assert_eq!(
        ok(&["capture", "screen", "--json"]),
        "error: capture screen: not allowed over the network"
    );
    assert_eq!(ok(&["remote", "off"]), "error: remote off: not allowed over the network");
    assert_eq!(ok(&["remote", "status"]), "remote status remote=true json=false");
}

#[test]
fn only_tailnet_addresses_are_permitted_outside_tests() {
    for ip in ["100.64.0.1", "100.127.255.254", "fd7a:115c:a1e0::1"] {
        assert!(permitted(ip.parse().unwrap(), false), "{ip}");
    }
    for ip in [
        "0.0.0.0",
        "::",
        "127.0.0.1",
        "::1",
        "192.168.1.2",
        "10.0.0.1",
        "100.128.0.1",
        "::ffff:100.64.0.1",
    ] {
        assert!(!permitted(ip.parse().unwrap(), false), "{ip}");
    }
    assert!(permitted("127.0.0.1".parse().unwrap(), true));
    assert!(!permitted("0.0.0.0".parse().unwrap(), true));
}

#[test]
fn the_production_transport_refuses_to_bind_non_tailnet_addresses() {
    static HUB: Hub = Hub::new();
    let ips = vec!["0.0.0.0", "::", "192.168.1.2", "127.0.0.1", "::1"];
    let net: &'static Net =
        Box::leak(Box::new(Net::new(Box::new(Fake { ips, whois: Ok(vec!["a"]) }), handler, &HUB)));
    let e = net.sync(Some(settings(&["a"], false))).unwrap_err();
    assert!(e.contains("no Tailscale address"), "{e}");
    let status = net.status();
    assert!(status.listening.is_empty());
    assert_eq!(status.error.as_deref(), Some(e.as_str()));
    assert!(status.peers.is_empty());
}

/// A tailnet whose daemon does not answer.
struct Down;

impl Tailnet for Down {
    fn ips(&self) -> Result<Vec<IpAddr>, String> {
        Err("tailscale ip: timed out after 2s".into())
    }
    fn whois(&self, _: SocketAddr) -> Result<Vec<String>, String> {
        Err("down".into())
    }
}

#[test]
fn no_peers_or_no_tailscale_means_no_listener() {
    static HUB: Hub = Hub::new();
    let e = net(Ok(vec!["a"]), &HUB).sync(Some(settings(&[], false))).unwrap_err();
    assert!(e.contains("no peers"), "{e}");
    let down = Net::loopback(Box::new(Down), handler, &HUB);
    assert_eq!(
        down.sync(Some(settings(&["a"], false))).unwrap_err(),
        "tailscale ip: timed out after 2s"
    );
    assert_eq!(down.status().error.as_deref(), Some("tailscale ip: timed out after 2s"));
    // Turning it off clears the error.
    assert_eq!(down.sync(None).unwrap(), NetStatus::default());
}

#[test]
fn an_allowed_peer_is_served_as_a_remote_caller() {
    static HUB: Hub = Hub::new();
    let net = net(Ok(vec!["mbp-server", "mbp-server.tail1.ts.net."]), &HUB);
    let status = net.sync(Some(settings(&["MBP-Server"], false))).unwrap();
    assert_eq!(status.listening.len(), 1);
    assert!(status.listening[0].ip().is_loopback());
    assert_eq!(status.peers, ["MBP-Server"]);
    assert_eq!(status.error, None);

    let mut c = connect(&status);
    assert_eq!(ask(&mut c, r#"["task","ls"]"#), "{\"ok\":\"task ls remote=true json=false\"}\n");
    assert_eq!(
        ask(&mut c, r#"["flick","rebuild"]"#),
        "{\"error\":\"flick rebuild: not allowed over the network\"}\n"
    );
    assert_eq!(
        ask(&mut c, r#"["events"]"#),
        "{\"error\":\"events: not allowed over the network\"}\n"
    );
    let last = net.status().last.unwrap();
    assert_eq!(last.name.as_deref(), Some("mbp-server"));
    assert!(last.ip.is_loopback());
    assert!(last.allowed);
    assert_eq!(last.reason, None);
    assert!(last.at > 0);
    net.sync(None).unwrap();
}

#[test]
fn an_unknown_peer_or_a_failed_whois_is_closed_before_any_request() {
    static HUB: Hub = Hub::new();
    let other = net(Ok(vec!["laptop"]), &HUB);
    let status = other.sync(Some(settings(&["mbp-server"], false))).unwrap();
    assert!(closed_unanswered(&mut connect(&status)));
    let last = other.status().last.unwrap();
    assert_eq!(last.name.as_deref(), Some("laptop"));
    assert!(!last.allowed);
    assert_eq!(last.reason.as_deref(), Some("laptop is not in [remote] peers"));
    other.sync(None).unwrap();

    for (whois, reason) in [
        (Err("tailscale whois: timed out after 2s"), "tailscale whois: timed out after 2s"),
        (Ok(vec![]), "peer is not in [remote] peers"),
    ] {
        let failing = net(whois, &HUB);
        let status = failing.sync(Some(settings(&["mbp-server"], false))).unwrap();
        assert!(closed_unanswered(&mut connect(&status)));
        let last = failing.status().last.unwrap();
        assert_eq!((last.allowed, last.reason.as_deref()), (false, Some(reason)));
        failing.sync(None).unwrap();
    }
}

#[test]
fn events_stream_over_the_network_only_when_allowed() {
    static HUB: Hub = Hub::new();
    let net = net(Ok(vec!["a"]), &HUB);
    let status = net.sync(Some(settings(&["a"], true))).unwrap();
    let mut c = connect(&status);
    c.get_mut().set_read_timeout(None).unwrap();
    writeln!(c.get_mut(), "[\"events\"]").unwrap();
    wait_for("a subscriber", || HUB.len() == 1);
    HUB.publish(|| r#"{"event":"wake"}"#.into());
    let mut line = String::new();
    c.read_line(&mut line).unwrap();
    assert_eq!(line, "{\"event\":\"wake\"}\n");
    // Stopping ends the stream and the subscription.
    net.sync(None).unwrap();
    wait_for("the subscriber to go", || HUB.len() == 0);
    let mut rest = String::new();
    assert_eq!(c.read_to_string(&mut rest).unwrap_or(0), 0);
}

#[test]
fn stopping_closes_the_listener_and_open_connections_and_it_can_restart() {
    static HUB: Hub = Hub::new();
    let net = net(Ok(vec!["a"]), &HUB);
    let status = net.sync(Some(settings(&["a"], false))).unwrap();
    let mut c = connect(&status);
    assert!(ask(&mut c, r#"["x"]"#).contains("remote=true"));
    let stopped = net.sync(None).unwrap();
    assert!(stopped.listening.is_empty());
    assert!(stopped.peers.is_empty());
    assert!(stopped.last.is_some(), "the last connection outlives a stop");
    let mut rest = String::new();
    assert_eq!(c.read_to_string(&mut rest).unwrap_or(0), 0);
    assert!(lock(&net.conns).is_empty());
    assert!(not_listening(net, status.listening[0]));

    let again = net.sync(Some(settings(&["a"], false))).unwrap();
    assert!(ask(&mut connect(&again), r#"["y"]"#).contains("y remote=true"));
    // Applying new settings replaces the running listener.
    let replaced = net.sync(Some(settings(&["b", "a"], false))).unwrap();
    assert!(again.listening == replaced.listening || not_listening(net, again.listening[0]));
    assert_eq!(net.status().peers, ["b", "a"]);
    net.sync(None).unwrap();
}

#[test]
fn connections_past_the_cap_are_closed_at_once() {
    static HUB: Hub = Hub::new();
    let net = net(Ok(vec!["a"]), &HUB);
    let status = net.sync(Some(settings(&["a"], false))).unwrap();
    let mut open: Vec<_> = (0..MAX_CONNS).map(|_| connect(&status)).collect();
    for c in &mut open {
        assert!(ask(c, r#"["x"]"#).contains("remote=true"));
    }
    assert!(closed_unanswered(&mut connect(&status)));
    assert_eq!(net.status().last.unwrap().reason.as_deref(), Some("too many connections"));
    drop(open.pop());
    wait_for("a free slot", || lock(&net.conns).len() < MAX_CONNS);
    assert!(ask(&mut connect(&status), r#"["y"]"#).contains("remote=true"));
    net.sync(None).unwrap();
}

#[test]
fn an_overlong_request_line_ends_the_connection() {
    static HUB: Hub = Hub::new();
    let net = net(Ok(vec!["a"]), &HUB);
    let status = net.sync(Some(settings(&["a"], false))).unwrap();
    let mut c = connect(&status);
    c.get_mut().write_all(&vec![b'x'; MAX_LINE + 10]).unwrap();
    let mut line = String::new();
    c.read_line(&mut line).unwrap();
    assert_eq!(line, "{\"error\":\"request too long\"}\n");
    net.sync(None).unwrap();
}

/// A tailnet that takes `SLOW` to list its addresses.
struct Slow;

const SLOW: Duration = Duration::from_millis(300);

impl Tailnet for Slow {
    fn ips(&self) -> Result<Vec<IpAddr>, String> {
        thread::sleep(SLOW);
        Ok(vec!["127.0.0.1".parse().unwrap()])
    }
    fn whois(&self, _: SocketAddr) -> Result<Vec<String>, String> {
        Ok(vec!["a".into()])
    }
}

#[test]
fn apply_returns_at_once_and_the_latest_apply_wins() {
    static HUB: Hub = Hub::new();
    let net = Net::loopback(Box::new(Slow), handler, &HUB);
    let start = Instant::now();
    assert!(net.apply(Some(settings(&["a"], false))).unwrap().listening.is_empty());
    assert!(start.elapsed() < SLOW);
    let _ = net.status();
    assert!(start.elapsed() < SLOW, "status does not wait for a start");
    wait_for("the listener", || !net.status().listening.is_empty());

    // Of overlapping applies only the last takes effect: here, off.
    net.apply(Some(settings(&["b"], false))).unwrap();
    net.apply(None).unwrap();
    wait_for("the stop", || net.status().listening.is_empty() && lock(&net.run).is_none());
    thread::sleep(SLOW * 2);
    assert_eq!(net.status().listening, []);
    // A superseded apply does nothing.
    let stale = net.wanted.load(Ordering::SeqCst);
    net.wanted.fetch_add(1, Ordering::SeqCst);
    assert_eq!(net.apply_now(stale, Some(settings(&["a"], false))).unwrap().listening, []);
}

#[test]
fn an_address_listed_twice_is_bound_once() {
    static HUB: Hub = Hub::new();
    let twice = Fake { ips: vec!["127.0.0.1", "127.0.0.1"], whois: Ok(vec!["a"]) };
    let net = Net::loopback(Box::new(twice), handler, &HUB);
    let status = net.sync(Some(settings(&["a"], false))).unwrap();
    assert_eq!(status.listening.len(), 1);
    assert_eq!(status.error, None);
    net.sync(None).unwrap();
}

#[test]
fn stopping_frees_every_port_before_it_returns() {
    static HUB: Hub = Hub::new();
    let net = net(Ok(vec!["a"]), &HUB);
    for _ in 0..3 {
        let status = net.sync(Some(settings(&["a"], false))).unwrap();
        let addr = status.listening[0];
        // Restart on the same port, as a reload or a toggle does: no listener of the old
        // run may still hold it.
        let port = NetSettings { port: addr.port(), ..settings(&["a"], false) };
        let again = net.sync(Some(port)).unwrap();
        assert_eq!((again.listening.as_slice(), again.error), ([addr].as_slice(), None));
        net.sync(None).unwrap();
        // No retry here: the port is free the moment the stop returns.
        drop(TcpListener::bind(addr).unwrap());
    }
}

#[test]
fn a_connection_left_in_time_wait_does_not_block_a_restart() {
    static HUB: Hub = Hub::new();
    let net = net(Ok(vec!["a"]), &HUB);
    let status = net.sync(Some(settings(&["a"], false))).unwrap();
    let addr = status.listening[0];
    let mut c = connect(&status);
    assert!(ask(&mut c, r#"["x"]"#).contains("remote=true"));
    // The server closes first, so its end of the connection waits in TIME_WAIT on `addr`.
    net.sync(None).unwrap();
    let mut rest = String::new();
    assert_eq!(c.read_to_string(&mut rest).unwrap_or(0), 0);
    drop(c);
    let port = NetSettings { port: addr.port(), ..settings(&["a"], false) };
    assert_eq!(net.sync(Some(port)).unwrap().listening, [addr]);
    net.sync(None).unwrap();
}

#[test]
fn an_address_in_use_is_named_and_the_others_still_listen() {
    static HUB: Hub = Hub::new();
    let both = Fake { ips: vec!["127.0.0.1", "::1"], whois: Ok(vec!["a"]) };
    let net = Net::loopback(Box::new(both), handler, &HUB);
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let _held = TcpListener::bind(("::1", port)).unwrap();
    let status = net.sync(Some(NetSettings { port, ..settings(&["a"], false) })).unwrap();
    assert_eq!(status.listening, [SocketAddr::from(([127, 0, 0, 1], port))]);
    let e = status.error.unwrap();
    assert!(e.starts_with(&format!("[::1]:{port}: port {port} is in use by ")), "{e}");
    net.sync(None).unwrap();
}
