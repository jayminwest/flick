//! The chat window through the module, with fake hooks (`chat/fake.rs`): no window opens and
//! no ssh runs. The window's calls land in the module tests' log as `chat …` lines.

use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::core::{Event, ItemId, Module, Outcome, later};
use crate::modules::message::chat::fake::{self, queue};
use crate::modules::message::store::{Progress, Role};
use crate::modules::message::tests::{Fixture, inbox, take_log};

const OK: &str = "[message]\nchat_hotkey = \"cmd+KeyJ\"\nkota_host = \"ok@host\"";

fn key(c: char) -> Keystroke {
    Keystroke { key: Key::Char(c), cmd: true, shift: false, opt: false }
}

fn event(f: &mut Fixture, m: &mut Inbox) {
    f.cx("", false, |cx| m.on_event(Event::ModuleChanged { module: "message" }, cx));
}

/// Handle queued notes and finished asks until no ask runs or waits.
fn settle(f: &mut Fixture, m: &mut Inbox) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        event(f, m);
        let idle = m.chat.busy.is_none() && m.chat.queue.is_empty();
        if idle || Instant::now() > deadline {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn hotkey(f: &mut Fixture, m: &mut Inbox) -> bool {
    f.cx("", false, |cx| m.hotkey("chat", cx)).is_none()
}

/// The log lines of one kind (`chat rows`, `chat header`, …), dropping the rest.
fn only(kind: &str, log: &[String]) -> Vec<String> {
    log.iter().filter_map(|l| l.strip_prefix(kind).map(|r| r.trim_start().to_string())).collect()
}

#[test]
fn chat_keys_are_cmd_alone() {
    let k = |c, shift, opt| Keystroke { key: Key::Char(c), cmd: true, shift, opt };
    assert_eq!(binding(key('n')), Some(Command::New));
    assert_eq!(binding(key('[')), Some(Command::Older));
    assert_eq!(binding(key(']')), Some(Command::Newer));
    assert_eq!(binding(key('r')), Some(Command::Retry));
    assert_eq!(binding(key('w')), Some(Command::Close));
    assert_eq!(binding(key('x')), None);
    assert_eq!(binding(k('n', true, false)), None);
    assert_eq!(binding(k('n', false, true)), None);
    assert_eq!(binding(Keystroke { cmd: false, ..key('n') }), None);
    assert_eq!(binding(Keystroke { key: Key::Return, ..key('n') }), None);
}

#[test]
fn chat_is_inert_until_its_hotkey_is_bound() {
    let m = inbox("");
    assert!(m.hotkeys().iter().all(|b| b.key != Ok("chat".to_string())));
    let m = inbox(OK);
    let chat = m.hotkeys().into_iter().find(|b| b.key == Ok("chat".to_string())).unwrap();
    assert_eq!(chat.spec, "cmd+KeyJ");
    assert!(m.chat_showing().is_none());
}

#[test]
fn the_hotkey_toggles_the_window_and_gives_the_keyboard_back() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    fake::set_front(Some(42));
    assert!(hotkey(&mut f, &mut m), "the launcher stays as it is");
    let log = take_log();
    // A fresh store: a new thread, empty, built once and shown with the keyboard.
    assert_eq!(log.first().map(String::as_str), Some("chat open"));
    assert_eq!(only("chat header", &log), ["New chat|⌘N new thread  ·  ⌘[ ⌘] switch  ·  Esc hides|idle"]);
    assert_eq!(log.last().map(String::as_str), Some("chat show"));
    assert_eq!(m.chat_showing(), Some("tnew1"));
    // Pressed again while it has the keyboard: it hides and the app in front gets it back.
    hotkey(&mut f, &mut m);
    assert_eq!(take_log(), ["chat hide", "chat refocus 42"]);
    assert_eq!(m.chat_showing(), None);
    // Shown but without the keyboard (the user clicked elsewhere): it takes it again.
    hotkey(&mut f, &mut m);
    fake::blur();
    hotkey(&mut f, &mut m);
    let log = take_log();
    assert!(!log.contains(&"chat open".to_string()), "built once: {log:?}");
    assert_eq!(only("chat show", &log).len(), 2);
    assert_eq!(m.chat_showing(), Some("tnew1"), "the same thread");
}

