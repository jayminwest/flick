//! Table `[dictation]`: the engine, the recorder and how text goes in. Every key has a
//! default, so a config without the table loads unchanged; the module stays off until the
//! table sets at least one key.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The speech-to-text program.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// whisper.cpp's `whisper-cli`.
    #[default]
    Whisper,
    /// whisper.cpp's `parakeet-cli`.
    Parakeet,
    /// `command`, an argv template; the transcript is its stdout.
    Command,
}

impl Engine {
    pub fn name(self) -> &'static str {
        match self {
            Engine::Whisper => "whisper",
            Engine::Parakeet => "parakeet",
            Engine::Command => "command",
        }
    }
}

/// How the transcript reaches the focused field.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Insert {
    /// Through the pasteboard and cmd+V; the pasteboard is restored after `restore_ms`.
    #[default]
    Paste,
    /// Typed as unicode key events; never touches the pasteboard.
    Type,
}

impl Insert {
    pub fn name(self) -> &'static str {
        match self {
            Insert::Paste => "paste",
            Insert::Type => "type",
        }
    }
}

/// Longest recording `max_seconds` may allow (16 kHz mono 16-bit: about 19 MB).
const MAX_SECONDS: u32 = 600;
/// Upper bound for the millisecond settings.
const MAX_MS: u32 = 5000;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub engine: Engine,
    /// `whisper-cli`, for `engine = "whisper"`.
    pub whisper_bin: String,
    /// `parakeet-cli`, for `engine = "parakeet"`.
    pub parakeet_bin: String,
    /// `engine = "command"`: program and arguments; `{wav}` (required), `{model}`,
    /// `{language}` and `{prompt}` are replaced. The transcript is its stdout.
    pub command: Vec<String>,
    /// The model file (`~/` allowed). Flick never downloads it.
    pub model: String,
    /// A language code such as `en`, or `auto`.
    pub language: String,
    /// Initial prompt for whisper (names, jargon); empty for none.
    pub prompt: String,
    /// Silero VAD model for `whisper-cli --vad`; empty for none.
    pub vad_model: String,
    /// The recorder: sox's `rec`, which writes raw 16 kHz mono PCM to stdout.
    pub recorder: String,
    /// Recording stops by itself after this many seconds.
    pub max_seconds: u32,
    /// A hold shorter than this (milliseconds) is an accidental press: nothing is recorded.
    pub min_hold_ms: u32,
    /// The engine gets this many seconds per clip.
    pub timeout_secs: u32,
    /// A clip whose loudest 50 ms stays under this RMS (0 to 1 of full scale) is silence
    /// and is not transcribed.
    pub silence_rms: f32,
    pub insert: Insert,
    /// With `insert = "paste"`: the pasteboard is restored this many milliseconds later.
    pub restore_ms: u32,
    /// Add a space after the transcript, so the next dictation does not run into it.
    pub trailing_space: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            engine: Engine::Whisper,
            whisper_bin: "/opt/homebrew/bin/whisper-cli".into(),
            parakeet_bin: "/opt/homebrew/bin/parakeet-cli".into(),
            command: vec![],
            model: "~/Library/Application Support/Flick/models/ggml-large-v3-turbo-q5_0.bin".into(),
            language: "en".into(),
            prompt: String::new(),
            vad_model: String::new(),
            recorder: "/opt/homebrew/bin/rec".into(),
            max_seconds: 120,
            min_hold_ms: 300,
            timeout_secs: 30,
            silence_rms: 0.01,
            insert: Insert::Paste,
            restore_ms: 250,
            trailing_space: true,
        }
    }
}

impl Settings {
    /// Checks the values serde cannot. Errors start with `[dictation] <key>: `.
    pub fn check(&self) -> Result<(), String> {
        let bad = |key: &str, why: &str| Err(format!("[dictation] {key}: {why}"));
        let empty = |s: &str| s.trim().is_empty();
        for (key, value) in [("recorder", &self.recorder), ("language", &self.language)] {
            if empty(value) {
                return bad(key, "must not be empty");
            }
        }
        match self.engine {
            Engine::Whisper if empty(&self.whisper_bin) => return bad("whisper_bin", "must not be empty"),
            Engine::Parakeet if empty(&self.parakeet_bin) => {
                return bad("parakeet_bin", "must not be empty");
            }
            Engine::Command => {
                if self.command.first().is_none_or(|p| empty(p)) {
                    return bad("command", "engine = \"command\" needs a program, e.g. [\"/path/stt\", \"{wav}\"]");
                }
                if !self.command.iter().any(|a| a.contains("{wav}")) {
                    return bad("command", "no {wav} argument: the program would never see the audio");
                }
            }
            Engine::Whisper | Engine::Parakeet => {}
        }
        if self.engine != Engine::Command && empty(&self.model) {
            return bad("model", "must not be empty");
        }
        if !(1..=MAX_SECONDS).contains(&self.max_seconds) {
            return bad("max_seconds", &format!("must be 1 to {MAX_SECONDS}"));
        }
        if !(1..=MAX_SECONDS).contains(&self.timeout_secs) {
            return bad("timeout_secs", &format!("must be 1 to {MAX_SECONDS}"));
        }
        for (key, ms) in [("min_hold_ms", self.min_hold_ms), ("restore_ms", self.restore_ms)] {
            if ms > MAX_MS {
                return bad(key, &format!("must be at most {MAX_MS}"));
            }
        }
        if !(0.0..1.0).contains(&self.silence_rms) {
            return bad("silence_rms", "must be at least 0 and below 1");
        }
        Ok(())
    }

