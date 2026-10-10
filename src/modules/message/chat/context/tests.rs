//! Context chips, pure and through the module with fake hooks (`chat/fake.rs`): no window,
//! no clipboard, no screen and no ssh are touched.

use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::core::{Event, Module};
use crate::modules::message::chat::fake::{self, queue};
use crate::modules::message::chat::session::{Command, Note};
use crate::modules::message::store::{Messages, Progress};
use crate::modules::message::tests::{Fixture, inbox, take_log};

fn front(window: Option<&str>, selection: Option<&str>) -> Front {
    Front {
        pid: 42,
        app: "Safari".into(),
        bundle_id: Some("com.apple.Safari".into()),
        window: window.map(str::to_string),
        selection: selection.map(str::to_string),
    }
}

fn png(n: usize) -> Arc<[u8]> {
    Arc::from(vec![7u8; n])
}

#[test]
fn the_front_app_gives_app_window_and_selection_chips() {
    let all = from_front(front(Some("Inbox"), Some("picked")));
    let app = ask::Item::App { name: "Safari".into(), bundle: Some("com.apple.Safari".into()) };
    assert_eq!(all, [Attached::Item(app.clone()), Attached::Item(ask::Item::Window("Inbox".into())), Attached::Item(ask::Item::Selection("picked".into()))]);
    assert_eq!(from_front(front(None, None)), [Attached::Item(app)]);
    assert_eq!(Attached::Shot(png(1)).chip(), ask::Item::Screenshot(String::new()).chip());
}

#[test]
fn a_plan_numbers_screenshots_and_names_their_remote_paths() {
    let attached = [Attached::Shot(png(3)), Attached::Item(ask::Item::Window("W".into())), Attached::Shot(png(5))];
    let (items, uploads) = plan(&attached, "h@x", ".cache/flick/attach", "k1").unwrap();
    assert_eq!(
        items,
        [
            ask::Item::Screenshot(".cache/flick/attach/k1-1.png".into()),
            ask::Item::Window("W".into()),
            ask::Item::Screenshot(".cache/flick/attach/k1-2.png".into()),
        ]
    );
    assert_eq!(uploads.len(), 2);
    assert_eq!(uploads[0].argv, attach::upload_argv("h@x", ".cache/flick/attach", "k1", 1).unwrap());
    assert_eq!(uploads[1].argv, attach::upload_argv("h@x", ".cache/flick/attach", "k1", 2).unwrap());
    assert_eq!((uploads[0].png.len(), uploads[1].png.len()), (3, 5));
    // No screenshot: no upload, and the dir is not even looked at.
    let (items, uploads) = plan(&[Attached::Item(ask::Item::Clipboard("c".into()))], "h@x", "../bad", "k1").unwrap();
    assert_eq!((items.len(), uploads.len()), (1, 0));
    assert!(plan(&[Attached::Shot(png(1))], "h@x", "../bad", "k1").unwrap_err().contains("attach_dir"));
}

#[test]
fn uploads_stop_at_the_first_failure() {
    let up = |dir: &str| Upload { argv: vec![dir.into()], png: png(1) };
    let ok: fn(&[String], &[u8]) -> Exit = |argv, _| match argv[0].as_str() {
        "ok" => Exit::Sent,
        "no" => Exit::Rejected("full".into()),
        _ => Exit::Failed("ssh: lost".into()),
    };
    assert_eq!(upload_all(ok, &[]), Ok(()));
    assert_eq!(upload_all(ok, &[up("ok"), up("ok")]), Ok(()));
    assert_eq!(upload_all(ok, &[up("ok"), up("down"), up("no")]), Err("screenshot upload failed: ssh: lost".into()));
    assert_eq!(upload_all(ok, &[up("no")]), Err("screenshot upload failed: full".into()));
}

const OK: &str = "[message]\nchat_hotkey = \"cmd+KeyJ\"\nkota_host = \"ok@host\"";

fn event(f: &mut Fixture, m: &mut Inbox) {
    f.cx("", false, |cx| m.on_event(Event::ModuleChanged { module: "message" }, cx));
}