#[test]
fn closing_refocuses_only_the_app_still_in_front() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    fake::set_front(Some(42));
    hotkey(&mut f, &mut m);
    take_log();
    // Esc: the surface hid itself; the app from summon is still in front.
    fake::closed();
    event(&mut f, &mut m);
    assert_eq!(take_log(), ["chat refocus 42"]);
    // Another app took over meanwhile: leave it be. ⌘W hides the same way.
    hotkey(&mut f, &mut m);
    take_log();
    fake::set_front(Some(7));
    queue(Note::Key(Command::Close));
    event(&mut f, &mut m);
    assert_eq!(take_log(), ["chat hide"]);
    fake::set_front(None);
    hotkey(&mut f, &mut m);
    fake::closed();
    event(&mut f, &mut m);
    assert!(only("chat refocus", &take_log()).is_empty());
}

#[test]
fn a_question_shows_at_once_and_the_reply_streams_into_its_bubble() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    hotkey(&mut f, &mut m);
    take_log();
    queue(Note::Submit("What's on **today**?".into()));
    settle(&mut f, &mut m);
    let q = f.store.message("m1").unwrap();
    assert_eq!((q.role, q.state, q.thread.as_deref(), q.body.as_str()), (Role::Me, Progress::Done, Some("tnew1"), "What's on **today**?"));
    let log = take_log();
    let rows = only("chat rows", &log);
    assert_eq!(rows.last().unwrap(), "-- Today / m1 Done|Mine||What's on **today**? / thinking:m1 Pending|Theirs|Messages|Thinking…");
    assert_eq!(only("chat header", &log).last().unwrap(), "What's on today?|KOTA is on it…|busy");
    assert!(only("show", &log).is_empty(), "no HUD card for the user's own question");
    // KOTA streams the answer into the thread the window shows: one bubble, no HUD, no sound.
    for body in ["Two", "Two calls"] {
        f.run(&mut m, false, &["post", "--thread", "tnew1", "--reply-to", "m1", "--id", "r1", "--partial", body]).unwrap();
    }
    let log = take_log();
    assert!(only("show", &log).is_empty() && only("notify", &log).is_empty(), "{log:?}");
    assert_eq!(only("chat rows", &log).last().unwrap(), "-- Today / m1 Done|Mine||What's on **today**? / r1 Streaming|Theirs|Messages|Two calls");
    f.run(&mut m, false, &["post", "--thread", "tnew1", "--id", "r1", "Two calls and a PR."]).unwrap();
    let log = take_log();
    assert!(only("show", &log).is_empty());
    assert_eq!(only("chat rows", &log).last().unwrap(), "-- Today / m1 Done|Mine||What's on **today**? / r1 Done|Theirs|Messages|Two calls and a PR.");
    assert_eq!(only("chat header", &log).last().unwrap(), "What's on today?|⌘N new thread  ·  ⌘[ ⌘] switch  ·  Esc hides|idle");
    // The question stays: a reply to a 'me' message never takes it.
    assert_eq!(f.store.thread("tnew1", 10).len(), 2);
}

#[test]
fn posts_to_other_threads_or_a_hidden_window_still_alert() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "a", "first"]).unwrap();
    f.cx("", false, |cx| m.summon(Some("t1".into()), cx));
    take_log();
    f.run(&mut m, false, &["post", "--thread", "t2", "--id", "b", "elsewhere"]).unwrap();
    assert_eq!(only("show", &take_log()).len(), 1, "another thread shows its card");
    f.run(&mut m, false, &["post", "--id", "c", "loose"]).unwrap();
    assert_eq!(only("show", &take_log()).len(), 1, "an unthreaded post shows as before");
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "d", "here"]).unwrap();
    assert!(only("show", &take_log()).is_empty());
    hotkey(&mut f, &mut m);
    take_log();
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "e", "hidden"]).unwrap();
    let log = take_log();
    assert_eq!(only("show", &log).len(), 1);
    assert!(only("chat rows", &log).is_empty(), "a hidden window is not redrawn");
}

