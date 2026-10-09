//! Fakes for the module's thread tests: a herdr server on a short socket path (macOS caps
//! them at 104 bytes, mulch mx-6c21fe) and a `herdr` CLI shell script. Like herdr 0.9.1
//! the server answers one request per connection, except `events.subscribe`, which it
//! holds open for `push`. Replies are synthesized from the schema; none hold real output.

use super::io::Hooks;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const HOOKS: Hooks = Hooks {
    post: || {},
    now: || 1_000,
    visible: || false,
    front: |_| {},
    is_front: |_| false,
    notify: |id, title, body| NOTES.with_borrow_mut(|n| n.push(format!("{id} | {title} | {body}"))),
    listen: |ask| LISTENS.with_borrow_mut(|l| l.push(ask)),
    clicks: || CLICKS.with_borrow_mut(std::mem::take),
    notifications: || "test".into(),
    copy: |text| COPIED.with_borrow_mut(|c| c.push(text.to_string())),
};

thread_local! {
    /// Notifications "posted" on this thread, as `id | title | body`. Never a real one.
    pub static NOTES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// `listen` calls on this thread: whether each asked for permission.
    pub static LISTENS: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    /// Clicked notification ids the next `ModuleChanged` on this thread reads.
    pub static CLICKS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Text "copied" on this thread. Never the real pasteboard.
    pub static COPIED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Wait up to 5 s for `cond`.
pub fn wait(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// An `AgentInfo` in a list reply.
pub fn info(pane: &str, status: &str) -> Value {
    json!({ "pane_id": pane, "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t1",
            "agent": "claude", "name": format!("n-{pane}"), "agent_status": status,
            "cwd": "/Users/example/src/api", "focused": false, "revision": 1, "state_change_seq": 1 })
}

pub fn status_event(pane: &str, status: &str) -> Value {
    json!({ "event": "pane_agent_status_changed",
            "data": { "pane_id": pane, "workspace_id": "w1", "agent_status": status } })
}

pub struct Server {
    pub dir: PathBuf,
    pub socket: PathBuf,
    /// The `agents` of the next `agent.list` reply.
    pub agents: Arc<Mutex<Vec<Value>>>,
    /// Methods in arrival order, with their `target` param when they have one.
    pub calls: Arc<Mutex<Vec<String>>>,
    subs: Arc<Mutex<Vec<UnixStream>>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.hang_up();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Server {
    pub fn start(name: &str, agents: Vec<Value>) -> Server {
        let dir = PathBuf::from(format!("/tmp/fk.{}.{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("h.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let agents = Arc::new(Mutex::new(agents));
        let calls = Arc::new(Mutex::new(vec![]));
        let subs = Arc::new(Mutex::new(vec![]));
        let (a, c, s) = (Arc::clone(&agents), Arc::clone(&calls), Arc::clone(&subs));
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let (a, c, s) = (Arc::clone(&a), Arc::clone(&c), Arc::clone(&s));
                thread::spawn(move || serve(stream, &a, &c, &s));
            }
        });
        Server { dir, socket, agents, calls, subs }
    }

    pub fn subscribers(&self) -> usize {
        self.subs.lock().unwrap().len()
    }

    pub fn push(&self, event: &Value) {
        for s in self.subs.lock().unwrap().iter_mut() {
            let _ = s.write_all(format!("{event}\n").as_bytes());
        }
    }

    /// Close every subscription, as a quitting server does.
    pub fn hang_up(&self) {
        for s in self.subs.lock().unwrap().drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

fn serve(
    mut stream: UnixStream,
    agents: &Mutex<Vec<Value>>,
    calls: &Mutex<Vec<String>>,
    subs: &Mutex<Vec<UnixStream>>,
) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let Ok(req) = serde_json::from_str::<Value>(&line) else { return };
    let method = req["method"].as_str().unwrap_or("").to_string();
    let target = req["params"]["target"].as_str().map(|t| format!(" {t}")).unwrap_or_default();
    calls.lock().unwrap().push(format!("{method}{target}"));
    let result = match method.as_str() {
        "ping" => json!({ "type": "pong", "version": "0.9.1", "protocol": 22 }),
        "agent.list" => json!({ "type": "agent_list", "agents": *agents.lock().unwrap() }),
        "agent.read" => json!({ "type": "pane_read", "read": { "text": "one\n\ntwo\n" } }),
        "events.subscribe" => json!({ "type": "subscription_started" }),
        _ => json!({ "type": "ok" }),
    };
    let reply = json!({ "id": req["id"], "result": result });
    let _ = stream.write_all(format!("{reply}\n").as_bytes());
    if method == "events.subscribe"
        && let Ok(clone) = stream.try_clone()
    {
        subs.lock().unwrap().push(clone);
    }
}

/// A temp dir with an executable `herdr` script running `body`; each call appends its
/// arguments to `args.log`.
pub struct Cli {
    pub dir: PathBuf,
    pub herdr: PathBuf,
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Cli {
    pub fn new(name: &str, body: &str) -> Cli {
        let dir = std::env::temp_dir().join(format!("flk-{}-herdr-mod-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let herdr = dir.join("herdr");
        let log = dir.join("args.log");
        std::fs::write(&herdr, format!("#!/bin/sh\necho \"$*\" >> '{}'\n{body}\n", log.display()))
            .unwrap();
        std::fs::set_permissions(&herdr, std::fs::Permissions::from_mode(0o755)).unwrap();
        Cli { dir, herdr }
    }

    pub fn args(&self) -> Vec<String> {
        let log = std::fs::read_to_string(self.dir.join("args.log")).unwrap_or_default();
        log.lines().map(str::to_string).collect()
    }
}

/// A CLI where `hub` has one blocked agent, `off` is unreachable, and `machine list` names
/// `hub` and `off`.
pub const CLI: &str = r#"
case "$*" in
  "--machine hub agent list") echo '{"id":"c","result":{"type":"agent_list","agents":[{"pane_id":"w1:p1","workspace_id":"w1","agent":"claude","name":"hub-agent","agent_status":"blocked","state_change_seq":3}]}}' ;;
  "--machine off agent list") echo '{"id":"c","error":{"code":"x","message":"unreachable"}}' >&2; exit 1 ;;
  "--machine hub agent read"*) printf 'remote one\nremote two\n' ;;
  "--machine hub agent focus"*) echo '{"id":"c","result":{"type":"ok"}}' ;;
  "--machine off agent focus"*) echo '{"id":"c","error":{"code":"x","message":"unreachable"}}' >&2; exit 1 ;;
  "machine list --json") echo '[{"id":"a","label":"hub","enabled":true},{"id":"b","label":"off","enabled":true},{"id":"c","label":"no","enabled":false}]' ;;
esac
"#;
