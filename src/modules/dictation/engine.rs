//! The speech-to-text program: its argv for each engine, and one run of it with a time
//! budget and a cancel flag. The transcript is its stdout; stderr is closed, so nothing it
//! prints about the audio ends up anywhere.

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::proc::{self, Spawn, Spawned};
use super::settings::{Engine, Settings, expand};

/// How often a run looks at the engine, the budget and the cancel flag.
const POLL: Duration = Duration::from_millis(5);

/// Why a run gave no transcript.
#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
    Cancelled,
    /// Over budget, killed.
    TimedOut,
    /// It could not start, or exited with an error.
    Error(String),
}

/// The engine's argv for the clip at `wav`.
pub fn argv(s: &Settings, home: &Path, wav: &Path) -> Vec<String> {
    let path = |p: &Path| p.display().to_string();
    let bin = path(&s.engine_bin(home));
    let (model, wav) = (path(&expand(&s.model, home)), path(wav));
    match s.engine {
        Engine::Whisper => {
            let mut a = vec![bin, "-m".into(), model, "-f".into(), wav, "-l".into(), s.language.clone(), "-nt".into(), "-np".into()];
            if !s.prompt.trim().is_empty() {
                a.extend(["--prompt".into(), s.prompt.clone()]);
            }
            if !s.vad_model.trim().is_empty() {
                a.extend(["--vad".into(), "-vm".into(), path(&expand(&s.vad_model, home))]);
            }
            a
        }
        Engine::Parakeet => vec![bin, "-m".into(), model, "-f".into(), wav, "-np".into()],
        Engine::Command => {
            let fill = |arg: &String| {
                arg.replace("{wav}", &wav)
                    .replace("{model}", &model)
                    .replace("{language}", &s.language)
                    .replace("{prompt}", &s.prompt)
            };
            std::iter::once(bin).chain(s.command.iter().skip(1).map(fill)).collect()
        }
    }
}

/// Run `argv` to completion and return its stdout, killing it when `budget` runs out or
/// `cancel` is set. Blocks: call it on a worker thread.
pub fn run(spawn: Spawn, argv: &[String], budget: Duration, cancel: &AtomicBool) -> Result<String, Fail> {
    let name = proc::name(argv).to_string();
    let Spawned { mut stdout, mut child } = spawn(argv).map_err(Fail::Error)?;
    // Read on a thread of its own, so a full pipe never stalls the engine.
    let reader = thread::Builder::new().name("flick-dictation-out".into()).spawn(move || {
        let mut out = vec![];
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + budget;
    let exited = loop {
        let fail = match child.try_wait() {
            Ok(Some(ok)) => break ok,
            Err(e) => Fail::Error(format!("{name}: {e}")),
            Ok(None) if cancel.load(Ordering::Acquire) => Fail::Cancelled,
            Ok(None) if Instant::now() >= deadline => Fail::TimedOut,
            Ok(None) => {
                thread::sleep(POLL);
                continue;
            }
        };
        child.kill();
        return Err(fail);
    };
    let out = reader.map_err(|e| Fail::Error(format!("{name}: {e}")))?.join().unwrap_or_default();
    if !exited {
        return Err(Fail::Error(format!("{name} failed")));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::parse;
    use crate::modules::dictation::{clip, fake};

    fn settings(text: &str) -> Settings {
        parse(&format!("[dictation]\n{text}")).unwrap().section("dictation").unwrap().unwrap().get().unwrap()
    }

    fn argv_of(text: &str) -> String {
        argv(&settings(text), Path::new("/Users/u"), Path::new("/c/1.wav")).join(" ")
    }

    #[test]
    fn builds_each_engines_argv() {
        let model = "/Users/u/Library/Application Support/Flick/models/ggml-large-v3-turbo-q5_0.bin";
        assert_eq!(argv_of(""), format!("/opt/homebrew/bin/whisper-cli -m {model} -f /c/1.wav -l en -nt -np"));
        assert_eq!(
            argv_of("prompt = \"Flick, KOTA\"\nvad_model = \"~/v.bin\"\nlanguage = \"auto\"\nmodel = \"/m.bin\""),
            "/opt/homebrew/bin/whisper-cli -m /m.bin -f /c/1.wav -l auto -nt -np --prompt Flick, KOTA --vad -vm /Users/u/v.bin"
        );
        assert_eq!(argv_of("engine = \"parakeet\"\nmodel = \"~/p.bin\""), "/opt/homebrew/bin/parakeet-cli -m /Users/u/p.bin -f /c/1.wav -np");
        let command = "engine = \"command\"\nmodel = \"/m\"\nprompt = \"p\"\n\
                       command = [\"~/stt\", \"--in={wav}\", \"{model}\", \"{language}\", \"{prompt}\"]";
        assert_eq!(argv_of(command), "/Users/u/stt --in=/c/1.wav /m en p");
    }

    /// A clip for the fake engine to find, in a directory of the test's own.
    fn clip(test: &str) -> (clip::Clip, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("flick-dictation-engine-{test}-{}", std::process::id()));
        let wav = dir.join("1.wav");
        let clip = clip::write(&wav, b"RIFF").unwrap();
        (clip, vec!["/fake/stt".into(), "-f".into(), wav.display().to_string()])
    }

    fn run_with(test: &str, extra: &str, budget: Duration, cancel: bool) -> Result<String, Fail> {
        let (clip, mut argv) = clip(test);
        argv.push(extra.into());
        let out = run(fake::spawn, &argv, budget, &AtomicBool::new(cancel));
        let dir: PathBuf = clip.path().parent().unwrap().into();
        drop(clip);
        let _ = std::fs::remove_dir(dir);
        out
    }

    #[test]
    fn returns_stdout() {
        assert_eq!(run_with("ok", "-np", Duration::from_secs(5), false).unwrap(), fake::SPOKEN);
    }

    #[test]
    fn errors_time_outs_and_cancels() {
        assert_eq!(run_with("fail", "fail", Duration::from_secs(5), false), Err(Fail::Error("stt failed".into())));
        let since = Instant::now();
        assert_eq!(run_with("slow", "slow", Duration::from_millis(30), false), Err(Fail::TimedOut));
        assert!(since.elapsed() < Duration::from_secs(2));
        assert_eq!(run_with("cancel", "slow", Duration::from_secs(5), true), Err(Fail::Cancelled));
        let missing = run(fake::spawn, &["/no/stt".into()], Duration::from_secs(1), &AtomicBool::new(false));
        assert_eq!(missing, Err(Fail::Error("/no/stt: No such file or directory".into())));
    }
}