#[test]
fn a_failed_ask_is_marked_and_retried_under_its_id() {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nkota_host = \"down@host\""));
    f.cx("", false, |cx| m.summon(Some("t1".into()), cx));
    take_log();
    queue(Note::Submit("ping".into()));
    settle(&mut f, &mut m);
    assert_eq!(f.store.message("m1").unwrap().state, Progress::Failed);
    let log = take_log();
    assert_eq!(only("chat rows", &log).last().unwrap(), "-- Today / m1 Failed|Mine|Not sent · ⌘R retries|ping");
    assert_eq!(only("chat header", &log).last().unwrap(), "ping|Last question not sent  ·  ⌘R retries|error");
    assert_eq!(only("chat notice", &log).last().unwrap(), "Not sent: ssh down@host: down");
    // ⌘R sends the same request again, to the host now set.
    m.settings.kota_host = "echo@host".into();
    queue(Note::Key(Command::Retry));
    settle(&mut f, &mut m);
    let notice = only("chat notice", &take_log()).pop().unwrap();
    assert_eq!(notice, "Not sent: .dotfiles/home/.local/bin/kota-ask --id m1 --thread t1 <ping");
    m.settings.kota_host = "reject@host".into();
    queue(Note::Key(Command::Retry));
    settle(&mut f, &mut m);
    assert_eq!(only("chat notice", &take_log()).pop().unwrap(), "Not sent: KOTA rejected: not now");
    m.settings.kota_host = "ok@host".into();
    queue(Note::Key(Command::Retry));
    settle(&mut f, &mut m);
    assert_eq!(f.store.message("m1").unwrap().state, Progress::Done);
    assert_eq!(f.store.thread("t1", 10).len(), 1, "one question, not two");
    assert!(only("chat notice", &take_log()).is_empty());
    // Nothing failed: ⌘R does nothing.
    queue(Note::Key(Command::Retry));
    settle(&mut f, &mut m);
    assert!(m.chat.busy.is_none());
}

#[test]
fn a_refused_question_goes_back_in_the_input() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    hotkey(&mut f, &mut m);
    take_log();
    let question = "x".repeat(2001);
    queue(Note::Submit(question.clone()));
    event(&mut f, &mut m);
    let log = take_log();
    assert_eq!(only("chat input", &log), [question]);
    assert_eq!(only("chat notice", &log), ["Over 2000 characters"]);
    assert!(f.store.threads(10).is_empty(), "nothing stored");
    // ⌘R with no thread shown does nothing.
    m.chat.thread = None;
    queue(Note::Key(Command::Retry));
    event(&mut f, &mut m);
    assert!(m.chat.busy.is_none() && m.chat.queue.is_empty());
}

#[test]
fn asks_run_one_at_a_time_in_order() {
    let (mut f, mut m) = (Fixture::new(), inbox("[message]\nkota_host = \"order@host\""));
    m.env.new_id = || format!("q{}", NEXT.fetch_add(1, Ordering::Relaxed));
    f.cx("", false, |cx| {
        m.ask("t1", "order one", false, cx).unwrap();
        m.ask("t1", "order two", false, cx).unwrap();
    });
    assert_eq!(m.chat.queue.len(), 1, "the second waits for the first");
    settle(&mut f, &mut m);
    let mine: Vec<String> = fake::asked().into_iter().filter(|q| q.starts_with("order ")).collect();
    assert_eq!(mine, ["order one", "order two"]);
}

static NEXT: AtomicU32 = AtomicU32::new(1);

