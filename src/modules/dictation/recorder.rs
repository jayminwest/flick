//! The recorder: `rec` writing raw 16 kHz mono PCM to its stdout, read by a named thread
//! that keeps the audio in memory (no file while recording), publishes the input level for
//! the pill and stops `rec` at `max_seconds`. `finish` (on a worker thread, never the main
//! one) stops it with SIGTERM, drains what it flushed and reaps it.

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::audio::{self, SAMPLE_RATE};
use super::proc::{self, Shared, Spawn, Spawned};

/// Bytes per second of 16-bit mono PCM.
pub const BYTES_PER_SEC: usize = SAMPLE_RATE as usize * 2;
/// How long a stopped recorder gets to exit before it is killed.
pub const GRACE: Duration = Duration::from_secs(1);
/// How often `finish` looks at the reader and the child.
const POLL: Duration = Duration::from_millis(5);

/// The argv for `recorder`: raw signed 16-bit little-endian mono 16 kHz PCM on stdout, in
/// 50 ms buffers (sox's default 8 KiB would update the level only every 256 ms).
pub fn argv(recorder: &str) -> Vec<String> {
    let args = ["--buffer", "1600", "-q", "-t", "raw", "-r", "16000", "-e", "signed", "-b", "16", "-c", "1", "-"];
    std::iter::once(recorder.to_string()).chain(args.map(String::from)).collect()
}

/// What a finished recording left.
pub struct Take {
    /// Raw PCM, at most `max_seconds` of it.
    pub pcm: Vec<u8>,
    /// The recorder exited cleanly (or was stopped by us); false when it failed.
    pub exit_ok: bool,
}

pub struct Recorder {
    child: Arc<Shared>,
    /// The meter value (0..=1, see `audio::meter`) of the last buffer, as `f32` bits.
    level: Arc<AtomicU32>,
    /// `max_seconds` reached: the reader stopped `rec` by itself.
    full: Arc<AtomicBool>,
    reader: JoinHandle<Vec<u8>>,
}

impl Recorder {
    /// Start `argv` and its reader thread; `notify` runs on that thread when the recording
    /// ends by itself (`max_bytes` reached, or the recorder exited) and after a stop.
    pub fn start(spawn: Spawn, argv: &[String], max_bytes: usize, notify: fn()) -> Result<Recorder, String> {
        let Spawned { stdout, child } = spawn(argv)?;
        let child: Arc<Shared> = Arc::new(Mutex::new(child));
        let (level, full) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicBool::new(false)));
        let shared = (child.clone(), level.clone(), full.clone());
        let reader = thread::Builder::new()
            .name("flick-dictation-rec".into())
            .spawn(move || read(stdout, max_bytes, &shared.0, &shared.1, &shared.2, notify));
        match reader {
            Ok(reader) => Ok(Recorder { child, level, full, reader }),
            Err(e) => {
                proc::lock(&child).kill();
                Err(format!("dictation: cannot start the reader: {e}"))
            }
        }
    }

    /// The level the reader publishes, for the pill.
    pub fn level(&self) -> Arc<AtomicU32> {
        self.level.clone()
    }

    /// The recording ended without a stop: full, or the recorder exited.
    pub fn ended(&self) -> bool {
        self.full() || self.reader.is_finished()
    }

    pub fn full(&self) -> bool {
        self.full.load(Ordering::Acquire)
    }

    /// Stop the recorder (SIGTERM, then SIGKILL after `grace`), wait for the reader to
    /// drain it, reap it, and return the audio. Blocks: call it on a worker thread.
    pub fn finish(self, grace: Duration) -> Take {
        proc::lock(&self.child).terminate();
        let since = Instant::now();
        while !self.reader.is_finished() {
            if since.elapsed() >= grace {
                proc::lock(&self.child).kill();
                break;
            }
            thread::sleep(POLL);
        }
        let pcm = self.reader.join().unwrap_or_default();
        let exit_ok = reap(&self.child, grace).unwrap_or(false);
        Take { pcm, exit_ok }
    }

    /// Stop the recorder and drop the audio, on a thread of its own.
    pub fn discard(self) {
        let _ = thread::Builder::new().name("flick-dictation-rec".into()).spawn(move || self.finish(GRACE));
    }
}

