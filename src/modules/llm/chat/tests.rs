//! The chat window and the launcher views through the module, over fake curls and a fake
//! window (`testkit`): no window opens and no server is reached.

use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::config::parse;
use crate::core::store::Store;
use crate::core::{Event, ItemId, ListView, Module, Outcome, Ranker};
use crate::modules::llm::testkit::{self, HOOKS, UI, blur, closed, queue, sent, set_front, take_log};
use crate::modules::llm::{ID, store::Chats};

const MLX: &str = "[[llm.servers]]\nname = \"mlx\"\nurl = \"http://mlx\"\n";
const VAULT: &str = "[[llm.servers]]\nname = \"vault\"\nurl = \"http://mlx:11235\"\nprivate = true\n";

/// A store that lives across calls, as flick.db does.
struct Fx {
    store: Store,
    ranker: Ranker,
}

impl Fx {
    fn new() -> Fx {
        let store = Store::in_memory();
        store.migrate(ID, crate::modules::llm::store::MIGRATIONS).unwrap();
        Fx { store, ranker: Ranker::new() }
    }

    fn cx<R>(&mut self, query: &str, f: impl FnOnce(&mut Cx) -> R) -> R {
        f(&mut Cx { query, store: &self.store, ranker: &mut self.ranker, hide: || {}, json: false, remote: false })
    }
}

fn llm(text: &str) -> Llm {
    let mut m = Llm::with_hooks(HOOKS, UI, testkit::PRIVATE);
    m.configure(&parse(text).unwrap().section(ID).unwrap().unwrap()).unwrap();
    take_log();
    m
}

fn event(f: &mut Fx, m: &mut Llm) -> bool {
    f.cx("", |cx| m.on_event(Event::ModuleChanged { module: ID }, cx))
}