#[test]
fn threads_switch_with_cmd_n_and_brackets() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    for (t, id) in [("t1", "a"), ("t2", "b"), ("t3", "c")] {
        f.run(&mut m, false, &["post", "--thread", t, "--id", id, &format!("in {t}")]).unwrap();
    }
    // Summoned without a thread: the newest one.
    hotkey(&mut f, &mut m);
    assert_eq!(m.chat_showing(), Some("t3"));
    let step = |f: &mut Fixture, m: &mut Inbox, c| {
        queue(Note::Key(c));
        event(f, m);
        m.chat_showing().map(str::to_string)
    };
    assert_eq!(step(&mut f, &mut m, Command::Older).as_deref(), Some("t2"));
    assert_eq!(step(&mut f, &mut m, Command::Older).as_deref(), Some("t1"));
    assert_eq!(step(&mut f, &mut m, Command::Older).as_deref(), Some("t1"), "the oldest stays");
    assert_eq!(step(&mut f, &mut m, Command::Newer).as_deref(), Some("t2"));
    take_log();
    assert_eq!(step(&mut f, &mut m, Command::New).as_deref(), Some("tnew1"));
    assert_eq!(only("chat rows", &take_log()).last().unwrap(), "");
    assert_eq!(step(&mut f, &mut m, Command::Newer).as_deref(), Some("tnew1"), "nothing newer than a new thread");
    assert_eq!(step(&mut f, &mut m, Command::Older).as_deref(), Some("t3"));
}

#[test]
fn verbs_open_the_window_and_ask() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    assert_eq!(f.run(&mut m, false, &["chat", "--snapshot", "/x.png"]), Err("The chat window was not opened yet".into()));
    assert_eq!(f.run(&mut m, false, &["chat", "--thread", "t9"]), Ok("Chat shows thread t9".into()));
    assert_eq!(m.chat_showing(), Some("t9"));
    take_log();
    assert_eq!(f.run(&mut m, false, &["chat", "--snapshot", "/x.png"]), Ok("Wrote /x.png".into()));
    assert_eq!(only("chat snapshot", &take_log()), ["/x.png"]);
    assert_eq!(f.run(&mut m, false, &["chat", "--thread", "t8", "--snapshot", "x.png"]), Err("can't write x.png".into()));
    assert_eq!(m.chat.thread.as_deref(), Some("t8"));
    let usage = "usage: flick message chat [--thread t] [--snapshot <png>]";
    assert_eq!(f.run(&mut m, false, &["chat", "now"]), Err(usage.into()));
    assert!(f.run(&mut m, false, &["chat", "--thread", "a b"]).unwrap_err().contains("a thread id is"));
}

#[test]
fn ask_answers_once_ssh_is_done() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    f.cx("", false, |cx| m.summon(Some("t8".into()), cx));
    assert_eq!(f.run(&mut m, false, &["ask"]), Err("usage: flick message ask [--thread t] <text...>".into()));
    assert!(later::take().is_none());
    // `ask` answers once ssh is done; without --thread it asks in the window's thread.
    let asked = f.run(&mut m, false, &["ask", "status", "please"]);
    assert_eq!(asked, Ok("Asking KOTA (m1) in thread t8".into()));
    let rx = later::take().unwrap();
    settle(&mut f, &mut m);
    assert_eq!(later::settle(asked, Some(rx), later::MAX_WAIT), Ok("Asked KOTA (m1)".into()));
    assert_eq!(f.store.message("m1").unwrap().body, "status please");
    m.settings.kota_host = "down@host".into();
    m.env.new_id = || "m2".into();
    let asked = f.run(&mut m, false, &["ask", "--thread", "t2", "hi"]);
    let rx = later::take().unwrap();
    settle(&mut f, &mut m);
    assert_eq!(later::settle(asked, Some(rx), later::MAX_WAIT), Err("message ask: ssh down@host: down".into()));
    assert_eq!(f.store.message("m2").unwrap().thread.as_deref(), Some("t2"));
    // A refused question answers at once and stores nothing.
    assert_eq!(f.run(&mut m, false, &["ask", " "]), Err("Type a question first".into()));
    assert!(later::take().is_none());
}

