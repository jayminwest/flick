//! The control protocol over TCP, for peers in the user's tailnet. Off until the remote
//! module calls `HOOKS.apply`. It fails closed at every step:
//!
//! - It binds one listener per Tailscale address of this machine (`is_tailnet`), never a
//!   wildcard or LAN address. No Tailscale address means no listener.
//! - Each connection's peer must be a Tailscale address that `tailscale whois` names as one
//!   of the allowed peers. That check runs before any request is read; anything else closes
//!   the connection.
//! - Every request runs with `Cx::remote` set, whatever the client sent, and must pass
//!   `net_policy`. `["events"]` is refused unless `[remote] events` allows it.
//! - Request lines are capped, an idle connection is closed, and the number of open
//!   connections is capped.
//!
//! Stopping sets a flag, wakes each blocked `accept` with a connection to itself, and shuts
//! down every open network connection.

use std::collections::HashMap;
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::server::{self, Handler, Hub, Limits};
use super::tailscale::{Cached, Cli, Tailnet};
use crate::core::control::{
    Flags, LastConn, NetHooks, NetSettings, NetStatus, Reply, is_tailnet, net_policy, peer_matches,
    split_flags,
};

/// How the remote module drives this process's network transport.
#[expect(
    dead_code,
    reason = "first caller: modules/mod.rs registers the remote module, flick-3537"
)]
pub const HOOKS: NetHooks = NetHooks { apply, status };

/// The longest request line a network caller may send, newline included.
const MAX_LINE: usize = 64 * 1024;
/// How long a network connection may wait between requests.
const IDLE: Duration = Duration::from_secs(30);
/// How long a write to a network caller may block.
const WRITE: Duration = Duration::from_secs(10);
/// Open network connections at most; more are closed at once.
const MAX_CONNS: usize = 16;
/// How long stopping waits to wake one blocked `accept`.
const WAKE: Duration = Duration::from_millis(500);

/// This process's transport: the `tailscale` CLI, the real controller and event hub.
static NET: LazyLock<Net> =
    LazyLock::new(|| Net::new(Box::new(Cached::new(Cli)), net_on_main, &super::HUB));

fn apply(settings: Option<NetSettings>) -> Result<NetStatus, String> {
    NET.apply(settings)
}

fn status() -> NetStatus {
    NET.status()
}

/// Answer a network request on the main thread.
fn net_on_main(words: Vec<String>) -> Reply {
    gate(words, super::run)
}

/// Run a network request through `run`: always as a remote caller, and only if
/// `net_policy` allows it.
fn gate(words: Vec<String>, run: fn(Vec<String>, Flags) -> Reply) -> Reply {
    let (words, flags) = split_flags(words);
    let flags = Flags { remote: true, ..flags };
    match net_policy(&words) {
        Ok(()) => run(words, flags),
        Err(e) => Reply::Error(e),
    }
}