fn settle(f: &mut Fixture, m: &mut Inbox) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        event(f, m);
        if (m.chat.busy.is_none() && m.chat.queue.is_empty()) || Instant::now() > deadline {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn only(kind: &str, log: &[String]) -> Vec<String> {
    log.iter().filter_map(|l| l.strip_prefix(kind).map(|r| r.trim_start().to_string())).collect()
}

fn note(f: &mut Fixture, m: &mut Inbox, n: Note) -> Vec<String> {
    queue(n);
    event(f, m);
    take_log()
}

/// ⌘⇧S, then the shot's note, which the window queues later.
fn shoot(f: &mut Fixture, m: &mut Inbox) -> Vec<String> {
    queue(Note::Key(Command::Screenshot));
    event(f, m);
    event(f, m);
    take_log()
}

fn summon(f: &mut Fixture, m: &mut Inbox) -> Vec<String> {
    f.cx("", false, |cx| m.summon(None, cx));
    take_log()
}

#[test]
fn summon_shows_the_front_apps_chips_and_a_new_summon_replaces_them() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    fake::set_context(Some(front(Some("Inbox - Fastmail"), Some("the bit I mean"))));
    let log = summon(&mut f, &mut m);
    let chips = only("chat chips", &log);
    assert_eq!(chips, ["app.dashed:Safari | macwindow:Inbox - Fastmail | text.quote:“the bit I mean”"]);
    let at = |l: &str| log.iter().position(|x| x.starts_with(l)).unwrap();
    assert!(at("chat open") < at("chat chips") && at("chat chips") < at("chat show"), "{log:?}");
    // Shown already: no new read, the chips stay.
    fake::set_context(None);
    assert!(only("chat chips", &summon(&mut f, &mut m)).is_empty());
    assert_eq!(m.chat.attached.len(), 3);
    // Hidden and summoned again: the new front app's chips (here none).
    f.cx("", false, |cx| m.chat_toggle(cx));
    assert_eq!(only("chat chips", &summon(&mut f, &mut m)), [""]);
}

#[test]
fn chips_come_off_when_clicked() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    fake::set_context(Some(front(Some("W"), None)));
    summon(&mut f, &mut m);
    assert_eq!(only("chat chips", &note(&mut f, &mut m, Note::Unchip(0))), ["macwindow:W"]);
    assert!(only("chat chips", &note(&mut f, &mut m, Note::Unchip(5))).is_empty(), "no such chip");
    assert_eq!(only("chat chips", &note(&mut f, &mut m, Note::Unchip(0))), [""]);
}

#[test]
fn cmd_shift_v_attaches_the_clipboard_once() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    summon(&mut f, &mut m);
    fake::set_clipboard(None);
    let log = note(&mut f, &mut m, Note::Key(Command::Clipboard));
    assert_eq!(only("chat notice", &log), ["The clipboard holds no text"]);
    fake::set_clipboard(Some("  \n"));
    assert_eq!(only("chat notice", &note(&mut f, &mut m, Note::Key(Command::Clipboard))), ["The clipboard holds no text"]);
    fake::set_clipboard(Some("first"));
    assert_eq!(only("chat chips", &note(&mut f, &mut m, Note::Key(Command::Clipboard))), ["doc.on.clipboard:Clipboard"]);
    fake::set_clipboard(Some("second"));
    assert_eq!(only("chat chips", &note(&mut f, &mut m, Note::Key(Command::Clipboard))), ["doc.on.clipboard:Clipboard"]);
    assert_eq!(m.chat.attached, [Attached::Item(ask::Item::Clipboard("second".into()))]);
}

#[test]
fn cmd_shift_s_attaches_screenshots_up_to_the_cap() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    summon(&mut f, &mut m);
    fake::set_shot(Err("Screen Recording is off for Flick"));
    let log = shoot(&mut f, &mut m);
    assert_eq!(only("chat shoot", &log).len(), 1);
    assert_eq!(only("chat notice", &log), ["Screen Recording is off for Flick"]);
    assert!(m.chat.attached.is_empty());
    fake::set_shot(Ok(&[]));
    assert_eq!(only("chat notice", &shoot(&mut f, &mut m)), ["The screenshot is empty"]);
    fake::set_shot(Ok(b"png"));
    for _ in 0..MAX_SHOTS {
        let log = shoot(&mut f, &mut m);
        assert_eq!(only("chat chips", &log).last().unwrap().matches("camera.viewfinder:Screenshot").count(), m.chat.attached.len());
    }
    let log = shoot(&mut f, &mut m);
    assert!(only("chat shoot", &log).is_empty(), "no fourth shot taken");
    assert_eq!(only("chat notice", &log).last().unwrap(), "At most 3 screenshots per question");
}

