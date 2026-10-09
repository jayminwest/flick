//! A fake herdr server on a short socket path (macOS caps them at 104 bytes, mulch
//! mx-6c21fe). Like herdr 0.9.1 it answers one request per connection and closes it,
//! except a subscription, which it holds open. Replies are synthesized from the schema.

use super::*;
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};
use std::thread;

/// What the fake writes for one request; `hold` keeps the connection open until the
/// client closes it.
struct Reply {
    lines: Vec<String>,
    hold: bool,
}

fn ok(result: &Value) -> Reply {
    Reply { lines: vec![json!({ "id": "x", "result": result }).to_string()], hold: false }
}

struct Fake {
    dir: PathBuf,
    local: Local,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fake(name: &str, answer: impl Fn(&Value, usize) -> Reply + Send + 'static) -> Fake {
    let dir = PathBuf::from(format!("/tmp/fk.{}.{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("h.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let requests = Arc::new(Mutex::new(vec![]));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        for (n, stream) in listener.incoming().enumerate() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let reply = answer(&request, n);
            seen.lock().unwrap().push(request);
            for l in reply.lines {
                stream.write_all(format!("{l}\n").as_bytes()).unwrap();
            }
            if reply.hold {
                // Until the client hangs up.
                let _ = stream.read(&mut [0; 64]);
            }
        }
    });
    Fake { dir, local: Local::new(socket), requests }
}

fn agents(panes: &[&str]) -> Value {
    let agent = |p: &&str| {
        json!({ "pane_id": p, "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t1",
                "agent": "claude", "agent_status": "idle", "focused": false, "revision": 1 })
    };
    json!({ "type": "agent_list", "agents": panes.iter().map(agent).collect::<Vec<_>>() })
}

fn started() -> String {
    json!({ "id": "x", "result": { "type": "subscription_started" } }).to_string()
}

fn status(pane: &str, status: &str) -> String {
    json!({ "event": "pane_agent_status_changed",
            "data": { "pane_id": pane, "workspace_id": "w1", "agent_status": status } })
    .to_string()
}

#[test]
fn ping_list_read_and_focus_send_one_request_per_connection() {
    let f = fake("calls", |req, _| match req["method"].as_str().unwrap() {
        "ping" => ok(&json!({ "type": "pong", "version": "0.9.1", "protocol": 22 })),
        "agent.list" => ok(&agents(&["w1:p1", "w2:p1"])),
        "agent.read" => ok(&json!({ "type": "pane_read", "read": { "text": "a\nb\n" } })),
        _ => ok(&json!({ "type": "ok" })),
    });
    let pong = f.local.ping().unwrap();
    assert_eq!(pong, Pong { version: "0.9.1".into(), protocol: 22 });
    assert_eq!(pong.mismatch(), None);
    let listed = f.local.list("local").unwrap();
    let ids: Vec<_> = listed.iter().map(|a| (a.machine.as_str(), a.pane_id.as_str())).collect();
    assert_eq!(ids, [("local", "w1:p1"), ("local", "w2:p1")]);
    assert_eq!(f.local.read("w1:p1", 40).unwrap(), "a\nb\n");
    f.local.focus("w2:p1").unwrap();

    let requests = f.requests.lock().unwrap();
    let methods: Vec<_> = requests.iter().map(|r| r["method"].as_str().unwrap()).collect();
    assert_eq!(methods, ["ping", "agent.list", "agent.read", "agent.focus"]);
    assert_eq!(
        requests[2]["params"],
        json!({ "target": "w1:p1", "source": "recent_unwrapped", "lines": 40 })
    );
    assert_eq!(requests[3]["params"], json!({ "target": "w2:p1" }));
    let ids: Vec<_> = requests.iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
    assert!(ids.iter().all(|id| id.starts_with("flick:")), "{ids:?}");
    assert_eq!(ids.iter().collect::<std::collections::BTreeSet<_>>().len(), ids.len());
}

#[test]
fn errors_name_the_cause() {
    let f = fake("errors", |req, _| match req["method"].as_str().unwrap() {
        "agent.focus" => Reply {
            lines: vec![
                json!({ "id": "x", "error": { "code": "agent_not_found", "message": "no agent" } })
                    .to_string(),
            ],
            hold: false,
        },
        "agent.read" => Reply { lines: vec!["not json".into()], hold: false },
        "agent.list" => Reply { lines: vec![], hold: false },
        "ping" => ok(&json!({ "type": "pong", "version": "9.9", "protocol": 23 })),
        _ => ok(&json!({ "type": "ok" })),
    });
    assert_eq!(f.local.focus("w9:p9").unwrap_err(), "no agent");
    assert!(f.local.read("w1:p1", 1).unwrap_err().starts_with("bad herdr reply"));
    assert_eq!(f.local.list("local").unwrap_err(), "herdr closed the connection");
    assert_eq!(
        f.local.subscribe(&[]).unwrap_err(),
        "events.subscribe: unexpected reply Some(\"ok\")"
    );
    let pong = f.local.ping().unwrap();
    assert!(pong.mismatch().unwrap().contains("protocol 23, Flick expects 22"));
    assert!(f.local.call("pane.get", &json!({})).is_ok());

    let gone = Local::new(f.dir.join("none.sock"));
    assert_eq!(gone.ping().unwrap_err(), "herdr not running");
    assert_eq!(gone.socket(), f.dir.join("none.sock"));
    // A path that is not a socket.
    let file = f.dir.join("file");
    std::fs::write(&file, "").unwrap();
    assert!(Local::new(&file).ping().unwrap_err().starts_with("herdr"));
}

