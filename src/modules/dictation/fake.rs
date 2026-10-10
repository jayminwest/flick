//! Scripted stand-ins for the recorder and the engine (tests only). What a fake does depends
//! only on its argv, never on shared state, so tests run in parallel.
//!
//! Recorders (`argv[0]`): `/fake/rec` (0.5 s of tone, then waits for SIGTERM),
//! `/fake/rec-long` (2 s of tone), `/fake/rec-quiet` (0.5 s of digital silence),
//! `/fake/rec-dies` (exits at once, failing), `/fake/rec-stubborn` (ignores SIGTERM).
//! Engine `/fake/stt`: needs its `.wav` argument to exist as a 0600 file, then prints
//! " Hello world.\n"; an argument containing `slow` makes it hang until killed, `fail`
//! makes it fail, `noise` makes it print only `[BLANK_AUDIO]`.

use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use super::audio::SAMPLE_RATE;
use super::proc::{Child, Spawned};

/// The transcript the fake engine prints.
pub const SPOKEN: &str = " Hello world.\n";

#[derive(Default)]
struct State {
    data: VecDeque<u8>,
    /// Stdout still open.
    open: bool,
    exit: Option<bool>,
    /// SIGTERM does nothing.
    stubborn: bool,
}

#[derive(Clone, Default)]
struct Script(Arc<(Mutex<State>, Condvar)>);

impl Script {
    fn spawned(data: Vec<u8>, open: bool, exit: Option<bool>, stubborn: bool) -> Spawned {
        let state = State { data: data.into(), open, exit, stubborn };
        let script = Script(Arc::new((Mutex::new(state), Condvar::new())));
        Spawned { stdout: Box::new(script.clone()), child: Box::new(script) }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.0.0.lock().unwrap_or_else(PoisonError::into_inner);
        let out = f(&mut state);
        self.0.1.notify_all();
        out
    }
}

impl Read for Script {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let (lock, wake) = &*self.0;
        let mut s = lock.lock().unwrap_or_else(PoisonError::into_inner);
        while s.data.is_empty() && s.open {
            s = wake.wait(s).unwrap_or_else(PoisonError::into_inner);
        }
        let n = buf.len().min(s.data.len()).min(1600);
        for (i, b) in s.data.drain(..n).enumerate() {
            buf[i] = b;
        }
        Ok(n)
    }
}

impl Child for Script {
    fn terminate(&mut self) {
        self.with(|s| {
            if !s.stubborn {
                s.open = false;
                s.exit.get_or_insert(true);
            }
        });
    }

    fn kill(&mut self) {
        self.with(|s| {
            s.open = false;
            s.exit.get_or_insert(false);
        });
    }

    fn try_wait(&mut self) -> Result<Option<bool>, String> {
        Ok(self.with(|s| if s.open { None } else { s.exit }))
    }
}

/// `secs` of 16-bit PCM at `amplitude` (a square wave; 0 is silence).
pub fn pcm(secs: f32, amplitude: i16) -> Vec<u8> {
    let n = (SAMPLE_RATE as f32 * secs) as usize;
    (0..n).flat_map(|i| (if i % 40 < 20 { amplitude } else { -amplitude }).to_le_bytes()).collect()
}

pub fn spawn(argv: &[String]) -> Result<Spawned, String> {
    let program = argv.first().map_or("", String::as_str);
    let has = |word: &str| argv.iter().skip(1).any(|a| a.contains(word));
    Ok(match program {
        "/fake/rec" => Script::spawned(pcm(0.5, 8000), true, None, false),
        "/fake/rec-long" => Script::spawned(pcm(2.0, 8000), true, None, false),
        "/fake/rec-quiet" => Script::spawned(pcm(0.5, 0), true, None, false),
        "/fake/rec-dies" => Script::spawned(vec![], false, Some(false), false),
        "/fake/rec-stubborn" => Script::spawned(pcm(0.5, 8000), true, None, true),
        "/fake/stt" => {
            let wav = argv.iter().find(|a| std::path::Path::new(a).extension().is_some_and(|x| x == "wav")).ok_or("stt: no wav")?;
            let mode = std::fs::metadata(wav).map_err(|e| format!("stt: {wav}: {e}"))?.permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the clip is private");
            if has("slow") {
                Script::spawned(vec![], true, None, true)
            } else if has("fail") {
                Script::spawned(vec![], false, Some(false), false)
            } else if has("noise") {
                Script::spawned(b"[BLANK_AUDIO]\n".to_vec(), false, Some(true), false)
            } else {
                Script::spawned(SPOKEN.as_bytes().to_vec(), false, Some(true), false)
            }
        }
        _ => return Err(format!("{program}: No such file or directory")),
    })
}
