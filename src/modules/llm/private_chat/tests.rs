//! The private chat through the module, over fake curls and a fake window (`testkit`): no
//! window opens and no server is reached. The privacy checks: a private exchange writes no
//! row to flick.db, no command, item or event carries its text, and every wipe empties it.

use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::config::parse;
use crate::core::store::Store;
use crate::core::{Cx, Event, ItemId, Module, Outcome, Ranker};
use crate::modules::llm::settings::NO_PRIVATE;
use crate::modules::llm::testkit::{self, HOOKS, PRIVATE, UI, closed_private, queue, queue_private, sent, take_log};
use crate::modules::llm::{ID, store::MIGRATIONS};

const MLX: &str = "[[llm.servers]]\nname = \"mlx\"\nurl = \"http://mlx\"\n";
/// A private server the fake answers like `mlx` (the port is ignored).
const VAULT: &str = "[[llm.servers]]\nname = \"vault\"\nurl = \"http://mlx:11235\"\nprivate = true\n";
const QWEN: &str = "qwen3-30b-a3b-4bit";

struct Fx {
    store: Store,
    ranker: Ranker,
}

impl Fx {
    fn new() -> Fx {
        let store = Store::in_memory();
        store.migrate(ID, MIGRATIONS).unwrap();
        Fx { store, ranker: Ranker::new() }
    }

    fn cx<R>(&mut self, f: impl FnOnce(&mut Cx) -> R) -> R {
        f(&mut Cx { query: "", store: &self.store, ranker: &mut self.ranker, hide: || {}, json: false, remote: false })
    }

    /// Every row of every table, one string per row.
    fn dump(&self) -> Vec<String> {
        let conn = self.store.conn();
        let mut names = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap();
        let tables: Vec<String> = names.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        let mut rows = vec![];
        for t in tables {
            let mut q = conn.prepare(&format!("SELECT * FROM \"{t}\"")).unwrap();
            let n = q.column_count();
            let got = q.query_map([], |r| Ok((0..n).map(|i| format!("{:?}", r.get_ref(i).unwrap())).collect::<Vec<_>>().join("|")));
            rows.extend(got.unwrap().map(|r| format!("{t}: {}", r.unwrap())));
        }
        rows
    }
}

fn llm(text: &str) -> Llm {
    let mut m = Llm::with_hooks(HOOKS, UI, PRIVATE);
    m.configure(&parse(text).unwrap().section(ID).unwrap().unwrap()).unwrap();
    take_log();
    m
}

fn reconfigure(m: &mut Llm, text: &str) {
    m.configure(&parse(text).unwrap().section(ID).unwrap().unwrap()).unwrap();
}

fn event(f: &mut Fx, m: &mut Llm, e: Event) -> bool {
    f.cx(|cx| m.on_event(e, cx))
}

fn changed(f: &mut Fx, m: &mut Llm) {
    event(f, m, Event::ModuleChanged { module: ID });
}

fn busy(m: &Llm) -> bool {
    lock(&m.private.room).as_ref().is_some_and(|o| o.session.streaming().is_some() || o.waiting.is_some())
}

fn model(m: &Llm) -> String {
    lock(&m.private.room).as_ref().map(|o| o.session.model().to_string()).unwrap_or_default()
}