/// Whether the transport may bind to, or serve a peer at, `ip`. Loopback only for tests.
fn permitted(ip: IpAddr, loopback: bool) -> bool {
    is_tailnet(ip) || (loopback && ip.is_loopback())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A network transport: at most one set of listeners at a time.
pub struct Net {
    tailnet: Box<dyn Tailnet>,
    handler: Handler,
    hub: &'static Hub,
    /// Whether loopback addresses count as tailnet ones. Only `Net::loopback` (tests) sets it.
    loopback: bool,
    /// The running listeners. Held while they start or stop.
    run: Mutex<Option<Run>>,
    /// What `status` reports, apart from `last`. Locked only briefly: `status` never waits
    /// for a start or a stop.
    shown: Mutex<NetStatus>,
    last: Mutex<Option<LastConn>>,
    /// The latest `apply` call; an older one that has not run yet is skipped.
    wanted: AtomicU64,
    /// Open network connections, so that stopping can shut them down.
    conns: Mutex<HashMap<u64, TcpStream>>,
    next: AtomicU64,
}

/// Running listeners: the stop flag their threads check and each accept thread.
struct Run {
    stop: Arc<AtomicBool>,
    accepts: Vec<(SocketAddr, JoinHandle<()>)>,
}

impl Net {
    fn new(tailnet: Box<dyn Tailnet>, handler: Handler, hub: &'static Hub) -> Net {
        Net {
            tailnet,
            handler,
            hub,
            loopback: false,
            run: Mutex::new(None),
            shown: Mutex::new(NetStatus::default()),
            last: Mutex::new(None),
            wanted: AtomicU64::new(0),
            conns: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
        }
    }

    /// A transport that also admits loopback addresses, for tests with a fake tailnet.
    #[cfg(test)]
    fn loopback(tailnet: Box<dyn Tailnet>, handler: Handler, hub: &'static Hub) -> &'static Net {
        Box::leak(Box::new(Net { loopback: true, ..Net::new(tailnet, handler, hub) }))
    }

    /// The transport's state for `remote status`. Never blocks on a start or a stop.
    pub fn status(&self) -> NetStatus {
        let status = lock(&self.shown).clone();
        NetStatus { last: lock(&self.last).clone(), ..status }
    }

    /// Stop any running listeners and close their connections; then, for `Some`, listen
    /// with `settings`. Runs on a background thread, since asking Tailscale can take
    /// seconds and the caller is the main thread; returns the status at once, and `status`
    /// shows the outcome. When applies overlap, the latest wins.
    pub fn apply(&'static self, settings: Option<NetSettings>) -> Result<NetStatus, String> {
        let wanted = self.wanted.fetch_add(1, Ordering::SeqCst) + 1;
        thread::Builder::new()
            .name("flick-net-apply".into())
            .spawn(move || self.apply_now(wanted, settings))
            .map_err(|e| e.to_string())?;
        Ok(self.status())
    }

    /// `apply` on this thread, unless a later `apply` (`wanted`) has come since. `Err` (no
    /// peers, no Tailscale, no bind) leaves it stopped, with the error in `status`.
    fn apply_now(
        &'static self,
        wanted: u64,
        settings: Option<NetSettings>,
    ) -> Result<NetStatus, String> {
        let mut run = lock(&self.run);
        if self.wanted.load(Ordering::SeqCst) != wanted {
            return Ok(self.status());
        }
        if let Some(old) = run.take() {
            self.stop(old);
        }
        *lock(&self.shown) = NetStatus::default();
        let started = settings.map(|settings| self.start(settings)).transpose();
        let shown = match started {
            Ok(started) => started.map_or_else(NetStatus::default, |(new, status)| {
                *run = Some(new);
                status
            }),
            Err(e) => NetStatus { error: Some(e), ..NetStatus::default() },
        };
        *lock(&self.shown) = shown;
        drop(run);
        let status = self.status();
        status.error.clone().filter(|_| status.listening.is_empty()).map_or(Ok(status), Err)
    }

    fn stop(&self, run: Run) {
        run.stop.store(true, Ordering::SeqCst);
        for (addr, accept) in run.accepts {
            // A thread whose address no longer answers is left blocked; it sees the flag
            // and closes whatever it accepts next.
            if TcpStream::connect_timeout(&addr, WAKE).is_ok() {
                let _ = accept.join();
            }
        }
        for (_, stream) in lock(&self.conns).drain() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    fn start(&'static self, settings: NetSettings) -> Result<(Run, NetStatus), String> {
        if settings.peers.is_empty() {
            return Err("no peers allowed: set [remote] peers".into());
        }
        let (listeners, error) = self.bind(&self.tailnet.ips()?, settings.port)?;
        let stop = Arc::new(AtomicBool::new(false));
        let settings = Arc::new(settings);
        let mut accepts = Vec::new();
        for listener in listeners {
            let addr = listener.local_addr().map_err(|e| e.to_string())?;
            let (stop, settings) = (Arc::clone(&stop), Arc::clone(&settings));
            let accept = thread::Builder::new()
                .name("flick-net".into())
                .spawn(move || self.accept(&listener, &settings, &stop))
                .map_err(|e| e.to_string())?;
            accepts.push((addr, accept));
        }
        let listening = accepts.iter().map(|(addr, _)| *addr).collect();
        let status = NetStatus { listening, peers: settings.peers.clone(), last: None, error };
        Ok((Run { stop, accepts }, status))
    }

    /// One listener per permitted address in `ips` on `port`. `Err` when none binds; else
    /// the listeners and the errors of addresses that did not bind.
    fn bind(
        &self,
        ips: &[IpAddr],
        port: u16,
    ) -> Result<(Vec<TcpListener>, Option<String>), String> {
        let ips: Vec<IpAddr> =
            ips.iter().copied().filter(|ip| permitted(*ip, self.loopback)).collect();
        if ips.is_empty() {
            return Err("no Tailscale address to listen on (is Tailscale up?)".into());
        }
        let mut listeners = Vec::new();
        let mut errors = Vec::new();
        for ip in ips {
            let addr = SocketAddr::new(ip, port);
            match TcpListener::bind(addr) {
                Ok(listener) => listeners.push(listener),
                Err(e) => errors.push(format!("{addr}: {e}")),
            }
        }
        let errors = (!errors.is_empty()).then(|| errors.join("; "));
        if listeners.is_empty() {
            return Err(errors.unwrap_or_default());
        }
        Ok((listeners, errors))
    }

    fn accept(
        &'static self,
        listener: &TcpListener,
        settings: &Arc<NetSettings>,
        stop: &Arc<AtomicBool>,
    ) {
        for stream in listener.incoming() {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            match stream {
                Ok(stream) => self.admit(stream, settings, stop),
                Err(e) => eprintln!("flick: network control: {e}"),
            }
        }
    }

    /// Track `stream` and check and serve it on its own thread, unless too many are open.
    fn admit(
        &'static self,
        stream: TcpStream,
        settings: &Arc<NetSettings>,
        stop: &Arc<AtomicBool>,
    ) {
        let Ok(peer) = stream.peer_addr() else { return };
        let id = {
            let mut conns = lock(&self.conns);
            if conns.len() >= MAX_CONNS {
                drop(conns);
                self.record(peer.ip(), None, Err("too many connections".into()));
                return;
            }
            let Ok(tracked) = stream.try_clone() else { return };
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            conns.insert(id, tracked);
            id
        };
        let (settings, stop) = (Arc::clone(settings), Arc::clone(stop));
        let spawned = thread::Builder::new().name("flick-net-client".into()).spawn(move || {
            self.serve(stream, peer, &settings, &stop);
            lock(&self.conns).remove(&id);
        });
        if spawned.is_err()
            && let Some(stream) = lock(&self.conns).remove(&id)
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    /// Serve `stream` if its peer is allowed; else close it unread.
    fn serve(
        &self,
        stream: TcpStream,
        peer: SocketAddr,
        settings: &NetSettings,
        stop: &AtomicBool,
    ) {
        if self.check(peer, &settings.peers).is_err() || stop.load(Ordering::SeqCst) {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        let limits = Limits { line: MAX_LINE, idle: Some(IDLE), events: settings.events };
        if stream.set_write_timeout(Some(WRITE)).is_ok() {
            let _ = server::connection(stream, self.handler, self.hub, limits);
        }
    }

    /// Whether the peer at `peer` is one of `peers`, by its Tailscale address and whois
    /// names. Records the verdict for `status`.
    fn check(&self, peer: SocketAddr, peers: &[String]) -> Result<(), String> {
        let (name, verdict) = if permitted(peer.ip(), self.loopback) {
            match self.tailnet.whois(peer) {
                Ok(names) if peer_matches(&names, peers) => (names.first().cloned(), Ok(())),
                Ok(names) => {
                    let name = names.first().cloned();
                    let shown = name.as_deref().unwrap_or("peer");
                    (name.clone(), Err(format!("{shown} is not in [remote] peers")))
                }
                Err(e) => (None, Err(e)),
            }
        } else {
            (None, Err("not a Tailscale address".into()))
        };
        self.record(peer.ip(), name, verdict.clone());
        verdict
    }

    fn record(&self, ip: IpAddr, name: Option<String>, verdict: Result<(), String>) {
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let at = i64::try_from(at).unwrap_or(i64::MAX);
        let (allowed, reason) = (verdict.is_ok(), verdict.err());
        *lock(&self.last) = Some(LastConn { name, ip, at, allowed, reason });
    }
}

#[cfg(test)]
mod tests;
