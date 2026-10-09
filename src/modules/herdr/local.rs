//! The local herdr server over its Unix socket (`~/.config/herdr/herdr.sock`, mode 0600).
//!
//! Wire: one JSON request line `{"id","method","params"}`, one reply line `{"id","result"}`
//! or `{"id","error":{"code","message"}}`. herdr 0.9.1 serves one request per connection
//! and then closes it, so every call opens its own connection.
//!
//! `events.subscribe` keeps its connection open: the reply `subscription_started`, then one
//! event envelope per line. A second `events.subscribe` on that connection makes the
//! server drop it without a reply (spike, flick-4115), so new panes need a new
//! subscription, and the old one is dropped after the new one starts. Status changes come
//! only from per-pane `pane.agent_status_changed` subscriptions; the global pane events
//! tell the fleet a pane appeared or went away (`Applied::relist`).
//!
//! Nothing here retries. A missing server is an error; the module connects again on the
//! next launcher open, view open or wake. `Subscription::next` blocks in `read` with no
//! timeout (idle CPU 0, mulch mx-bd9f16); `Closer` ends it from another thread.

use super::model::{self, Agent};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The protocol herdr 0.9.1 reports in `ping`. Another one still works when the methods
/// used here kept their shape; `Pong::mismatch` says so for the log.
pub const PROTOCOL: u32 = 22;

/// Global subscriptions: panes appearing, changing and going away.
pub const GLOBAL: [&str; 5] =
    ["pane.created", "pane.updated", "pane.closed", "pane.exited", "pane.agent_detected"];

/// How long a request waits for its reply by default. Subscriptions have no read timeout.
const TIMEOUT: Duration = Duration::from_secs(5);

/// How many times `sync` subscribes again when panes appear while it subscribes.
const SYNC_TRIES: usize = 3;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// `path` with a leading `~/` replaced by the home directory.
pub fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// herdr's default server socket.
pub fn default_socket() -> PathBuf {
    expand("~/.config/herdr/herdr.sock")
}

/// A `ping` reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pong {
    pub version: String,
    pub protocol: u32,
}

impl Pong {
    /// A log line when the server speaks another protocol than `PROTOCOL`.
    pub fn mismatch(&self) -> Option<String> {
        (self.protocol != PROTOCOL).then(|| {
            format!(
                "herdr {} speaks protocol {}, Flick expects {PROTOCOL}; trying anyway",
                self.version, self.protocol
            )
        })
    }
}

/// The `events.subscribe` params: `GLOBAL` plus a status subscription per agent pane.
pub fn subscriptions(panes: &[&str]) -> Value {
    let global = GLOBAL.iter().map(|t| json!({ "type": t }));
    let status = panes.iter().map(|p| json!({ "type": "pane.agent_status_changed", "pane_id": p }));
    json!({ "subscriptions": global.chain(status).collect::<Vec<_>>() })
}

/// A client for one server socket. Cheap; holds no connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    socket: PathBuf,
    timeout: Duration,
}

impl Local {
    pub fn new(socket: impl Into<PathBuf>) -> Local {
        Local { socket: socket.into(), timeout: TIMEOUT }
    }

