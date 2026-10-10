//! One transcription on a worker thread (the rebuild module's pattern): stop the recorder
//! and drain it, gate silence, write the clip, run the engine within its budget, delete the
//! clip, clean the text, and leave the result in the inbox for the main thread, which hears
//! of it through `ModuleChanged { module: "dictation" }`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::engine::{self, Fail};
use super::proc::{self, Spawn};
use super::recorder::{BYTES_PER_SEC, GRACE, Recorder};
use super::{audio, clip, transcript};

/// What to transcribe, and how.
pub struct Job {
    pub epoch: u64,
    pub recorder: Recorder,
    /// Where the clip goes (see `clip`).
    pub wav: PathBuf,
    /// The engine's argv, naming `wav`.
    pub argv: Vec<String>,
    pub silence_rms: f32,
    pub trailing_space: bool,
    pub budget: Duration,
    /// Set by `dictation cancel`: the engine is killed and the result is `Cancelled`.
    pub cancel: Arc<AtomicBool>,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Cleaned text, ready to insert.
    Text(String),
    /// Nothing to insert: the clip was silent (`zeros`: not one non-zero sample, which is
    /// what a denied or muted microphone gives), or the engine heard no speech.
    Silent { zeros: bool },
    Failed(String),
    Cancelled,
}

/// A finished job. Holds the text, so it is never logged as a whole.
#[derive(Debug)]
pub struct Done {
    pub epoch: u64,
    pub outcome: Outcome,
    /// Seconds of audio, and seconds the engine took (0 when it did not run).
    pub audio_secs: f32,
    pub engine_secs: f32,
}

/// Results waiting for the main thread.
pub type Inbox = Arc<Mutex<Vec<Done>>>;

