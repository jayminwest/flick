//! Table `[ext]`: `[[ext.extensions]]`, one entry per extension. Pure: `~/` expansion is
//! string work on the home folder the caller passes, so `configure` never touches the disk
//! (a missing program fails its first call, on the worker).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use super::proto::{EventKind, KEYWORDS_MAX, Op, SHORT_MAX, hidden};
use crate::core::card::valid_id;

/// `timeout_ms` when an entry does not set it.
pub const TIMEOUT_MS: u64 = 2000;
/// The least time `act` and `submit` get: they do the extension's real work.
pub const LONG: Duration = Duration::from_secs(10);
/// Longest extension name, in chars.
pub const NAME_MAX: usize = 32;

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Settings {
    extensions: Vec<Entry>,
}

/// One `[[ext.extensions]]` entry.
#[derive(Deserialize)]
pub struct Entry {
    /// Its id (`ext:<name>/<key>` items, `FLICK_EXT`) and the prefix of its errors.
    name: String,
    /// The program and its arguments, run without a shell. A leading `~/` is the home folder.
    argv: Vec<String>,
    /// Time each call may take before the process is killed, in milliseconds.
    #[serde(default = "timeout_ms")]
    timeout_ms: u64,
    /// Global hotkeys that open its views: `<view> = "<hotkey>"`.
    #[serde(default)]
    hotkeys: BTreeMap<String, String>,
    /// Words that send `<keyword> <text>` in root search to it (`op: keyword`).
    #[serde(default)]
    keywords: Vec<String>,
    /// Events sent to it (`op: event`), by `EventKind::name`.
    #[serde(default)]
    subscribe: Vec<String>,
}

fn timeout_ms() -> u64 {
    TIMEOUT_MS
}

/// A configured extension, ready to spawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub name: String,
    pub argv: Vec<String>,
    pub timeout: Duration,
    /// `(view, hotkey)`, by view name; blank hotkeys left out.
    pub hotkeys: Vec<(String, String)>,
    /// Its root search keywords: one word each, unique among all extensions.
    pub keywords: Vec<String>,
    /// The events it gets, each once.
    pub subscribe: Vec<EventKind>,
}

impl Spec {
    /// The time a call with `op` may take: `timeout`, and at least `LONG` for `act` and
    /// `submit`.
    pub fn budget(&self, op: Op) -> Duration {
        match op {
            Op::Act | Op::Submit => self.timeout.max(LONG),
            _ => self.timeout,
        }
    }
}

/// The extensions in `settings`, each argument's leading `~/` replaced by `home`. Names are
/// 1-`NAME_MAX` of `a-z 0-9 -` and unique; argv names a program; the timeout is not 0;
/// hotkey views are valid view names; up to `KEYWORDS_MAX` keywords, each one word of at
/// most `SHORT_MAX` chars that no other extension has; `subscribe` names event kinds.
pub fn read(settings: Settings, home: Option<&Path>) -> Result<Vec<Spec>, String> {
    let mut specs: Vec<Spec> = Vec::new();
    for e in settings.extensions {
        let name = e.name;
        let bad = |why: &str| Err(format!("[ext] extensions: \"{name}\": {why}"));
        let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        if name.is_empty() || name.chars().count() > NAME_MAX || !name.chars().all(ok) {
            return bad(&format!("name must be 1-{NAME_MAX} of a-z, 0-9 and -"));
        }
        if specs.iter().any(|s| s.name == name) {
            return bad("duplicate name; keep one");
        }
        if e.argv.first().is_none_or(|p| p.trim().is_empty()) {
            return bad("argv must name a program");
        }
        if e.timeout_ms == 0 {
            return bad("timeout_ms must be over 0");
        }
        if let Some(view) = e.hotkeys.keys().find(|v| !valid_id(v)) {
            return bad(&format!("hotkeys: view \"{view}\" must be 1-64 of A-Z a-z 0-9 . _ -"));
        }
        let word = |(i, k): (usize, &String)| {
            let taken = specs.iter().any(|s| s.keywords.contains(k)) || e.keywords[..i].contains(k);
            !taken && !k.is_empty() && k.chars().count() <= SHORT_MAX && !k.chars().any(|c| c.is_whitespace() || hidden(c))
        };
        if e.keywords.len() > KEYWORDS_MAX || !e.keywords.iter().enumerate().all(word) {
            return bad(&format!("keywords: at most {KEYWORDS_MAX} words of 1-{SHORT_MAX} chars, each used once"));
        }
        let mut subscribe = vec![];
        for word in &e.subscribe {
            let Some(kind) = EventKind::ALL.into_iter().find(|k| k.name() == word) else {
                let all: Vec<&str> = EventKind::ALL.iter().map(|k| k.name()).collect();
                return bad(&format!("subscribe: \"{word}\" is not one of {}", all.join(", ")));
            };
            if !subscribe.contains(&kind) {
                subscribe.push(kind);
            }
        }
        let hotkeys = e.hotkeys.into_iter().filter(|(_, spec)| !spec.trim().is_empty()).collect();
        let argv = e.argv.into_iter().map(|a| expand(a, home)).collect();
        let timeout = Duration::from_millis(e.timeout_ms);
        specs.push(Spec { name, argv, timeout, hotkeys, keywords: e.keywords, subscribe });
    }
    Ok(specs)
}