    /// The same client with requests timing out after `timeout`.
    pub fn with_timeout(self, timeout: Duration) -> Local {
        Local { timeout, ..self }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Run `method` and return its `result`, or the server's error message.
    pub fn call(&self, method: &str, params: &Value) -> Result<Value, String> {
        let (_, mut reader) = self.send(method, params, Some(self.timeout))?;
        read_reply(&mut reader)
    }

    pub fn ping(&self) -> Result<Pong, String> {
        let r = self.call("ping", &json!({}))?;
        let version = r.get("version").and_then(Value::as_str).unwrap_or("?").to_string();
        let protocol = r.get("protocol").and_then(Value::as_u64).ok_or("bad ping reply")?;
        Ok(Pong { version, protocol: u32::try_from(protocol).unwrap_or(u32::MAX) })
    }

    /// Every agent, tagged with `machine`.
    pub fn list(&self, machine: &str) -> Result<Vec<Agent>, String> {
        model::parse_agents(machine, &self.call("agent.list", &json!({}))?)
    }

    /// The last `lines` lines of `target`'s output, escapes stripped by herdr.
    pub fn read(&self, target: &str, lines: u32) -> Result<String, String> {
        let params = json!({ "target": target, "source": "recent_unwrapped", "lines": lines });
        model::parse_read(&self.call("agent.read", &params)?)
    }

    /// Focus `target`'s pane in the herdr client.
    pub fn focus(&self, target: &str) -> Result<(), String> {
        self.call("agent.focus", &json!({ "target": target })).map(drop)
    }

    /// A subscription to `GLOBAL` and to the status of each pane in `panes`.
    pub fn subscribe(&self, panes: &[&str]) -> Result<Subscription, String> {
        let (stream, mut reader) = self.send("events.subscribe", &subscriptions(panes), None)?;
        match read_reply(&mut reader)?.get("type").and_then(Value::as_str) {
            Some("subscription_started") => Ok(Subscription { stream, reader }),
            other => Err(format!("events.subscribe: unexpected reply {other:?}")),
        }
    }

    /// List, subscribe to the listed panes, then list again, so a status change between
    /// the list and the subscription is not lost. When the second list has a pane the
    /// subscription lacks, subscribe again (up to `SYNC_TRIES` times).
    pub fn sync(&self, machine: &str) -> Result<(Vec<Agent>, Subscription), String> {
        let mut agents = self.list(machine)?;
        let mut tries = 0;
        loop {
            let panes: Vec<&str> = agents.iter().map(|a| a.pane_id.as_str()).collect();
            let sub = self.subscribe(&panes)?;
            let now = self.list(machine)?;
            tries += 1;
            if tries == SYNC_TRIES || now.iter().all(|a| panes.contains(&a.pane_id.as_str())) {
                return Ok((now, sub));
            }
            agents = now;
        }
    }

    /// Connect and write one request; the reader is for the reply.
    fn send(
        &self,
        method: &str,
        params: &Value,
        timeout: Option<Duration>,
    ) -> Result<(UnixStream, BufReader<UnixStream>), String> {
        let mut stream = UnixStream::connect(&self.socket).map_err(|e| match e.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => "herdr not running".to_string(),
            _ => format!("herdr socket {}: {e}", self.socket.display()),
        })?;
        let io = |e: std::io::Error| format!("herdr {method}: {e}");
        stream.set_read_timeout(timeout).map_err(io)?;
        stream.set_write_timeout(Some(self.timeout)).map_err(io)?;
        let id = format!("flick:{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let line = json!({ "id": id, "method": method, "params": params }).to_string();
        stream.write_all(format!("{line}\n").as_bytes()).map_err(io)?;
        let reader = BufReader::new(stream.try_clone().map_err(io)?);
        Ok((stream, reader))
    }
}

/// One reply line's `result`, or its error.
fn read_reply(reader: &mut impl BufRead) -> Result<Value, String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => return Err("herdr closed the connection".into()),
        Ok(_) => {}
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            return Err("herdr did not answer".into());
        }
        Err(e) => return Err(format!("herdr: {e}")),
    }
    let reply: Value = serde_json::from_str(&line).map_err(|e| format!("bad herdr reply: {e}"))?;
    model::reply_result(&reply).cloned()
}

/// A live event stream. Dropping it closes the connection.
#[derive(Debug)]
pub struct Subscription {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Subscription {
    /// The next event envelope `{"event","data"}`; blocks until one arrives. `None` when
    /// the server goes away or `Closer::close` ran. Lines that are not JSON are skipped.
    pub fn next(&mut self) -> Option<Value> {
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
            if let Ok(event) = serde_json::from_str::<Value>(&line)
                && event.get("event").is_some()
            {
                return Some(event);
            }
        }
    }

    /// A handle that ends this subscription from another thread.
    pub fn closer(&self) -> Result<Closer, String> {
        self.stream.try_clone().map(Closer).map_err(|e| format!("herdr subscription: {e}"))
    }
}

/// Ends a `Subscription`: its blocked `next` returns `None`.
#[derive(Debug)]
pub struct Closer(UnixStream);

impl Closer {
    pub fn close(&self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests;
