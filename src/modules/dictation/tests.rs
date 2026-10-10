//! Tests for module `dictation`.

mod flow;

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::time::Duration;

use super::*;
use crate::config::parse;
use crate::core::test_cx;

thread_local! {
    /// What the fake microphone check answers.
    static MIC: Cell<Mic> = const { Cell::new(Mic::Unchecked) };
    /// `ask_mic`, `arm` and `later` calls.
    static ASKED: Cell<u32> = const { Cell::new(0) };
    static ARMED: Cell<u32> = const { Cell::new(0) };
    static LATER: Cell<u32> = const { Cell::new(0) };
    /// Esc on the pill, taken by `take_cancel`.
    static ESC: Cell<bool> = const { Cell::new(false) };
    /// Milliseconds the fake clock is ahead of `base()`.
    static CLOCK: Cell<u64> = const { Cell::new(0) };
    static FLAGS: Cell<u64> = const { Cell::new(0) };
    static FRONT: Cell<Option<i32>> = const { Cell::new(Some(1)) };
    static SECURE: Cell<bool> = const { Cell::new(false) };
    static CACHE: RefCell<PathBuf> = RefCell::new(PathBuf::from("/nonexistent/flick-dictation"));
    /// Everything the fakes were asked to show, insert and log, in order.
    static SEEN: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}

fn base() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

fn saw(line: String) {
    SEEN.with_borrow_mut(|s| s.push(line));
}

/// What the fakes saw since the last call.
fn seen() -> Vec<String> {
    SEEN.with_borrow_mut(std::mem::take)
}

/// Fake system: everything under /opt/homebrew/bin and /fake is a program, files under
/// /models exist, nothing else does. Programs are `fake::spawn`'s scripts.
static FAKE: Hooks = Hooks {
    probe: |path| {
        if path.starts_with("/opt/homebrew/bin") || path.starts_with("/fake") {
            Probe::Program
        } else if path.starts_with("/models") || path.starts_with("/Users/u/bin") {
            Probe::File
        } else {
            Probe::Missing
        }
    },
    mic: || MIC.with(Cell::get),
    ask_mic: || ASKED.set(ASKED.get() + 1),
    home: || PathBuf::from("/Users/u"),
    cache: || CACHE.with_borrow(Clone::clone),
    spawn: fake::spawn,
    notify: || {},
    later: |secs| {
        assert!((secs - super::flow::RELEASE_POLL).abs() < f64::EPSILON);
        LATER.set(LATER.get() + 1);
    },
    arm: || ARMED.set(ARMED.get() + 1),
    take_cancel: || ESC.replace(false),
    now: || base() + Duration::from_millis(CLOCK.get()),
    pill: |p| {
        saw(match p {
            Pill::Recording(_) => "pill recording".into(),
            Pill::Transcribing => "pill transcribing".into(),
            Pill::Result(t) => format!("pill result {t}"),
            Pill::Error(t) => format!("pill error {t}"),
            Pill::Hide => "pill hide".into(),
        });
    },
    frontmost: || FRONT.get(),
    flags: || FLAGS.get(),
    secure_input: || SECURE.get(),
    paste: |text, ms| saw(format!("paste {text:?} restore {ms}")),
    type_text: |text| saw(format!("type {text:?}")),
    log: |line| saw(format!("log {line}")),
};

fn configured(text: &str) -> Result<Dictation, String> {
    let mut d = Dictation::with(&FAKE);
    d.configure(&parse(text)?.section("dictation")?.ok_or("disabled")?)?;
    Ok(d)
}

fn run(d: &mut Dictation, words: &[&str]) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|s| (*s).to_string()).collect();
    test_cx("", |cx| d.command(&args, cx))
}