/// `arg` with a leading `~/` replaced by `home` (unchanged without one).
fn expand(arg: String, home: Option<&Path>) -> String {
    match (arg.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => arg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(text: &str) -> Result<Vec<Spec>, String> {
        let section = crate::config::parse(text).unwrap().section("ext").unwrap().unwrap();
        read(section.get::<Settings>()?, Some(Path::new("/Users/me")))
    }

    #[test]
    fn entries_expand_home_and_default_the_timeout() {
        let text = "[[ext.extensions]]\nname = \"notes-2\"\nargv = [\"~/bin/notes\", \"--json\", \"~/n\", \"a~/b\"]\n\
                    [[ext.extensions]]\nname = \"gh\"\nargv = [\"/usr/bin/env\", \"gh-ext\"]\ntimeout_ms = 500\n\
                    hotkeys = { prs = \"cmd+KeyP\", issues = \" \" }\nkeywords = [\"pr\", \"is\"]\n\
                    subscribe = [\"wake\", \"launcher_opened\", \"wake\"]";
        let got = specs(text).unwrap();
        assert_eq!(got[0].argv, ["/Users/me/bin/notes", "--json", "/Users/me/n", "a~/b"]);
        assert_eq!(got[0].timeout, Duration::from_millis(TIMEOUT_MS));
        assert_eq!((got[1].name.as_str(), got[1].timeout), ("gh", Duration::from_millis(500)));
        assert!(got[0].hotkeys.is_empty() && got[0].keywords.is_empty());
        assert_eq!(got[1].keywords, ["pr", "is"]);
        assert!(got[0].subscribe.is_empty());
        assert_eq!(got[1].subscribe, [EventKind::Wake, EventKind::LauncherOpened]);
        assert_eq!(got[1].hotkeys, [("prs".to_string(), "cmd+KeyP".to_string())]);
        assert!(specs("").unwrap().is_empty());
        let none = read(Settings { extensions: vec![] }, None).unwrap();
        assert!(none.is_empty());
        assert_eq!(expand("~/x".into(), None), "~/x");
    }

    #[test]
    fn act_and_submit_get_at_least_ten_seconds() {
        let spec = |ms| Spec { name: "x".into(), argv: vec![], timeout: Duration::from_millis(ms), hotkeys: vec![], keywords: vec![], subscribe: vec![] };
        assert_eq!(spec(2000).budget(Op::Root), Duration::from_secs(2));
        assert_eq!(spec(2000).budget(Op::Act), LONG);
        assert_eq!(spec(2000).budget(Op::Submit), LONG);
        assert_eq!(spec(30_000).budget(Op::Submit), Duration::from_secs(30));
    }

    #[test]
    fn bad_entries_name_the_extension() {
        let entry = |name: &str, rest: &str| format!("[[ext.extensions]]\nname = \"{name}\"\n{rest}");
        let err = |text: &str| specs(text).unwrap_err();
        let argv = "argv = [\"/bin/x\"]";
        for name in ["", "Notes", "a b", "a_b", &"x".repeat(NAME_MAX + 1)] {
            assert!(err(&entry(name, argv)).contains("name must be 1-32 of a-z, 0-9 and -"), "{name}");
        }
        let twice = format!("{}\n{}", entry("a", argv), entry("a", argv));
        assert_eq!(err(&twice), "[ext] extensions: \"a\": duplicate name; keep one");
        assert_eq!(err(&entry("a", "argv = []")), "[ext] extensions: \"a\": argv must name a program");
        assert!(err(&entry("a", "argv = [\" \"]")).ends_with("argv must name a program"));
        assert!(err(&entry("a", &format!("{argv}\ntimeout_ms = 0"))).ends_with("timeout_ms must be over 0"));
        assert!(err(&entry("a", "argv = \"/bin/x\"")).starts_with("[ext]"));
        let view = format!("{argv}\nhotkeys = {{ \"a b\" = \"cmd+KeyA\" }}");
        assert_eq!(err(&entry("a", &view)), "[ext] extensions: \"a\": hotkeys: view \"a b\" must be 1-64 of A-Z a-z 0-9 . _ -");
        let keywords = |list: &str| entry("a", &format!("{argv}\nkeywords = {list}"));
        let many = format!("[{}]", (0..=KEYWORDS_MAX).map(|i| format!("\"k{i}\"")).collect::<Vec<_>>().join(", "));
        let other = format!("{}\n{}", keywords("[\"k\"]"), entry("b", &format!("{argv}\nkeywords = [\"j\", \"k\"]")));
        assert_eq!(err(&other), "[ext] extensions: \"b\": keywords: at most 16 words of 1-80 chars, each used once");
        let event = entry("a", &format!("{argv}\nsubscribe = [\"wake\", \"boot\"]"));
        let want = "[ext] extensions: \"a\": subscribe: \"boot\" is not one of started, reloaded, wake, unlocked, launcher_opened";
        assert_eq!(err(&event), want);
        let long = format!("[\"{}\"]", "k".repeat(SHORT_MAX + 1));
        for bad in [&many, "[\"\"]", "[\"a b\"]", "[\"a\\u0007\"]", &long, "[\"k\", \"k\"]"] {
            assert!(err(&keywords(bad)).ends_with("keywords: at most 16 words of 1-80 chars, each used once"), "{bad}");
        }
    }

    #[test]
    fn the_example_documents_every_key() {
        crate::config::example::assert_documents::<Settings>("ext");
        crate::config::example::assert_documents::<Entry>("ext.extensions");
    }
}