#[test]
fn a_question_carries_its_chips_and_uploads_first() {
    let config = "[message]\nkota_host = \"echo@host\"\nattach_dir = \"t-carry/attach\"";
    let (mut f, mut m) = (Fixture::new(), inbox(config));
    fake::set_context(Some(front(Some("Inbox"), None)));
    summon(&mut f, &mut m);
    fake::set_shot(Ok(b"12345"));
    shoot(&mut f, &mut m);
    queue(Note::Submit("What is this?".into()));
    settle(&mut f, &mut m);
    let log = take_log();
    // The chips are used up at once.
    assert_eq!(only("chat chips", &log).last().unwrap(), "");
    // echo@host fails with the argv and stdin, so the notice shows what kota-ask got.
    let want = "Not sent: .dotfiles/home/.local/bin/kota-ask --id m1 --thread tnew1 <What is this?\n\n[context]\napp: Safari (com.apple.Safari)\nwindow: Inbox\nscreenshot: ~/t-carry/attach/m1-1.png\n[/context]";
    assert_eq!(only("chat notice", &log).last().unwrap(), want);
    let ups: Vec<String> = fake::uploads().into_iter().filter(|u| u.contains("t-carry/")).collect();
    let cmd = attach::upload_argv("echo@host", "t-carry/attach", "m1", 1).unwrap().pop().unwrap();
    assert_eq!(ups, [format!("{cmd} 5")]);
    // ⌘R sends the same chips again under the same paths.
    m.settings.kota_host = "ok@host".into();
    queue(Note::Key(Command::Retry));
    settle(&mut f, &mut m);
    let ups: Vec<String> = fake::uploads().into_iter().filter(|u| u.contains("t-carry/")).collect();
    assert_eq!(ups.len(), 2);
    assert!(ups[1].contains("cat > t-carry/attach/m1-1.png") && ups[1].ends_with(" 5"), "{ups:?}");
    assert_eq!(f.store.message("m1").unwrap().state, Progress::Done);
    assert!(m.chat.unsent.is_empty());
}

#[test]
fn a_failed_upload_fails_the_ask_before_kota_ask_runs() {
    let config = "[message]\nkota_host = \"order@host\"\nattach_dir = \"t-fail/attach\"";
    let (mut f, mut m) = (Fixture::new(), inbox(config));
    m.env.new_id = || "upfail1".into();
    summon(&mut f, &mut m);
    fake::set_shot(Ok(b"x"));
    shoot(&mut f, &mut m);
    queue(Note::Submit("never asked".into()));
    settle(&mut f, &mut m);
    assert_eq!(only("chat notice", &take_log()).last().unwrap(), "Not sent: screenshot upload failed: ssh: lost");
    assert_eq!(f.store.message("upfail1").unwrap().state, Progress::Failed);
    assert!(!fake::asked().iter().any(|q| q.starts_with("never asked")), "kota-ask never ran");
    assert_eq!(m.chat.unsent.len(), 1);
}

#[test]
fn a_refused_question_keeps_its_chips_and_message_ask_sends_none() {
    let (mut f, mut m) = (Fixture::new(), inbox(OK));
    fake::set_context(Some(front(None, Some("secret-ish"))));
    summon(&mut f, &mut m);
    let log = note(&mut f, &mut m, Note::Submit(" ".into()));
    assert_eq!(only("chat notice", &log), ["Type a question first"]);
    assert_eq!(m.chat.attached.len(), 2);
    // `message ask` sends no chips, and leaves the window's alone.
    m.settings.kota_host = "echo@host".into();
    f.run(&mut m, false, &["ask", "plain"]).unwrap();
    let _ = crate::core::later::take();
    settle(&mut f, &mut m);
    assert!(only("chat notice", &take_log()).last().unwrap().ends_with("<plain"));
    assert_eq!(m.chat.attached.len(), 2);
}

#[test]
fn only_the_last_few_unsent_questions_keep_their_chips() {
    let mut m = inbox(OK);
    let one = || vec![Attached::Item(ask::Item::Window("w".into()))];
    m.keep_unsent("none", vec![]);
    assert!(m.chat.unsent.is_empty(), "nothing to keep");
    for i in 0..=KEPT {
        m.keep_unsent(&format!("r{i}"), one());
    }
    m.keep_unsent("r4", one());
    let kept: Vec<&str> = m.chat.unsent.iter().map(|(r, _)| r.as_str()).collect();
    assert_eq!(kept, ["r1", "r2", "r3", "r4"]);
    assert_eq!(m.take_unsent("r2"), one());
    assert!(m.take_unsent("r2").is_empty());
}

#[test]
fn attach_dir_is_checked() {
    let table = crate::config::parse("[message]\nattach_dir = \"../up\"").unwrap();
    let mut m = Inbox::default();
    let err = m.configure(&table.section("message").unwrap().unwrap()).unwrap_err();
    assert!(err.starts_with("[message] attach_dir \"../up\""), "{err}");
    assert_eq!(inbox("").settings.attach_dir, attach::DEFAULT_DIR);
}