#[test]
fn a_bad_ping_reply_is_an_error() {
    let f = fake("badping", |_, _| ok(&json!({ "type": "pong" })));
    assert_eq!(f.local.ping().unwrap_err(), "bad ping reply");
}

#[test]
fn a_silent_server_times_out() {
    let f = fake("silent", |_, _| Reply { lines: vec![], hold: true });
    let local = f.local.clone().with_timeout(Duration::from_millis(50));
    let err = local.ping().unwrap_err();
    assert_eq!(err, "herdr did not answer");
}

#[test]
fn subscribe_asks_for_global_events_and_each_pane_status() {
    let p = subscriptions(&["w1:p1"]);
    let types: Vec<_> =
        p["subscriptions"].as_array().unwrap().iter().map(|s| s["type"].clone()).collect();
    assert_eq!(types.len(), GLOBAL.len() + 1);
    assert_eq!(
        p["subscriptions"][GLOBAL.len()],
        json!({ "type": "pane.agent_status_changed", "pane_id": "w1:p1" })
    );
    assert_eq!(p["subscriptions"][0], json!({ "type": "pane.created" }));
}

#[test]
fn a_subscription_yields_events_until_the_server_closes_it() {
    let f = fake("stream", |_, _| Reply {
        lines: vec![
            started(),
            status("w1:p1", "blocked"),
            "garbage".into(),
            "{}".into(),
            status("w1:p1", "done"),
        ],
        hold: false,
    });
    let mut sub = f.local.subscribe(&["w1:p1"]).unwrap();
    let event = sub.next().unwrap();
    let change = model::parse_event("local", &event).unwrap();
    assert!(matches!(change, model::Change::Status { status: model::Status::Blocked, .. }));
    assert_eq!(sub.next().unwrap()["data"]["agent_status"], "done");
    assert!(sub.next().is_none());
    let sent = &f.requests.lock().unwrap()[0];
    assert_eq!(sent["method"], "events.subscribe");
    assert_eq!(sent["params"], subscriptions(&["w1:p1"]));
}

#[test]
fn a_closer_ends_a_blocked_subscription_from_another_thread() {
    let f = fake("close", |_, _| Reply { lines: vec![started()], hold: true });
    let mut sub = f.local.subscribe(&[]).unwrap();
    let closer = sub.closer().unwrap();
    let reader = thread::spawn(move || sub.next());
    thread::sleep(Duration::from_millis(50));
    closer.close();
    assert!(reader.join().unwrap().is_none());
}

#[test]
fn sync_subscribes_again_when_panes_appear_meanwhile() {
    // Lists grow by one pane per call: the first subscription misses w2, the second sees all.
    let f = fake("sync", |req, n| match req["method"].as_str().unwrap() {
        "agent.list" if n == 0 => ok(&agents(&["w1:p1"])),
        "agent.list" => ok(&agents(&["w1:p1", "w2:p1"])),
        _ => Reply { lines: vec![started()], hold: false },
    });
    let (listed, _sub) = f.local.sync("local").unwrap();
    assert_eq!(listed.len(), 2);
    let requests = f.requests.lock().unwrap();
    let methods: Vec<_> = requests.iter().map(|r| r["method"].as_str().unwrap()).collect();
    assert_eq!(
        methods,
        ["agent.list", "events.subscribe", "agent.list", "events.subscribe", "agent.list"]
    );
    assert_eq!(requests[3]["params"], subscriptions(&["w1:p1", "w2:p1"]));
}

#[test]
fn sync_gives_up_subscribing_again_after_a_few_tries() {
    let f = fake("churn", |req, n| match req["method"].as_str().unwrap() {
        "agent.list" => ok(&agents(&[&format!("w{n}:p1")])),
        _ => Reply { lines: vec![started()], hold: false },
    });
    let (listed, _sub) = f.local.sync("local").unwrap();
    assert_eq!(listed.len(), 1);
    let subscribes =
        f.requests.lock().unwrap().iter().filter(|r| r["method"] == "events.subscribe").count();
    assert_eq!(subscribes, SYNC_TRIES);
}

#[test]
fn paths_expand_home() {
    let home = dirs::home_dir().unwrap();
    assert_eq!(expand("~/x/y.sock"), home.join("x/y.sock"));
    assert_eq!(expand("/abs.sock"), PathBuf::from("/abs.sock"));
    assert_eq!(default_socket(), home.join(".config/herdr/herdr.sock"));
}