#[test]
fn ask_needs_a_window_thread_or_makes_one() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    assert!(f.run(&mut m, false, &["ask", "hello"]).unwrap().ends_with("in thread tnew1"));
    let _ = later::take();
    settle(&mut f, &mut m);
    // The window was never opened: nothing drawn.
    assert!(only("chat rows", &take_log()).is_empty());
}

#[test]
fn the_threads_view_lists_and_opens_threads() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "a", "**Plan** for today?"]).unwrap();
    f.run(&mut m, false, &["post", "--thread", "t1", "--id", "b", "Two calls."]).unwrap();
    f.run(&mut m, false, &["post", "--thread", "t2", "--id", "c", "Hm"]).unwrap();
    take_log();
    let list = ItemId::new("message", "list");
    let actions = f.cx("", false, |cx| m.actions(&list, cx));
    assert_eq!(actions.iter().map(|a| a.key).collect::<Vec<_>>(), ["threads"]);
    let Outcome::Push(mut view) = f.cx("", false, |cx| m.act(&list, "threads", cx)) else { panic!("no view") };
    assert_eq!(view.name, "threads");
    let mut opened = f.cx("", false, |cx| m.open("threads", cx)).unwrap();
    assert_eq!(opened.empty, "No chat threads yet");
    f.cx("", false, |cx| m.refresh(&mut view, cx));
    let rows: Vec<String> = view.items.iter().map(|i| format!("{} {} {}", i.id, i.title, i.subtitle)).collect();
    assert_eq!(rows, ["message:thread:t2 Hm 11:31  ·  1 message", "message:thread:t1 Plan for today? 11:31  ·  2 messages"]);
    f.cx("plan", false, |cx| m.refresh(&mut opened, cx));
    assert_eq!(opened.items.len(), 1);
    let id = ItemId::new("message", "thread:t1");
    assert!(matches!(f.cx("", false, |cx| m.activate(&id, cx)), Outcome::Hide));
    assert_eq!(m.chat_showing(), Some("t1"));
    let log = take_log();
    assert_eq!(log.first().map(String::as_str), Some("launcher hide"));
    assert_eq!(only("chat header", &log), ["Plan for today?|⌘N new thread  ·  ⌘[ ⌘] switch  ·  Esc hides|idle"]);
    assert!(matches!(f.cx("", false, |cx| m.act(&list, "other", cx)), Outcome::Stay(None)));
}

#[test]
fn card_presses_in_the_window_go_through_the_card_dispatch() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    let card = r#"{"id":"c1","title":"Ship?","thread":"t1","actions":[{"id":"go","label":"Ship"}]}"#;
    f.run(&mut m, false, &["card", "post", card]).unwrap();
    f.cx("", false, |cx| m.summon(Some("t1".into()), cx));
    take_log();
    queue(Note::Press { card: "c1".into(), action: "go".into(), values: "{}".into() });
    event(&mut f, &mut m);
    let rows = only("chat rows", &take_log());
    // No action_command: the error shows on the card in the transcript.
    assert_eq!(rows.last().unwrap(), "-- Today / c1 card|false|Some(\"Set [message] action_command to send presses to KOTA\")");
}

#[test]
fn kota_host_and_path_are_checked() {
    for (config, want) in [
        ("kota_host = \"-oProxy\"", "[message]: \"-oProxy\": not an ssh host"),
        ("kota_ask = \"a;b\"", "[message]: \"a;b\": kota-ask must be a path of A-Z a-z 0-9 . _ / ~ -"),
    ] {
        let table = crate::config::parse(&format!("[message]\n{config}")).unwrap();
        let mut m = Inbox::default();
        assert_eq!(m.configure(&table.section("message").unwrap().unwrap()), Err(want.into()));
    }
    let m = inbox("");
    assert_eq!((m.settings.kota_host.as_str(), m.settings.kota_ask.as_str()), ("jaymin@mbp-server", ".dotfiles/home/.local/bin/kota-ask"));
    assert_eq!(m.settings.chat_hotkey, None);
}