/// Take every waiting result.
pub fn drain(inbox: &Inbox) -> Vec<Done> {
    std::mem::take(&mut *inbox.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Run `job` on a thread; its result goes to `inbox`, then `notify` runs.
pub fn start(job: Job, spawn: Spawn, inbox: &Inbox, notify: fn()) {
    let inbox = inbox.clone();
    let _ = thread::Builder::new().name("flick-dictation-stt".into()).spawn(move || {
        let done = run(job, spawn);
        inbox.lock().unwrap_or_else(PoisonError::into_inner).push(done);
        notify();
    });
}

/// The job, start to end, on this thread.
pub fn run(job: Job, spawn: Spawn) -> Done {
    let take = job.recorder.finish(GRACE);
    let samples = audio::samples(&take.pcm);
    let audio_secs = take.pcm.len() as f32 / BYTES_PER_SEC as f32;
    let done = |outcome, engine_secs| Done { epoch: job.epoch, outcome, audio_secs, engine_secs };
    if job.cancel.load(Ordering::Acquire) {
        return done(Outcome::Cancelled, 0.0);
    }
    if take.pcm.is_empty() && !take.exit_ok {
        return done(Outcome::Failed("The recorder failed (see flick dictation status)".into()), 0.0);
    }
    if audio::is_silent(&samples, job.silence_rms) {
        return done(Outcome::Silent { zeros: samples.iter().all(|&s| s == 0) }, 0.0);
    }
    let clip = match clip::write(&job.wav, &audio::wav(&samples)) {
        Ok(clip) => clip,
        Err(e) => return done(Outcome::Failed(format!("Cannot write the clip: {e}")), 0.0),
    };
    let since = Instant::now();
    let result = engine::run(spawn, &job.argv, job.budget, &job.cancel);
    drop(clip);
    let outcome = match result {
        Ok(raw) => transcript::clean(&raw, job.trailing_space).map_or(Outcome::Silent { zeros: false }, Outcome::Text),
        Err(Fail::Cancelled) => Outcome::Cancelled,
        Err(Fail::TimedOut) => {
            Outcome::Failed(format!("{} took over {} s", proc::name(&job.argv), job.budget.as_secs()))
        }
        Err(Fail::Error(e)) => Outcome::Failed(e),
    };
    done(outcome, since.elapsed().as_secs_f32())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::modules::dictation::{fake, recorder};

    /// A job recording with `rec` into a directory of the test's own; the engine gets
    /// `extra` as its last argument.
    fn job(test: &str, rec: &str, extra: &str) -> Job {
        let dir = std::env::temp_dir().join(format!("flick-dictation-worker-{test}-{}", std::process::id()));
        let wav = dir.join(clip::name(1));
        let argv = vec!["/fake/stt".into(), "-f".into(), wav.display().to_string(), extra.into()];
        let recorder = Recorder::start(fake::spawn, &recorder::argv(rec), BYTES_PER_SEC * 10, || {}).unwrap();
        Job {
            epoch: 1,
            recorder,
            wav,
            argv,
            silence_rms: 0.01,
            trailing_space: true,
            budget: Duration::from_secs(5),
            cancel: Arc::default(),
        }
    }

    /// Run `job` and check that it left no clip behind.
    fn outcome(job: Job) -> Outcome {
        let dir = job.wav.parent().unwrap().to_path_buf();
        let done = run(job, fake::spawn);
        assert_eq!(std::fs::read_dir(&dir).map_or(0, Iterator::count), 0, "no clip left");
        let _ = std::fs::remove_dir(&dir);
        done.outcome
    }

    #[test]
    fn transcribes_speech_and_deletes_the_clip() {
        let j = job("speech", "/fake/rec", "-np");
        let dir = j.wav.parent().unwrap().to_path_buf();
        let done = run(j, fake::spawn);
        assert_eq!(done.outcome, Outcome::Text("Hello world. ".into()));
        assert!((done.audio_secs - 0.5).abs() < 1e-3);
        assert!(done.engine_secs >= 0.0);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn silence_never_reaches_the_engine() {
        assert_eq!(outcome(job("zeros", "/fake/rec-quiet", "")), Outcome::Silent { zeros: true });
        assert_eq!(outcome(job("noise", "/fake/rec", "noise")), Outcome::Silent { zeros: false });
    }

    #[test]
    fn failures_and_cancels_delete_the_clip_too() {
        assert_eq!(outcome(job("fail", "/fake/rec", "fail")), Outcome::Failed("stt failed".into()));
        let mut slow = job("slow", "/fake/rec", "slow");
        slow.budget = Duration::from_millis(20);
        assert_eq!(outcome(slow), Outcome::Failed("stt took over 0 s".into()));
        let dead = job("dead", "/fake/rec-dies", "");
        assert_eq!(outcome(dead), Outcome::Failed("The recorder failed (see flick dictation status)".into()));
        let early = job("early", "/fake/rec", "");
        early.cancel.store(true, Ordering::Release);
        assert_eq!(outcome(early), Outcome::Cancelled);
        let late = job("late", "/fake/rec", "slow");
        let cancel = late.cancel.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel.store(true, Ordering::Release);
        });
        assert_eq!(outcome(late), Outcome::Cancelled);
        canceller.join().unwrap();
        let mut nowhere = job("nowhere", "/fake/rec", "");
        nowhere.wav = Path::new("/dev/null/1.wav").into();
        assert!(matches!(run(nowhere, fake::spawn).outcome, Outcome::Failed(e) if e.starts_with("Cannot write the clip")));
    }

    #[test]
    fn start_posts_to_the_inbox() {
        let inbox = Inbox::default();
        start(job("inbox", "/fake/rec", ""), fake::spawn, &inbox, || {});
        let since = Instant::now();
        let done = loop {
            if let Some(done) = drain(&inbox).pop() {
                break done;
            }
            assert!(since.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(2));
        };
        assert_eq!((done.epoch, done.outcome), (1, Outcome::Text("Hello world. ".into())));
        let _ = std::fs::remove_dir(std::env::temp_dir().join(format!("flick-dictation-worker-inbox-{}", std::process::id())));
    }
}