/// Drain until no reply runs and no prompt waits.
fn settle(f: &mut Fx, m: &mut Llm) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        event(f, m);
        if (m.chat.reply.is_none() && m.chat.waiting.is_none()) || Instant::now() > deadline {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn send(f: &mut Fx, m: &mut Llm, text: &str) {
    queue(Note::Submit(text.into()));
    settle(f, m);
}

fn hotkey(f: &mut Fx, m: &mut Llm) {
    assert!(f.cx("", |cx| m.hotkey("chat", cx)).is_none(), "the launcher stays as it is");
}

/// The log lines of one kind (`rows`, `header`, …), the kind dropped.
fn only(kind: &str, log: &[String]) -> Vec<String> {
    log.iter().filter_map(|l| l.strip_prefix(kind).map(|r| r.trim_start().to_string())).collect()
}

fn last(kind: &str) -> String {
    only(kind, &take_log()).pop().unwrap_or_default()
}

fn key(c: char) -> Keystroke {
    Keystroke { key: Key::Char(c), cmd: true, shift: false, opt: false }
}

#[test]
fn chat_keys_are_cmd_alone() {
    assert_eq!(binding(key('n')), Some(Command::New));
    assert_eq!(binding(key('.')), Some(Command::Stop));
    assert_eq!(binding(key('w')), Some(Command::Close));
    assert_eq!(binding(key('x')), None);
    assert_eq!(binding(Keystroke { shift: true, ..key('n') }), None);
    assert_eq!(binding(Keystroke { opt: true, ..key('n') }), None);
    assert_eq!(binding(Keystroke { cmd: false, ..key('n') }), None);
}

#[test]
fn thread_ids_are_base36_time_and_a_counter() {
    assert_eq!(thread_id(0, 0), "l00");
    assert_eq!(thread_id(35, 1), "lz1");
    assert_eq!(thread_id(36 * 36, 12), "l10012");
}

#[test]
fn without_a_normal_server_there_is_no_item_hotkey_or_window() {
    for text in ["[llm]\nhotkey = \"cmd+KeyL\"", &format!("[llm]\nhotkey = \"cmd+KeyL\"\n{VAULT}")] {
        let mut m = llm(text);
        let mut f = Fx::new();
        // Only the private chat's item, with a server of any kind (`private_chat`).
        let items: Vec<String> = f.cx("", |cx| m.items(cx)).iter().map(|i| i.id.to_string()).collect();
        assert!(items.iter().all(|i| i == "llm:private"), "{text}: {items:?}");
        assert!(m.hotkeys().is_empty());
        event(&mut f, &mut m);
        assert!(take_log().is_empty());
    }
}

#[test]
fn the_hotkey_toggles_the_window_and_gives_the_keyboard_back() {
    let mut m = llm(&format!("[llm]\nhotkey = \"cmd+KeyL\"\ndefault_model = \"qwen\"\n{MLX}"));
    let b = m.hotkeys();
    assert_eq!((b[0].spec.as_str(), b[0].key.clone()), ("cmd+KeyL", Ok("chat".to_string())));
    let mut f = Fx::new();
    set_front(Some(42));
    hotkey(&mut f, &mut m);
    let log = take_log();
    assert_eq!(log.first().map(String::as_str), Some("open"));
    assert_eq!(only("header", &log), ["New chat|mlx · qwen  ·  ⌘N new chat  ·  Esc hides|Idle"]);
    assert_eq!(only("rows", &log), [""]);
    assert_eq!(log.last().map(String::as_str), Some("show"));
    // With the keyboard: the hotkey hides it and the app from before comes back.
    hotkey(&mut f, &mut m);
    assert_eq!(take_log(), ["hide", "refocus 42"]);
    // Shown without the keyboard: the hotkey takes it again; no second open.
    hotkey(&mut f, &mut m);
    blur();
    hotkey(&mut f, &mut m);
    let log = take_log();
    assert!(!log.contains(&"open".to_string()) && log.last().map(String::as_str) == Some("show"), "{log:?}");
    // Another app came to the front meanwhile: Esc does not steal it back.
    set_front(Some(7));
    closed();
    event(&mut f, &mut m);
    assert!(!take_log().iter().any(|l| l.starts_with("refocus")));
    hotkey(&mut f, &mut m);
    take_log();
    queue(Note::Key(Command::Close));
    event(&mut f, &mut m);
    assert_eq!(take_log(), ["hide", "refocus 7"]);
}

#[test]
fn a_prompt_streams_into_the_window_and_is_kept() {
    let mut m = llm(&format!("[llm]\nsystem_prompt = \"be brief\"\nmax_tokens = 64\n{MLX}"));
    let mut f = Fx::new();
    hotkey(&mut f, &mut m);
    take_log();
    send(&mut f, &mut m, "hello canary 91c2");
    let log = take_log();
    let rows = only("rows", &log);
    // The model came from the server's list; the reply shows as it streams, then is kept.
    assert!(rows.iter().any(|r| r.contains("live ")), "{rows:?}");
    assert_eq!(rows.last().unwrap(), "m0 Done|Mine||hello canary 91c2 / m1 Done|Theirs|qwen3-30b-a3b-4bit|Hello, world.");
    assert_eq!(only("header", &log).last().unwrap(), "hello canary 91c2|mlx · qwen3-30b-a3b-4bit  ·  ⌘N new chat  ·  Esc hides|Idle");
    // The whole request went over stdin, with the system prompt and the token cap.
    let sent = sent("http://mlx/v1/chat/completions");
    let req = sent.iter().find(|s| String::from_utf8_lossy(&s.stdin).contains("canary 91c2")).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&req.stdin).unwrap();
    assert_eq!(body["model"], "qwen3-30b-a3b-4bit");
    assert_eq!(body["messages"][0], serde_json::json!({"role": "system", "content": "be brief"}));
    assert_eq!(body["max_tokens"], 64);
    assert!(!req.argv.iter().any(|w| w.contains("canary")));
    // Kept: the thread is in flick.db with both messages.
    let threads = f.store.llm_threads(10);
    assert_eq!((threads.len(), threads[0].messages, threads[0].title.as_str()), (1, 2, "hello canary 91c2"));
    // A second prompt sends the whole thread.
    send(&mut f, &mut m, "again");
    let sent = testkit::sent("http://mlx/v1/chat/completions");
    let req = sent.iter().rev().find(|s| String::from_utf8_lossy(&s.stdin).contains("again")).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&req.stdin).unwrap();
    let roles: Vec<&str> = body["messages"].as_array().unwrap().iter().map(|t| t["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
    assert_eq!(f.store.llm_threads(10)[0].messages, 4);
}

#[test]
fn history_reopens_after_a_restart_and_the_view_lists_it() {
    let text = format!("[llm]\ndefault_model = \"qwen\"\n{MLX}");
    let mut f = Fx::new();
    let mut m = llm(&text);
    hotkey(&mut f, &mut m);
    send(&mut f, &mut m, "first chat");
    queue(Note::Key(Command::New));
    send(&mut f, &mut m, "second chat");
    drop(m);
    // A fresh module (Flick restarted) shows the newest kept thread.
    let mut m = llm(&text);
    hotkey(&mut f, &mut m);
    assert!(last("rows").starts_with("m0 Done|Mine||second chat / m1 Done|Theirs|qwen|Hello, world."));
    let view = f.cx("", |cx| m.open("threads", cx)).unwrap();
    assert_eq!(view.empty, "No chats kept yet");
    let mut view = view;
    f.cx("", |cx| m.refresh(&mut view, cx));
    let titles: Vec<(&str, &str)> = view.items.iter().map(|i| (i.title.as_str(), i.subtitle.as_str())).collect();
    assert_eq!(titles, [("second chat", "mlx · qwen  ·  2 messages"), ("first chat", "mlx · qwen  ·  2 messages")]);
    // Enter on the older one hides the launcher and shows it.
    let first = view.items[1].id.clone();
    assert!(matches!(f.cx("", |cx| m.activate(&first, cx)), Outcome::Hide));
    assert!(last("rows").starts_with("m0 Done|Mine||first chat"));
    // A thread no longer kept: the window says so and keeps the one it showed.
    let gone = ItemId::new(ID, "thread:nope");
    f.cx("", |cx| m.activate(&gone, cx));
    let log = take_log();
    assert_eq!(only("notice", &log), ["Chat nope is no longer kept"]);
    assert!(only("rows", &log)[0].starts_with("m0 Done|Mine||first chat"));
}

#[test]
fn with_history_off_nothing_is_written() {
    let mut f = Fx::new();
    let mut m = llm(&format!("[llm]\nhistory = false\n{MLX}"));
    hotkey(&mut f, &mut m);
    send(&mut f, &mut m, "keep me out");
    assert!(last("rows").ends_with("Hello, world."));
    let rows: i64 = f.store.conn().query_row("SELECT (SELECT COUNT(*) FROM llm_threads) + (SELECT COUNT(*) FROM llm_messages)", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0);
    let mut view = f.cx("", |cx| m.open("threads", cx)).unwrap();
    assert_eq!(view.empty, "History is off ([llm] history = false)");
    f.cx("", |cx| m.refresh(&mut view, cx));
    assert!(view.items.is_empty());
}

#[test]
fn a_full_store_drops_the_oldest_thread() {
    let mut f = Fx::new();
    let mut m = llm(&format!("[llm]\nmax_threads = 1\ndefault_model = \"q\"\n{MLX}"));
    hotkey(&mut f, &mut m);
    send(&mut f, &mut m, "one");
    queue(Note::Key(Command::New));
    send(&mut f, &mut m, "two");
    let titles: Vec<String> = f.store.llm_threads(10).into_iter().map(|t| t.title).collect();
    assert_eq!(titles, ["two"]);
}

#[test]
fn stop_keeps_what_arrived() {
    let mut f = Fx::new();
    let mut m = llm("[llm]\ndefault_model = \"q\"\n[[llm.servers]]\nname = \"half\"\nurl = \"http://half\"\n");
    hotkey(&mut f, &mut m);
    queue(Note::Submit("go".into()));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !take_log().iter().any(|l| l.contains("live Streaming|Theirs|q|part")) {
        assert!(Instant::now() < deadline, "no partial reply");
        event(&mut f, &mut m);
        thread::sleep(Duration::from_millis(2));
    }
    // A second prompt while it streams goes back into the input.
    queue(Note::Submit("too soon".into()));
    event(&mut f, &mut m);
    let log = take_log();
    assert_eq!(only("input", &log), ["too soon"]);
    assert_eq!(only("notice", &log), ["A reply is on its way; ⌘. stops it"]);
    assert!(only("header", &log)[0].ends_with("half · q  ·  ⌘. stops|Busy"));
    queue(Note::Key(Command::Stop));
    event(&mut f, &mut m);
    assert!(m.chat.reply.is_none());
    assert_eq!(last("rows"), "m0 Done|Mine||go / m1 Done|Theirs|q · stopped|part");
    let (_, msgs) = f.store.llm_thread(&f.store.llm_threads(1)[0].id).unwrap();
    assert_eq!(msgs[1].end, End::Stopped);
}

#[test]
fn a_new_chat_mid_reply_keeps_the_reply_in_its_thread() {
    let mut f = Fx::new();
    let mut m = llm("[llm]\ndefault_model = \"q\"\n[[llm.servers]]\nname = \"half\"\nurl = \"http://half\"\n");
    hotkey(&mut f, &mut m);
    queue(Note::Submit("go".into()));
    event(&mut f, &mut m);
    queue(Note::Key(Command::New));
    event(&mut f, &mut m);
    assert_eq!(last("header"), "New chat|half · q  ·  ⌘N new chat  ·  Esc hides|Idle");
    let old = f.store.llm_threads(5);
    let (_, msgs) = f.store.llm_thread(&old[0].id).unwrap();
    assert_eq!((msgs.len(), &msgs[1].end), (2, &End::Stopped));
}

#[test]
fn a_failed_reply_shows_red_and_is_kept() {
    let mut f = Fx::new();
    let mut m = llm("[llm]\ndefault_model = \"q\"\n[[llm.servers]]\nname = \"err\"\nurl = \"http://err\"\n");
    hotkey(&mut f, &mut m);
    send(&mut f, &mut m, "x");
    let log = take_log();
    assert_eq!(only("rows", &log).last().unwrap(), "m0 Done|Mine||x / m1 Failed|Theirs|q · failed|model not found");
    assert!(only("header", &log).last().unwrap().ends_with("|Error"));
}

#[test]
fn the_model_list_decides_the_first_send() {
    let server = |host: &str| format!("[[llm.servers]]\nname = \"{host}\"\nurl = \"http://{host}\"\n");
    let mut f = Fx::new();
    // The list fails or is empty: the prompt goes back into the input with why.
    for (host, why) in [
        ("down", "llm: down: (7) Failed to connect to down port 80 after 1 ms: Couldn't connect to server (is the server running, and its port published (tailscale serve)?)"),
        ("none", "llm: none lists no models"),
    ] {
        let mut m = llm(&server(host));
        hotkey(&mut f, &mut m);
        take_log();
        queue(Note::Submit("hi".into()));
        event(&mut f, &mut m);
        assert_eq!(last("header"), format!("hi|{host} · loading models…|Busy"));
        settle(&mut f, &mut m);
        let log = take_log();
        assert_eq!(only("input", &log), ["hi"]);
        assert_eq!(only("notice", &log), [why]);
    }
    // A thread whose server is gone or private now: refused, nothing sent.
    let mut m = llm(&format!("{}{VAULT}", server("mlx")));
    hotkey(&mut f, &mut m);
    m.chat.thread.as_mut().unwrap().server = "vault".into();
    send(&mut f, &mut m, "secret");
    assert_eq!(only("notice", &take_log()), ["llm: vault is private; only the private chat talks to it"]);
}

#[test]
fn the_root_item_offers_the_picker_and_the_history() {
    let mut f = Fx::new();
    let mut m = llm(&format!("{MLX}{VAULT}"));
    let item = f.cx("", |cx| m.items(cx)).remove(0);
    assert_eq!((item.id.as_str(), item.title.as_str(), item.subtitle.as_str()), ("llm:chat", "Local Model Chat", "mlx · first listed model"));
    let keys: Vec<&str> = f.cx("", |cx| m.actions(&item.id, cx)).iter().map(|a| a.key).collect();
    assert_eq!(keys, ["models", "threads"]);
    assert!(matches!(f.cx("", |cx| m.act(&item.id, "models", cx)), Outcome::Push(ListView { name, .. }) if name == "models"));
    assert!(matches!(f.cx("", |cx| m.act(&item.id, "nope", cx)), Outcome::Stay(None)));
    assert!(f.cx("", |cx| m.actions(&ItemId::new(ID, "thread:x"), cx)).is_empty());
    assert!(f.cx("", |cx| m.open("nope", cx)).is_none());
}

/// A module over `mlx`, `down` and a private server, with the models view open and every
/// list landed.
fn picker(f: &mut Fx) -> (Llm, ListView) {
    let mut m = llm(&format!("{MLX}[[llm.servers]]\nname = \"down\"\nurl = \"http://down\"\n{VAULT}"));
    // Before the lists land each server says it is loading; after, its models or its error.
    let mut view = ListView::new(ID, "models");
    f.cx("", |cx| m.refresh(&mut view, cx));
    assert_eq!(view.items[0].title, "mlx: loading models…");
    let mut view = f.cx("", |cx| m.open("models", cx)).unwrap();
    let landed = |m: &Llm| ["mlx", "down"].iter().all(|s| m.shared.lock().models.get(*s).is_some_and(|l| l.result.is_some()));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !landed(&m) {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    f.cx("", |cx| m.refresh(&mut view, cx));
    (m, view)
}

#[test]
fn the_model_picker_lists_each_normal_servers_models_or_its_error() {
    let mut f = Fx::new();
    let (mut m, view) = picker(&mut f);
    let rows: Vec<(&str, &str)> = view.items.iter().map(|i| (i.id.as_str(), i.subtitle.as_str())).collect();
    assert_eq!(
        rows,
        [
            ("llm:model:mlx/qwen3-30b-a3b-4bit", "mlx · loaded · ctx 40960"),
            ("llm:model:mlx/gemma-3-12b-it-4bit", "mlx · unloaded"),
            ("llm:retry:down", ""),
        ]
    );
    assert!(view.items[2].title.starts_with("down: (7) Failed to connect"));
    let retry = view.items[2].id.clone();
    assert!(matches!(f.cx("", |cx| m.activate(&retry, cx)), Outcome::Stay(Some(s)) if s == "Asking down for its models…"));
}

#[test]
fn a_picked_model_is_the_chats_from_then_on() {
    let mut f = Fx::new();
    let (mut m, mut view) = picker(&mut f);
    // Enter on a model: the chat uses it from now on, and the window shows.
    let gemma = view.items[1].id.clone();
    assert!(matches!(f.cx("", |cx| m.activate(&gemma, cx)), Outcome::Hide));
    assert_eq!(last("header"), "New chat|mlx · gemma-3-12b-it-4bit  ·  ⌘N new chat  ·  Esc hides|Idle");
    f.cx("", |cx| m.refresh(&mut view, cx));
    assert_eq!(view.items[1].accessory, "Current");
    assert_eq!(f.cx("", |cx| m.items(cx))[0].subtitle, "mlx · gemma-3-12b-it-4bit");
    // Picking again switches the shown thread too; ⌘N keeps the pick.
    let qwen = view.items[0].id.clone();
    f.cx("", |cx| m.activate(&qwen, cx));
    queue(Note::Key(Command::New));
    event(&mut f, &mut m);
    assert!(last("header").contains("mlx · qwen3-30b-a3b-4bit"));
    for bad in ["model:nope/x", "nope"] {
        assert!(matches!(f.cx("", |cx| m.activate(&ItemId::new(ID, bad), cx)), Outcome::Stay(None)));
    }
}

#[test]
fn the_root_item_opens_the_chat() {
    let mut f = Fx::new();
    let mut m = llm(&format!("[llm]\ndefault_model = \"q\"\n{MLX}"));
    assert!(matches!(f.cx("", |cx| m.activate(&ItemId::new(ID, "chat"), cx)), Outcome::Hide));
    assert_eq!(take_log().last().map(String::as_str), Some("show"));
}

#[test]
fn edge_cases_reasoning_a_dropped_stream_and_a_failed_save() {
    let mut f = Fx::new();
    let server = |host: &str| format!("[llm]\ndefault_model = \"q\"\n[[llm.servers]]\nname = \"{host}\"\nurl = \"http://{host}\"\n");
    // A prompt before the window ever showed starts a thread; reasoning shows as thinking.
    let mut m = llm(&server("reason"));
    queue(Note::Submit("2+2".into()));
    event(&mut f, &mut m);
    assert_eq!(m.chat.thread.as_ref().map(|t| t.id.as_str()), Some("lnew1"));
    settle(&mut f, &mut m);
    let t = m.chat.thread.as_ref().unwrap();
    assert_eq!((t.msgs[1].body.as_str(), &t.msgs[1].end), ("4", &End::Done));
    // The module stopped every call (it was dropped or disabled): the reply is kept as failed.
    let mut f = Fx::new();
    let mut m = llm(&server("half"));
    hotkey(&mut f, &mut m);
    queue(Note::Submit("go".into()));
    event(&mut f, &mut m);
    m.shared.stop();
    event(&mut f, &mut m);
    let msgs = &m.chat.thread.as_ref().unwrap().msgs;
    assert_eq!((msgs.len(), &msgs[1].end), (2, &End::Failed("the request was dropped".into())));
    // flick.db refuses the write: the chat goes on and the notice says why.
    let mut f = Fx::new();
    f.store.conn().execute_batch("DROP TABLE llm_messages").unwrap();
    let mut m = llm(&server("mlx"));
    hotkey(&mut f, &mut m);
    send(&mut f, &mut m, "x");
    assert!(only("notice", &take_log()).iter().any(|n| n.starts_with("llm: can't save the chat: ")));
    assert_eq!(m.chat.thread.as_ref().unwrap().msgs.len(), 2);
}