#[test]
fn off_without_a_table() {
    let mut d = configured("").unwrap();
    assert!(!d.on);
    assert_eq!(d.settings, Settings::default());
    let status = run(&mut d, &["status"]).unwrap();
    assert_eq!(
        status,
        "dictation: off: set a key in [dictation] to turn it on\n\
         state: idle\n\
         engine: whisper /opt/homebrew/bin/whisper-cli (found)\n\
         model: /Users/u/Library/Application Support/Flick/models/ggml-large-v3-turbo-q5_0.bin \
         (missing; Flick never downloads models, see docs/dictation.md)\n\
         recorder: /opt/homebrew/bin/rec (found)\n\
         microphone: not checked\n\
         insert: paste (clipboard restored after 250 ms)"
    );
    // An empty table, or one with only `enabled = true`, is the same as none: no mic path.
    for bare in ["[dictation]", "[dictation]\nenabled = true"] {
        let mut d = configured(bare).unwrap();
        assert!(!d.on, "{bare}");
        assert_eq!(run(&mut d, &["start"]).unwrap_err(), "dictation: off: set a key in [dictation] to turn it on");
    }
}

#[test]
fn status_reports_the_configured_engine() {
    let text = "[dictation]\nengine = \"parakeet\"\nmodel = \"/models/p.bin\"\nparakeet_bin = \"~/bin/p\"\n\
                insert = \"type\"\nrecorder = \"/usr/local/bin/rec\"";
    let mut d = configured(text).unwrap();
    MIC.with(|m| m.set(Mic::Denied));
    assert_eq!(
        run(&mut d, &["status"]).unwrap(),
        "dictation: on\nstate: idle\nengine: parakeet /Users/u/bin/p (not executable)\n\
         model: /models/p.bin (found)\nrecorder: /usr/local/bin/rec (missing)\n\
         microphone: denied (System Settings > Privacy & Security > Microphone)\ninsert: type"
    );
    let text = "[dictation]\nengine = \"command\"\ncommand = [\"/opt/homebrew/bin/stt\", \"{wav}\"]";
    MIC.with(|m| m.set(Mic::Authorized));
    let status = run(&mut configured(text).unwrap(), &["status"]).unwrap();
    assert!(status.contains("engine: command /opt/homebrew/bin/stt (found)\nrecorder:"), "{status}");
    assert!(status.contains("microphone: authorized"), "{status}");
    assert!(status.ends_with("command: /opt/homebrew/bin/stt {wav}"), "{status}");
    MIC.with(|m| m.set(Mic::Unchecked));
}

#[test]
fn microphone_states_have_text() {
    let texts = [Mic::Restricted, Mic::NotDetermined].map(Mic::text);
    assert_eq!(texts, ["restricted by a profile", "not determined (macOS asks at the first dictation)"]);
}

#[test]
fn reload_keeps_the_session_and_takes_the_new_hold() {
    let mut d = configured("[dictation]\nmin_hold_ms = 0").unwrap();
    d.session.step(session::Input::Start).unwrap();
    let table = parse("[dictation]\nmin_hold_ms = 500").unwrap().section("dictation").unwrap().unwrap();
    d.configure(&table).unwrap();
    assert_eq!(d.session.state(), session::State::Recording);
    let short = d.session.step(session::Input::Stop { held_ms: 400 });
    assert_eq!(short, Ok(session::Effect::Discard(session::Discard::TooShort)));
}

#[test]
fn bad_tables_and_verbs_are_errors() {
    let err = configured("[dictation]\nmax_seconds = 0").err().unwrap();
    assert_eq!(err, "[dictation] max_seconds: must be 1 to 600");
    assert!(configured("[dictation]\nengine = 3").err().unwrap().starts_with("[dictation]: "));
    let mut d = configured("").unwrap();
    assert_eq!(run(&mut d, &["start", "now"]).unwrap_err(), "dictation: unknown command \"start\"");
    assert_eq!(run(&mut d, &[]).unwrap_err(), "dictation: missing command");
    assert_eq!(d.verbs(), "dictation start | dictation stop | dictation cancel | dictation last | dictation status");
    assert_eq!(Dictation::default().id(), "dictation");
}

#[test]
fn the_example_config_lists_every_key() {
    crate::config::example::assert_documents::<Settings>("dictation");
}