/// Drain until `done` holds (at most 15 s).
fn until(f: &mut Fx, m: &mut Llm, done: impl Fn(&Llm) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        changed(f, m);
        if done(m) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

fn send(f: &mut Fx, m: &mut Llm, text: &str) {
    queue_private(Note::Submit(text.into()));
    until(f, m, |m| !busy(m));
}

fn hotkey(f: &mut Fx, m: &mut Llm) -> Option<crate::core::ListView> {
    f.cx(|cx| m.hotkey("private", cx))
}

/// Open the private window and wait for the server's model list.
fn opened(f: &mut Fx, m: &mut Llm) {
    assert!(hotkey(f, m).is_none());
    until(f, m, |m| !model(m).is_empty());
}

fn only(kind: &str, log: &[String]) -> Vec<String> {
    log.iter().filter_map(|l| l.strip_prefix(kind).map(|r| r.trim_start().to_string())).collect()
}

fn last(kind: &str) -> String {
    only(kind, &take_log()).pop().unwrap_or_default()
}

fn cmd(c: char) -> Keystroke {
    Keystroke { key: Key::Char(c), cmd: true, shift: false, opt: false }
}

#[test]
fn the_private_window_adds_copy_to_the_chat_keys() {
    assert_eq!(binding(cmd('c')), Some(Command::Copy));
    assert_eq!(binding(cmd('n')), Some(Command::New));
    assert_eq!(binding(cmd('.')), Some(Command::Stop));
    assert_eq!(binding(cmd('w')), Some(Command::Close));
    assert_eq!(binding(Keystroke { shift: true, ..cmd('c') }), None);
    assert_eq!(binding(Keystroke { opt: true, ..cmd('c') }), None);
    assert_eq!(binding(Keystroke { cmd: false, ..cmd('c') }), None);
}

#[test]
fn without_a_private_server_the_item_and_hotkey_refuse_and_never_fall_back() {
    let mut f = Fx::new();
    let mut m = llm(&format!("[llm]\nprivate_hotkey = \"cmd+KeyP\"\n{MLX}"));
    let items = f.cx(|cx| m.items(cx));
    let private = items.iter().find(|i| i.id.as_str() == "llm:private").unwrap();
    assert_eq!(private.subtitle, "No private server ([[llm.servers]] private = true)");
    let item = private.id.clone();
    assert!(matches!(f.cx(|cx| m.activate(&item, cx)), Outcome::Stay(Some(e)) if e == NO_PRIVATE));
    // The hotkey is bound and shows the launcher saying why.
    let keys: Vec<(String, String)> = m.hotkeys().into_iter().map(|b| (b.spec, b.key.unwrap())).collect();
    assert_eq!(keys, [("cmd+KeyP".to_string(), "private".to_string())]);
    let view = hotkey(&mut f, &mut m).unwrap();
    assert_eq!((view.name.as_str(), view.empty.as_str(), view.escape_hides), ("private", NO_PRIVATE, true));
    let mut view = f.cx(|cx| m.open("private", cx)).unwrap();
    f.cx(|cx| m.refresh(&mut view, cx));
    assert!(view.items.is_empty());
    // No window, no session, nothing asked of the normal server.
    assert!(take_log().is_empty());
    assert!(lock(&m.private.room).is_none());
    assert!(!m.shared.lock().models.contains_key("mlx"));
    // No server at all: no item, but the hotkey still says why.
    let mut m = llm("[llm]\nprivate_hotkey = \"cmd+KeyP\"");
    assert!(f.cx(|cx| m.items(cx)).is_empty());
    assert_eq!(hotkey(&mut f, &mut m).unwrap().empty, NO_PRIVATE);
    assert!(f.cx(|cx| m.hotkey("nope", cx)).is_none());
}

/// The body of the private request carrying `marker`: sent over stdin to the private server
/// and never in argv or to the normal server.
fn private_request(marker: &str) -> serde_json::Value {
    let has = |s: &testkit::Sent| String::from_utf8_lossy(&s.stdin).contains(marker);
    let sent_private = sent("http://mlx:11235/v1/chat/completions");
    let req = sent_private.iter().find(|s| has(s)).unwrap();
    assert!(!req.argv.iter().any(|w| w.contains(marker)));
    assert!(!sent("http://mlx/v1/chat/completions").iter().any(has));
    serde_json::from_slice(&req.stdin).unwrap()
}

#[test]
fn a_private_exchange_streams_from_the_private_server_only() {
    let mut f = Fx::new();
    let mut m = llm(&format!("[llm]\nhistory = true\nsystem_prompt = \"be brief\"\n{MLX}{VAULT}"));
    let item = ItemId::new(ID, "private");
    let found = f.cx(|cx| m.items(cx)).into_iter().find(|i| i.id == item).unwrap();
    assert_eq!((found.title.as_str(), found.subtitle.as_str()), ("Private Model Chat", "vault · nothing is saved"));
    assert!(matches!(f.cx(|cx| m.activate(&item, cx)), Outcome::Hide));
    let log = take_log();
    assert_eq!(log.first().map(String::as_str), Some("open"));
    assert!(log.contains(&"hook quit".to_string()));
    assert_eq!(only("header", &log), ["Private Chat|vault · loading models…|Idle"]);
    assert_eq!(log.last().map(String::as_str), Some("show"));
    until(&mut f, &mut m, |m| !model(m).is_empty());
    assert_eq!(model(&m), QWEN);
    take_log();
    send(&mut f, &mut m, "canary 5e1d tell me");
    let log = take_log();
    let rows = only("rows", &log);
    assert_eq!(rows.last().unwrap(), &format!("p0 Done|Mine||canary 5e1d tell me / p1 Done|Theirs|{QWEN}|Hello, world."));
    assert_eq!(only("header", &log).last().unwrap(), &format!("Private Chat|vault · {QWEN}  ·  ⌘C copies the reply  ·  ⌘N or Esc clears|Idle"));
    // The request went to the private server only, over stdin, with the system prompt.
    let body = private_request("5e1d");
    assert_eq!(body["messages"][0]["content"], "be brief");
    // ⌘C copies the last reply through the concealed writer.
    queue_private(Note::Key(Command::Copy));
    changed(&mut f, &mut m);
    let log = take_log();
    assert_eq!(only("copy", &log), ["Hello, world."]);
    assert_eq!(only("notice", &log).last().unwrap(), COPIED);
}

fn copy<T: Copy>() {}

#[test]
fn a_private_exchange_reaches_no_row_command_item_or_event() {
    let mut f = Fx::new();
    let mut m = llm(&format!("[llm]\nhistory = true\n{MLX}{VAULT}"));
    let before = f.dump();
    opened(&mut f, &mut m);
    send(&mut f, &mut m, "canary 77ab tell me");
    assert!(last("rows").ends_with("Hello, world."));
    // flick.db: not one row more, and the text in none.
    let after = f.dump();
    assert_eq!(after, before);
    assert!(!after.iter().any(|r| r.contains("77ab") || r.contains("Hello, world")));
    assert!(m.chat.thread.is_none());
    // No command, item or view carries it; there is no private verb.
    for words in [&["private"][..], &["chat"], &["models", "vault"], &["ping", "vault"], &[]] {
        let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
        for json in [false, true] {
            let got = f.cx(|cx| {
                cx.json = json;
                m.command(&args, cx)
            });
            let text = got.unwrap_err();
            assert!(!text.contains("77ab") && !text.contains("Hello"), "{words:?}: {text}");
        }
    }
    // The models view's retry row never asks a private server.
    let retry = ItemId::new(ID, "retry:vault");
    assert!(matches!(f.cx(|cx| m.activate(&retry, cx)), Outcome::Stay(Some(s)) if s == "Asking vault for its models…"));
    assert!(!m.shared.lock().models.get("vault").is_some_and(|l| l.fetching));
    let shown: Vec<String> = f.cx(|cx| m.items(cx)).iter().map(|i| format!("{} {} {:?}", i.title, i.subtitle, i.keywords)).collect();
    assert!(!shown.iter().any(|s| s.contains("77ab")), "{shown:?}");
    // The only event the module posts is `ModuleChanged`, and no event can own text: `Event`
    // is `Copy`, so it holds no `String`.
    copy::<Event>();
    let posted = serde_json::to_string(&Event::ModuleChanged { module: ID }).unwrap();
    assert_eq!(posted, r#"{"event":"module_changed","module":"llm"}"#);
    // The session's Debug shows counts.
    let debug = format!("{:?}", lock(&m.private.room).as_ref().unwrap().session);
    assert!(!debug.contains("77ab") && debug.contains("entries: 2"), "{debug}");
}

/// A module with one finished private exchange, its log taken.
fn chatted(f: &mut Fx, extra: &str) -> Llm {
    let mut m = llm(&format!("{extra}{VAULT}"));
    opened(f, &mut m);
    send(f, &mut m, "secret");
    assert_eq!(lock(&m.private.room).as_ref().unwrap().session.entries().len(), 2);
    take_log();
    m
}

/// The session is gone and the window was emptied, with `notice`.
fn wiped(m: &Llm, notice: &str) {
    assert!(lock(&m.private.room).is_none());
    let log = take_log();
    assert_eq!(only("input", &log), [""], "{log:?}");
    assert_eq!(only("rows", &log), [""]);
    assert_eq!(only("header", &log), ["Private Chat|Nothing is kept  ·  type to start a new private chat|Idle"]);
    assert_eq!(only("notice", &log), [notice]);
    assert!(!log.iter().any(|l| l.contains("secret")));
}

#[test]
fn lock_sleep_reload_and_new_wipe_the_chat_and_unlock_does_not() {
    let mut f = Fx::new();
    for (e, notice) in [(Event::Locked, "Cleared when the screen locked"), (Event::Sleep, "Cleared when the Mac went to sleep")] {
        let mut m = chatted(&mut f, "");
        assert!(!event(&mut f, &mut m, e));
        wiped(&m, notice);
    }
    let mut m = chatted(&mut f, "");
    for e in [Event::Unlocked, Event::Wake, Event::Started] {
        event(&mut f, &mut m, e);
    }
    assert!(lock(&m.private.room).is_some());
    reconfigure(&mut m, VAULT);
    wiped(&m, "Cleared when the config reloaded");
    let mut m = chatted(&mut f, "");
    queue_private(Note::Key(Command::New));
    changed(&mut f, &mut m);
    let log = take_log();
    assert_eq!(only("notice", &log).last().unwrap(), "Cleared");
    assert!(lock(&m.private.room).is_none());
    // The next prompt opens a new session through the gate.
    send(&mut f, &mut m, "again");
    assert_eq!(lock(&m.private.room).as_ref().unwrap().session.entries().len(), 2);
}

#[test]
fn closing_the_window_wipes_it_and_gives_the_keyboard_back() {
    let mut f = Fx::new();
    // Esc: the surface hid itself.
    testkit::set_front(Some(42));
    let mut m = chatted(&mut f, "");
    closed_private();
    changed(&mut f, &mut m);
    let log = take_log();
    assert!(log.contains(&"refocus 42".to_string()), "{log:?}");
    assert!(lock(&m.private.room).is_none());
    // ⌘W, and the hotkey while the window has the keyboard.
    let closers: [fn(&mut Fx, &mut Llm); 2] = [|_, _| queue_private(Note::Key(Command::Close)), |f, m| assert!(hotkey(f, m).is_none())];
    for close in closers {
        let mut m = chatted(&mut f, "");
        close(&mut f, &mut m);
        changed(&mut f, &mut m);
        let log = take_log();
        assert_eq!(log.first().map(String::as_str), Some("hide"));
        assert_eq!(only("notice", &log).last().unwrap(), "Cleared when the window closed");
        assert!(lock(&m.private.room).is_none());
    }
    // Shown without the keyboard: the hotkey takes it again, no second open.
    let mut m = chatted(&mut f, "");
    testkit::blur();
    assert!(hotkey(&mut f, &mut m).is_none());
    let log = take_log();
    assert!(!log.contains(&"open".to_string()) && log.last().map(String::as_str) == Some("show"), "{log:?}");
    assert!(lock(&m.private.room).is_some());
}

#[test]
fn stop_keeps_what_arrived_and_a_wipe_mid_reply_kills_the_call() {
    let mut f = Fx::new();
    let half = "[[llm.servers]]\nname = \"h\"\nurl = \"http://half\"\nprivate = true\n";
    let start = |f: &mut Fx| {
        let mut m = llm(half);
        assert!(hotkey(f, &mut m).is_none());
        lock(&m.private.room).as_mut().unwrap().session.set_model("q");
        queue_private(Note::Submit("go".into()));
        until(f, &mut m, |_| take_log().iter().any(|l| l.contains("p1 Streaming|Theirs|q|part")));
        m
    };
    let mut m = start(&mut f);
    // A second prompt while it streams goes back into the input.
    queue_private(Note::Submit("too soon".into()));
    changed(&mut f, &mut m);
    let log = take_log();
    assert_eq!(only("input", &log), ["too soon"]);
    assert_eq!(only("notice", &log), ["A reply is on its way; ⌘. stops it"]);
    assert!(only("header", &log)[0].ends_with("h · q  ·  ⌘. stops|Busy"));
    queue_private(Note::Key(Command::Stop));
    until(&mut f, &mut m, |m| !busy(m));
    assert_eq!(last("rows"), "p0 Done|Mine||go / p1 Done|Theirs|q · stopped|part");
    // A prompt waiting for the model list is wiped with the rest.
    let mut m = Llm::with_hooks(testkit::PATIENT, UI, PRIVATE);
    reconfigure(&mut m, "[[llm.servers]]\nname = \"h\"\nurl = \"http://hang\"\nprivate = true\n");
    assert!(hotkey(&mut f, &mut m).is_none());
    queue_private(Note::Submit("wait".into()));
    changed(&mut f, &mut m);
    let log = take_log();
    assert_eq!(only("rows", &log).last().unwrap(), "waiting Pending|Mine||wait");
    assert_eq!(only("header", &log).last().unwrap(), "Private Chat|h · loading models…|Busy");
    queue_private(Note::Key(Command::New));
    changed(&mut f, &mut m);
    assert!(lock(&m.private.room).is_none());
    // ⌘N mid-reply: the call is stopped and nothing of it stays in the shared state.
    let mut m = start(&mut f);
    let id = lock(&m.private.room).as_ref().unwrap().session.streaming().unwrap();
    queue_private(Note::Key(Command::New));
    changed(&mut f, &mut m);
    assert!(m.shared.take(id).is_none());
    assert!(lock(&m.private.room).is_none());
    // The module stopped every call: the reply ends failed.
    let mut m = start(&mut f);
    m.shared.stop();
    changed(&mut f, &mut m);
    let log = take_log();
    assert_eq!(only("rows", &log).last().unwrap(), "p0 Done|Mine||go / p1 Failed|Theirs|q · failed|part");
    assert_eq!(only("notice", &log).last().unwrap(), "llm: the request was dropped");
    assert!(only("header", &log).last().unwrap().ends_with("|Error"));
}

#[test]
fn the_model_list_decides_the_first_send() {
    let mut f = Fx::new();
    let server = |host: &str| format!("[[llm.servers]]\nname = \"{host}\"\nurl = \"http://{host}\"\nprivate = true\n");
    for (host, why) in [
        ("down", "llm: down: (7) Failed to connect to down port 80 after 1 ms: Couldn't connect to server (is the server running, and its port published (tailscale serve)?)"),
        ("none", "llm: none lists no models"),
    ] {
        let mut m = llm(&server(host));
        assert!(hotkey(&mut f, &mut m).is_none());
        queue_private(Note::Submit("hi".into()));
        until(&mut f, &mut m, |m| !busy(m));
        let log = take_log();
        assert_eq!(only("input", &log), ["hi"], "{host}");
        assert_eq!(only("notice", &log).last().unwrap(), why);
        assert!(model(&m).is_empty());
    }
    // `default_model` when the private server lists it.
    let mut m = llm(&format!("[llm]\ndefault_model = \"gemma-3-12b-it-4bit\"\n{VAULT}"));
    opened(&mut f, &mut m);
    assert_eq!(model(&m), "gemma-3-12b-it-4bit");
    // A list that never landed (every call stopped) is asked for again by the next prompt.
    let mut m = llm(VAULT);
    opened(&mut f, &mut m);
    lock(&m.private.room).as_mut().unwrap().session.set_model("");
    m.shared.lock().models.remove("vault");
    queue_private(Note::Submit("late".into()));
    changed(&mut f, &mut m);
    // The fetch the prompt started is stopped before it lands; the prompt still goes out.
    m.shared.stop();
    m.shared.lock().models.insert("vault".into(), io::Models::default());
    until(&mut f, &mut m, |m| !busy(m));
    assert_eq!(lock(&m.private.room).as_ref().unwrap().session.entries().len(), 2);
    // A blank prompt (the surface never sends one) is refused.
    queue_private(Note::Submit("  ".into()));
    changed(&mut f, &mut m);
    assert_eq!(only("notice", &take_log()).last().unwrap(), "llm: nothing to send");
}

/// flick-f4b8: the server was down when the window opened, then started. The next prompt
/// asks for the list again instead of failing on the old error.
#[test]
fn a_prompt_after_a_failed_list_asks_again_and_sends() {
    let mut f = Fx::new();
    let mut m = llm(VAULT);
    opened(&mut f, &mut m);
    lock(&m.private.room).as_mut().unwrap().session.set_model("");
    m.shared.lock().models.get_mut("vault").unwrap().result = Some(Err("(7) Failed to connect".into()));
    take_log();
    send(&mut f, &mut m, "back up");
    assert_eq!(model(&m), QWEN);
    assert_eq!(lock(&m.private.room).as_ref().unwrap().session.entries().len(), 2);
    assert!(!only("notice", &take_log()).iter().any(|n| n.contains("Failed to connect")));
}

#[test]
fn a_reload_that_drops_the_private_server_refuses_the_next_prompt() {
    let mut f = Fx::new();
    let mut m = chatted(&mut f, MLX);
    reconfigure(&mut m, MLX);
    take_log();
    queue_private(Note::Submit("x".into()));
    changed(&mut f, &mut m);
    assert_eq!(only("notice", &take_log()), [NO_PRIVATE]);
    assert!(lock(&m.private.room).is_none());
}

#[test]
fn copy_with_no_reply_copies_nothing() {
    let mut f = Fx::new();
    let mut m = llm(VAULT);
    opened(&mut f, &mut m);
    take_log();
    queue_private(Note::Key(Command::Copy));
    changed(&mut f, &mut m);
    let log = take_log();
    assert!(only("copy", &log).is_empty());
    assert_eq!(only("notice", &log), [NOTHING_TO_COPY]);
    // The normal chat ignores ⌘C notes (its binding never makes one).
    queue(Note::Key(Command::Copy));
    changed(&mut f, &mut m);
    assert!(only("copy", &take_log()).is_empty());
}

#[test]
fn the_quit_hook_wipes_through_weak_handles() {
    let mut f = Fx::new();
    let m = chatted(&mut f, "");
    let (room, shared) = (Arc::downgrade(&m.private.room), Arc::downgrade(&m.shared));
    // A room locked right now is skipped, not waited on.
    {
        let held = lock(&m.private.room);
        wipe_for_quit(&room, &shared);
        assert!(held.is_some());
    }
    wipe_for_quit(&room, &shared);
    assert!(lock(&m.private.room).is_none());
    drop(m);
    // The module is gone: nothing to do.
    wipe_for_quit(&room, &shared);
}

#[test]
fn bubbles_show_each_state_and_never_copy_the_header_from_the_text() {
    let e = |role, text: &str, reasoning: &str, status| Entry { role, text: text.into(), reasoning: reasoning.into(), status };
    let line = |s: &Shown| format!("{} {:?}|{:?}|{}|{}", s.key, s.state, s.side, s.header, s.md);
    let cases = [
        (e(Role::User, "q", "", Status::Done), "p0 Done|Mine||q"),
        (e(Role::Assistant, "", "", Status::Running), "p0 Pending|Theirs|m|Waiting for the model (a cold start loads it first)…"),
        (e(Role::Assistant, "", "hm", Status::Running), "p0 Pending|Theirs|m|Thinking…"),
        (e(Role::Assistant, "ab", "", Status::Running), "p0 Streaming|Theirs|m|ab"),
        (e(Role::Assistant, "ab", "", Status::Cancelled), "p0 Done|Theirs|m · stopped|ab"),
        (e(Role::Assistant, "", "", Status::Failed("boom".into())), "p0 Failed|Theirs|m · failed|boom"),
    ];
    for (entry_, want) in &cases {
        assert_eq!(line(&entry(0, entry_, "m")), *want);
    }
    let a = entry(0, &cases[3].0, "m");
    assert_eq!(a.version, entry(0, &cases[3].0, "m").version);
    assert_ne!(a.version, entry(0, &cases[4].0, "m").version);
    assert_eq!(subtitle(None), "Nothing is kept  ·  type to start a new private chat");
    assert_eq!(status(None), surface::Status::Idle);
}