    /// The engine's program, `~/` expanded against `home`.
    pub fn engine_bin(&self, home: &Path) -> PathBuf {
        let bin = match self.engine {
            Engine::Whisper => &self.whisper_bin,
            Engine::Parakeet => &self.parakeet_bin,
            Engine::Command => self.command.first().map_or("", String::as_str),
        };
        expand(bin, home)
    }

    /// The model file, `None` when the engine does not use one (a `command` template
    /// without `{model}`).
    pub fn model_path(&self, home: &Path) -> Option<PathBuf> {
        let used = self.engine != Engine::Command || self.command.iter().any(|a| a.contains("{model}"));
        used.then(|| expand(&self.model, home))
    }

    pub fn recorder_path(&self, home: &Path) -> PathBuf {
        expand(&self.recorder, home)
    }
}

/// `path` with a leading `~/` (or a lone `~`) replaced by `home`; trimmed.
pub fn expand(path: &str, home: &Path) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    fn settings(text: &str) -> Result<Settings, String> {
        let s: Settings = parse(text)?.section("dictation")?.ok_or("disabled")?.get()?;
        s.check()?;
        Ok(s)
    }

    #[test]
    fn defaults_are_safe() {
        let s = settings("").unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!((s.engine, s.insert), (Engine::Whisper, Insert::Paste));
        assert_eq!((s.min_hold_ms, s.restore_ms, s.max_seconds), (300, 250, 120));
        assert!(s.trailing_space);
        let home = Path::new("/Users/u");
        assert_eq!(s.engine_bin(home), Path::new("/opt/homebrew/bin/whisper-cli"));
        assert_eq!(s.recorder_path(home), Path::new("/opt/homebrew/bin/rec"));
        assert_eq!(
            s.model_path(home).unwrap(),
            Path::new("/Users/u/Library/Application Support/Flick/models/ggml-large-v3-turbo-q5_0.bin")
        );
        assert_eq!((Engine::default().name(), Insert::default().name()), ("whisper", "paste"));
    }

    #[test]
    fn reads_every_engine() {
        let s = settings("[dictation]\nengine = \"parakeet\"\nmodel = \"/m.nemo\"\ninsert = \"type\"").unwrap();
        let home = Path::new("/h");
        assert_eq!((s.engine.name(), s.insert.name()), ("parakeet", "type"));
        assert_eq!(s.engine_bin(home), Path::new("/opt/homebrew/bin/parakeet-cli"));
        assert_eq!(s.model_path(home).unwrap(), Path::new("/m.nemo"));
        let text = "[dictation]\nengine = \"command\"\nmodel = \"\"\ncommand = [\"~/bin/stt\", \"-f\", \"{wav}\"]";
        let s = settings(text).unwrap();
        assert_eq!(s.engine.name(), "command");
        assert_eq!(s.engine_bin(home), Path::new("/h/bin/stt"));
        assert_eq!(s.model_path(home), None);
        let s = settings("[dictation]\nengine = \"command\"\ncommand = [\"stt\", \"{wav}\", \"{model}\"]").unwrap();
        assert!(s.model_path(home).is_some());
    }

    #[test]
    fn bad_values_name_the_key() {
        let cases = [
            ("engine = \"cloud\"", "unknown variant `cloud`"),
            ("insert = \"dictate\"", "unknown variant `dictate`"),
            ("modle = \"x\"", "unknown field `modle`"),
            ("recorder = \" \"", "[dictation] recorder: must not be empty"),
            ("language = \"\"", "[dictation] language: must not be empty"),
            ("whisper_bin = \"\"", "[dictation] whisper_bin: must not be empty"),
            ("engine = \"parakeet\"\nparakeet_bin = \"\"", "[dictation] parakeet_bin: must not be"),
            ("engine = \"command\"", "[dictation] command: engine = \"command\" needs a program"),
            ("engine = \"command\"\ncommand = [\" \"]", "needs a program"),
            ("engine = \"command\"\ncommand = [\"stt\"]", "[dictation] command: no {wav}"),
            ("model = \"\"", "[dictation] model: must not be empty"),
            ("max_seconds = 0", "[dictation] max_seconds: must be 1 to 600"),
            ("max_seconds = 601", "[dictation] max_seconds: must be 1 to 600"),
            ("timeout_secs = 0", "[dictation] timeout_secs: must be 1 to 600"),
            ("min_hold_ms = 5001", "[dictation] min_hold_ms: must be at most 5000"),
            ("restore_ms = 9000", "[dictation] restore_ms: must be at most 5000"),
            ("silence_rms = 1.0", "[dictation] silence_rms: must be at least 0"),
            ("silence_rms = -0.1", "[dictation] silence_rms: must be at least 0"),
        ];
        for (body, want) in cases {
            let err = settings(&format!("[dictation]\n{body}")).unwrap_err();
            assert!(err.contains(want), "{body}\n=> {err}");
        }
    }

    #[test]
    fn expands_home() {
        let home = Path::new("/Users/u");
        assert_eq!(expand(" ~/a/b ", home), Path::new("/Users/u/a/b"));
        assert_eq!(expand("~", home), Path::new("/Users/u"));
        assert_eq!(expand("~other/x", home), Path::new("~other/x"));
        assert_eq!(expand("/abs", home), Path::new("/abs"));
        assert_eq!(expand("rel", home), Path::new("rel"));
    }
}