/// Wait up to `grace` for the child to exit, then kill it. `Some(success)`.
fn reap(child: &Shared, grace: Duration) -> Option<bool> {
    let since = Instant::now();
    loop {
        if let Ok(Some(ok)) = proc::lock(child).try_wait() {
            return Some(ok);
        }
        if since.elapsed() >= grace {
            let mut child = proc::lock(child);
            child.kill();
            return child.try_wait().ok().flatten();
        }
        thread::sleep(POLL);
    }
}

/// The reader thread: keep at most `max_bytes` of PCM, publish each buffer's level, stop the
/// recorder once full, and read on to EOF.
fn read(mut out: Box<dyn Read + Send>, max_bytes: usize, child: &Shared, level: &AtomicU32, full: &AtomicBool, notify: fn()) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(max_bytes.min(BYTES_PER_SEC * 30));
    let mut buf = [0u8; 3200];
    loop {
        let n = match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let room = max_bytes.saturating_sub(pcm.len());
        pcm.extend_from_slice(&buf[..n.min(room)]);
        let meter = audio::meter(audio::rms(&audio::samples(&buf[..n])));
        level.store(meter.to_bits(), Ordering::Relaxed);
        if pcm.len() >= max_bytes && !full.swap(true, Ordering::AcqRel) {
            proc::lock(child).terminate();
            notify();
        }
    }
    level.store(0, Ordering::Relaxed);
    if !full.load(Ordering::Acquire) {
        notify();
    }
    pcm
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::dictation::fake;

    fn argv(program: &str) -> Vec<String> {
        super::argv(program)
    }

    fn wait_until(f: impl Fn() -> bool) {
        let since = Instant::now();
        while !f() {
            assert!(since.elapsed() < Duration::from_secs(5), "timed out");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn argv_asks_for_raw_16k_mono() {
        let a = argv("/opt/homebrew/bin/rec");
        assert_eq!(a.join(" "), "/opt/homebrew/bin/rec --buffer 1600 -q -t raw -r 16000 -e signed -b 16 -c 1 -");
    }

    #[test]
    fn records_until_stopped_and_publishes_the_level() {
        let r = Recorder::start(fake::spawn, &argv("/fake/rec"), BYTES_PER_SEC * 10, || {}).unwrap();
        let level = r.level();
        wait_until(|| level.load(Ordering::Relaxed) != 0);
        assert!(f32::from_bits(level.load(Ordering::Relaxed)) > 0.5);
        assert!(!r.ended() && !r.full());
        let take = r.finish(GRACE);
        assert_eq!(take.pcm, fake::pcm(0.5, 8000));
        assert!(take.exit_ok);
        assert_eq!(level.load(Ordering::Relaxed), 0, "the meter falls back once it ends");
    }

    #[test]
    fn stops_itself_at_max_seconds() {
        let r = Recorder::start(fake::spawn, &argv("/fake/rec-long"), BYTES_PER_SEC, || {}).unwrap();
        wait_until(|| r.ended());
        assert!(r.full());
        let take = r.finish(GRACE);
        assert_eq!(take.pcm.len(), BYTES_PER_SEC);
        assert!(take.exit_ok);
    }

    #[test]
    fn a_recorder_that_exits_ends_the_recording() {
        let r = Recorder::start(fake::spawn, &argv("/fake/rec-dies"), BYTES_PER_SEC, || {}).unwrap();
        wait_until(|| r.ended());
        assert!(!r.full());
        let take = r.finish(Duration::from_millis(50));
        assert!(take.pcm.is_empty());
        assert!(!take.exit_ok);
    }

    #[test]
    fn a_recorder_that_ignores_sigterm_is_killed() {
        let r = Recorder::start(fake::spawn, &argv("/fake/rec-stubborn"), BYTES_PER_SEC * 10, || {}).unwrap();
        let level = r.level();
        wait_until(|| level.load(Ordering::Relaxed) != 0);
        let since = Instant::now();
        let take = r.finish(Duration::from_millis(50));
        assert!(since.elapsed() < Duration::from_secs(2));
        assert_eq!(take.pcm.len(), fake::pcm(0.5, 8000).len());
        assert!(!take.exit_ok, "killed");
    }

    #[test]
    fn discard_and_spawn_errors() {
        Recorder::start(fake::spawn, &argv("/fake/rec"), BYTES_PER_SEC, || {}).unwrap().discard();
        let err = Recorder::start(fake::spawn, &argv("/no/rec"), BYTES_PER_SEC, || {}).err().unwrap();
        assert_eq!(err, "/no/rec: No such file or directory");
    }
}
