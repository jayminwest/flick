//! A dictation through the verbs and events, against the fake recorder, engine and system.

use std::fs;
use std::thread;

use super::*;
use crate::modules::dictation::flow::MODIFIERS;
use crate::modules::dictation::session::State;

/// The fake recorder and engine; `setup`'s `extra` lines replace keys set here.
const CONFIG: [&str; 3] = ["recorder = \"/fake/rec\"", "whisper_bin = \"/fake/stt\"", "model = \"/models/ok.bin\""];

/// The clip directory of one test; removed on drop.
struct Cache(PathBuf);

impl Drop for Cache {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A module on `CONFIG` plus `extra`, with a fresh fake system and clip directory.
fn setup(test: &str, extra: &str) -> (Dictation, Cache) {
    MIC.set(Mic::Authorized);
    CLOCK.set(0);
    FLAGS.set(0);
    FRONT.set(Some(1));
    SECURE.set(false);
    ESC.set(false);
    LATER.set(0);
    seen();
    let dir = std::env::temp_dir().join(format!("flick-dictation-flow-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    CACHE.set(dir.clone());
    let key = |line: &str| line.split(" =").next().unwrap_or_default().to_string();
    let extra: Vec<&str> = extra.lines().collect();
    let keep = CONFIG.into_iter().filter(|line| extra.iter().all(|e| key(e) != key(line)));
    let table = keep.chain(extra.iter().copied()).collect::<Vec<_>>().join("\n");
    (configured(&format!("[dictation]\n{table}")).unwrap(), Cache(dir))
}

fn event(d: &mut Dictation, event: Event) {
    assert!(!test_cx("", |cx| d.on_event(event, cx)));
}

fn wake(d: &mut Dictation) {
    event(d, Event::ModuleChanged { module: "dictation" });
}

fn wait_for(f: impl Fn() -> bool) {
    let since = Instant::now();
    while !f() {
        assert!(since.elapsed() < Duration::from_secs(5), "timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Wait for the worker's result, then let the module take it.
fn settle(d: &mut Dictation) {
    let inbox = d.inbox.clone();
    wait_for(|| !inbox.lock().unwrap().is_empty());
    wake(d);
}

/// Hold the chord for `ms`.
fn hold(d: &mut Dictation, ms: u64) -> Result<String, String> {
    assert_eq!(run(d, &["start"]), Ok("dictation: recording".into()));
    CLOCK.set(CLOCK.get() + ms);
    run(d, &["stop"])
}

fn clips(cache: &Cache) -> usize {
    fs::read_dir(&cache.0).map_or(0, Iterator::count)
}

#[test]
fn hold_speak_release_pastes_the_transcript() {
    let (mut d, cache) = setup("paste", "");
    assert_eq!(hold(&mut d, 1000), Ok("dictation: transcribing".into()));
    assert_eq!(seen(), ["pill recording", "pill transcribing"]);
    assert_eq!(d.session.state(), State::Transcribing);
    settle(&mut d);
    let seen = seen();
    assert!(seen[0].starts_with("log dictation: 0.5 s of audio, engine "), "{seen:?}");
    assert!(seen[0].ends_with(" s, 13 chars"), "{seen:?}");
    assert_eq!(seen[1..], ["paste \"Hello world. \" restore 250", "pill result Hello world."]);
    assert_eq!(d.session.state(), State::Idle);
    assert_eq!(run(&mut d, &["last"]), Ok("Hello world.".into()));
    assert_eq!(clips(&cache), 0, "the clip is gone");
    // Armed once, however many dictations.
    assert_eq!(hold(&mut d, 1000), Ok("dictation: transcribing".into()));
    settle(&mut d);
    assert_eq!(ARMED.get(), 1);
}

#[test]
fn no_log_line_carries_the_transcript() {
    let (mut d, _cache) = setup("privacy", "insert = \"type\"\ntrailing_space = false");
    hold(&mut d, 1000).unwrap();
    settle(&mut d);
    let seen = seen();
    assert!(seen.contains(&"type \"Hello world.\"".to_string()), "{seen:?}");
    assert!(seen.iter().filter(|l| l.starts_with("log ")).all(|l| !l.contains("Hello")), "{seen:?}");
}

#[test]
fn insertion_waits_for_the_modifiers_to_come_up() {
    let (mut d, _cache) = setup("modifiers", "");
    hold(&mut d, 1000).unwrap();
    FLAGS.set(MODIFIERS & 0x10_0000);
    settle(&mut d);
    assert_eq!(LATER.get(), 1);
    assert_eq!(d.session.state(), State::Inserting);
    wake(&mut d);
    assert_eq!(LATER.get(), 2);
    assert!(seen().iter().all(|l| !l.starts_with("paste")));
    FLAGS.set(0x100);
    wake(&mut d);
    assert_eq!(seen(), ["paste \"Hello world. \" restore 250", "pill result Hello world."]);
    assert_eq!(d.session.state(), State::Idle);
    // A start while it waits is refused.
    hold(&mut d, 1000).unwrap();
    FLAGS.set(MODIFIERS);
    settle(&mut d);
    assert_eq!(run(&mut d, &["start"]), Err("dictation: already inserting".into()));
    // Held past the cap: it goes in anyway (cmd+V carries explicit flags).
    CLOCK.set(CLOCK.get() + 500);
    wake(&mut d);
    assert!(seen().contains(&"paste \"Hello world. \" restore 250".to_string()));
}

#[test]
fn secure_input_and_a_new_frontmost_app_keep_the_text() {
    let (mut d, _cache) = setup("guards", "");
    hold(&mut d, 1000).unwrap();
    SECURE.set(true);
    settle(&mut d);
    let seen1 = seen();
    assert!(seen1.contains(&"pill error Secure input is on: kept, see flick dictation last".to_string()), "{seen1:?}");
    assert!(seen1.iter().all(|l| !l.starts_with("paste")));
    assert_eq!(run(&mut d, &["last"]), Ok("Hello world.".into()));
    assert!(run(&mut d, &["status"]).unwrap().ends_with("last error: not inserted: Secure input is on"));
    SECURE.set(false);
    hold(&mut d, 1000).unwrap();
    FRONT.set(Some(2));
    settle(&mut d);
    assert!(seen().contains(&"pill error Another app is in front: kept, see flick dictation last".to_string()));
    assert_eq!(d.session.state(), State::Idle);
}

#[test]
fn short_holds_and_stray_stops_do_nothing() {
    let (mut d, _cache) = setup("short", "");
    assert_eq!(hold(&mut d, 299), Ok("dictation: too short; nothing recorded".into()));
    assert_eq!(seen(), ["pill recording", "pill hide", "log dictation: a 299 ms hold, too short; discarded"]);
    assert_eq!(run(&mut d, &["stop"]), Ok("dictation: not recording".into()));
    assert_eq!(run(&mut d, &["cancel"]), Err("dictation: nothing to cancel".into()));
    assert_eq!(run(&mut d, &["last"]), Err("dictation: nothing dictated yet".into()));
    assert!(seen().is_empty());
}

#[test]
fn cancel_while_recording_or_transcribing() {
    let (mut d, cache) = setup("cancel", "model = \"/models/slow.bin\"");
    run(&mut d, &["start"]).unwrap();
    assert_eq!(run(&mut d, &["start"]), Err("dictation: already recording".into()));
    assert_eq!(run(&mut d, &["cancel"]), Ok("dictation: cancelled".into()));
    // The chord's key-up after Esc.
    assert_eq!(run(&mut d, &["stop"]), Ok("dictation: not recording".into()));
    assert_eq!(seen(), ["pill recording", "pill hide"]);
    // Esc on the pill while the (slow) engine runs.
    hold(&mut d, 1000).unwrap();
    wait_for(|| clips(&cache) == 1);
    ESC.set(true);
    wake(&mut d);
    assert_eq!(d.session.state(), State::Idle);
    settle(&mut d);
    assert_eq!(seen(), ["pill recording", "pill transcribing", "pill hide"]);
    assert_eq!(clips(&cache), 0, "a cancelled run deletes its clip");
    assert_eq!(run(&mut d, &["stop"]), Ok("dictation: not recording".into()));
}

#[test]
fn the_microphone_must_be_allowed() {
    let (mut d, _cache) = setup("mic", "");
    MIC.set(Mic::Denied);
    let err = run(&mut d, &["start"]).unwrap_err();
    assert_eq!(err, "dictation: microphone denied (System Settings > Privacy & Security > Microphone)");
    assert_eq!(seen(), ["pill error Microphone denied", format!("log {err}").as_str()]);
    let status = run(&mut d, &["status"]).unwrap();
    assert!(status.contains("microphone: denied"), "{status}");
    assert!(status.ends_with("last error: microphone denied (System Settings > Privacy & Security > Microphone)"));
    MIC.set(Mic::NotDetermined);
    let err = run(&mut d, &["start"]).unwrap_err();
    assert_eq!(err, "dictation: asked for microphone access; hold again once it is allowed");
    assert_eq!((ASKED.get(), d.session.state()), (1, State::Idle));
    assert_eq!(seen()[0], "pill error Allow the microphone, then hold again");
    MIC.set(Mic::Restricted);
    assert!(run(&mut d, &["start"]).unwrap_err().contains("microphone denied"));
    // A clean start clears the last error.
    MIC.set(Mic::Unchecked);
    run(&mut d, &["start"]).unwrap();
    assert!(!run(&mut d, &["status"]).unwrap().contains("last error"));
    run(&mut d, &["cancel"]).unwrap();
}

#[test]
fn missing_programs_and_models_stop_the_start() {
    let cases = [
        ("recorder = \"/nope/rec\"", "dictation: recorder /nope/rec is missing (see flick dictation status)", "Recorder"),
        ("whisper_bin = \"/models/stt\"", "dictation: engine /models/stt is missing (see flick dictation status)", "Engine"),
        ("model = \"/nope/m.bin\"", "dictation: model /nope/m.bin is missing (see flick dictation status)", "Model"),
    ];
    for (extra, want, what) in cases {
        let (mut d, _cache) = setup("missing", extra);
        assert_eq!(run(&mut d, &["start"]), Err(want.into()));
        assert_eq!(seen()[0], format!("pill error {what} missing: see flick dictation status"));
        assert_eq!(d.session.state(), State::Idle);
    }
    let (mut d, _cache) = setup("spawn", "recorder = \"/fake/rec-gone\"");
    let err = run(&mut d, &["start"]).unwrap_err();
    assert_eq!(err, "dictation: /fake/rec-gone: No such file or directory");
    assert_eq!(seen()[0], "pill error The recorder did not start");
    assert_eq!(d.session.state(), State::Idle);
}

#[test]
fn off_without_a_table_and_inert() {
    let mut d = configured("").unwrap();
    assert_eq!(run(&mut d, &["start"]), Err("dictation: off: set a key in [dictation] to turn it on".into()));
    assert_eq!(run(&mut d, &["stop"]), Ok("dictation: not recording".into()));
    seen();
    event(&mut d, Event::Started);
    wake(&mut d);
    event(&mut d, Event::ModuleChanged { module: "clip" });
    assert!(seen().is_empty());
}

#[test]
fn started_wipes_leftover_clips() {
    let (mut d, cache) = setup("wipe", "");
    event(&mut d, Event::Started);
    fs::create_dir_all(&cache.0).unwrap();
    fs::write(cache.0.join("1-1.wav"), b"RIFF").unwrap();
    event(&mut d, Event::Started);
    assert_eq!(seen(), ["log dictation: removed 1 leftover clips"]);
    assert_eq!(clips(&cache), 0);
}

#[test]
fn silence_and_engine_errors_insert_nothing() {
    let (mut d, _cache) = setup("zeros", "recorder = \"/fake/rec-quiet\"");
    hold(&mut d, 1000).unwrap();
    settle(&mut d);
    let seen1 = seen();
    assert!(seen1.contains(&"pill error No audio: microphone denied or muted?".to_string()), "{seen1:?}");
    let (mut d, _cache) = setup("noise", "model = \"/models/noise.bin\"");
    hold(&mut d, 1000).unwrap();
    settle(&mut d);
    assert_eq!(seen()[2..], ["log dictation: 0.5 s of audio, no speech", "pill result No speech heard"]);
    let (mut d, _cache) = setup("fail", "model = \"/models/fail.bin\"");
    hold(&mut d, 1000).unwrap();
    settle(&mut d);
    assert_eq!(seen()[2..], ["pill error stt failed", "log dictation: stt failed"]);
    assert!(run(&mut d, &["status"]).unwrap().ends_with("last error: stt failed"));
    assert_eq!(d.session.state(), State::Idle);
    assert!(run(&mut d, &["last"]).is_err());
}

#[test]
fn the_recorder_ending_by_itself() {
    // max_seconds: the recording stops and is transcribed.
    let (mut d, _cache) = setup("max", "recorder = \"/fake/rec-long\"\nmax_seconds = 1");
    run(&mut d, &["start"]).unwrap();
    CLOCK.set(1000);
    wait_for(|| d.live_ended());
    wake(&mut d);
    assert_eq!(d.session.state(), State::Transcribing);
    settle(&mut d);
    let seen1 = seen();
    assert!(seen1.iter().any(|l| l.starts_with("log dictation: 1.0 s of audio")), "{seen1:?}");
    assert!(seen1.contains(&"paste \"Hello world. \" restore 250".to_string()));
    // The recorder exiting while recording is an error.
    let (mut d, _cache) = setup("dies", "recorder = \"/fake/rec-dies\"");
    run(&mut d, &["start"]).unwrap();
    wait_for(|| d.live_ended());
    wake(&mut d);
    assert_eq!(seen(), ["pill recording", "pill hide", "pill error The recorder stopped (see flick dictation status)", "log dictation: the recorder exited while recording"]);
    assert_eq!(d.session.state(), State::Idle);
}

#[test]
fn a_late_result_from_a_cancelled_run_is_dropped() {
    let (mut d, _cache) = setup("stale", "");
    hold(&mut d, 1000).unwrap();
    run(&mut d, &["cancel"]).unwrap();
    settle(&mut d);
    assert_eq!(seen(), ["pill recording", "pill transcribing", "pill hide"]);
    assert_eq!(d.session.state(), State::Idle);
    assert!(run(&mut d, &["last"]).is_err());
}
