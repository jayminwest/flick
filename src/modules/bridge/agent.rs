//! The agent side of a bridge run: argv for claude and pi (one-shot, interactive, resume),
//! the login-shell and terminal wrappers, session ids, and parsers for each agent's JSON
//! output. Pure Rust; the worker (flick-3ffa) spawns what this builds.
//!
//! Flags follow `claude --help` (`-p --output-format json`, `--tools`, `--allowedTools`,
//! `--disallowedTools`, `--append-system-prompt`, `--session-id`, `--resume`, `--add-dir`,
//! `--model`, `--max-budget-usd`) and `pi --help` (`-p --mode json`, `--tools`,
//! `--append-system-prompt`, `--session-id`, `--session`, `--model`, `@file`). Parsers
//! ignore unknown fields and fall back to the raw output, so format drift degrades to
//! plain text instead of an error.

use serde_json::Value;
use std::fmt::Write as _;
use std::path::Path;

/// Longest answer text kept from raw (unparsed) output, in bytes.
pub const RAW_LIMIT: usize = 64 * 1024;

/// Verb prefixes an agent may never run, whatever the allowlist says: no agent spawns
/// agents or grants itself access to activity data.
pub const DENY: &[&str] = &["bridge", "activity remote"];

/// The agent program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Claude,
    Pi,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Pi => "pi",
        }
    }

    /// `claude` or `pi`, as the config and the CLI spell them.
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "claude" => Some(Kind::Claude),
            "pi" => Some(Kind::Pi),
            _ => None,
        }
    }
}

/// How to start one agent: the program and the settings from `[bridge]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub kind: Kind,
    /// The executable, resolved by the login shell (`claude`, `/opt/homebrew/bin/pi`).
    pub exe: String,
    /// `--model`; empty leaves the agent's default.
    pub model: String,
    /// Extra words before the prompt.
    pub args: Vec<String>,
    /// `--max-budget-usd` for claude one-shots; 0 means no cap. pi has no such flag.
    pub max_usd: f64,
    /// Flick verbs the agent may run (`task`, `herdr ls`), see `allowed_rules`.
    pub allow: Vec<String>,
    /// Working directory of the agent process.
    pub cwd: String,
}

/// What an agent run produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    pub text: String,
    pub session: Option<String>,
    pub cost_usd: Option<f64>,
    /// The agent reported a failure; `text` may still hold partial output.
    pub error: Option<String>,
}

/// Argv for a one-shot run that prints JSON and exits. `attach` is a file the agent may
/// read (a screenshot).
pub fn oneshot_argv(
    l: &Launch,
    brief: &str,
    prompt: &str,
    session: &str,
    attach: Option<&Path>,
) -> Vec<String> {
    build(l, brief, prompt, session, attach, true)
}

/// Argv for an interactive session in a terminal. An empty `prompt` starts it idle.
pub fn interactive_argv(
    l: &Launch,
    brief: &str,
    prompt: &str,
    session: &str,
    attach: Option<&Path>,
) -> Vec<String> {
    build(l, brief, prompt, session, attach, false)
}

/// Argv that reopens an earlier session interactively.
pub fn resume_argv(kind: Kind, exe: &str, session: &str) -> Vec<String> {
    let flag = match kind {
        Kind::Claude => "--resume",
        Kind::Pi => "--session",
    };
    strings(&[exe, flag, session])
}

fn build(
    l: &Launch,
    brief: &str,
    prompt: &str,
    session: &str,
    attach: Option<&Path>,
    oneshot: bool,
) -> Vec<String> {
    let mut v = vec![l.exe.clone()];
    if oneshot {
        v.push("-p".into());
        v.extend(strings(match l.kind {
            Kind::Claude => &["--output-format", "json"],
            Kind::Pi => &["--mode", "json"],
        }));
    }
    let mut prompt = prompt.to_string();
    match l.kind {
        Kind::Claude => {
            let tools = if attach.is_some() { "Bash,Read" } else { "Bash" };
            v.extend(strings(&["--tools", tools]));
            let mut allowed = allowed_rules(&l.allow);
            if let Some(a) = attach {
                allowed.push(format!("Read({})", a.display()));
            }
            if !allowed.is_empty() {
                v.push("--allowedTools".into());
                v.extend(allowed);
            }
            v.push("--disallowedTools".into());
            v.extend(DENY.iter().flat_map(|d| rules(d)));
        }
        Kind::Pi => {
            let tools = if attach.is_some() { "bash,read" } else { "bash" };
            v.extend(strings(&["--tools", tools]));
        }
    }
    v.extend(strings(&["--append-system-prompt", brief, "--session-id", session]));
    if !l.model.is_empty() {
        v.extend(strings(&["--model", &l.model]));
    }
    if l.kind == Kind::Claude && oneshot && l.max_usd > 0.0 {
        v.extend(strings(&["--max-budget-usd", &l.max_usd.to_string()]));
    }
    if let Some(a) = attach {
        match l.kind {
            Kind::Claude => {
                let dir = a.parent().unwrap_or(a);
                v.extend(strings(&["--add-dir", &dir.display().to_string()]));
                prompt = format!("{prompt}\n\nAttached file: {}", a.display());
            }
            Kind::Pi => v.push(format!("@{}", a.display())),
        }
    }
    v.extend(l.args.iter().cloned());
    if !prompt.is_empty() {
        v.push("--".into());
        v.push(prompt);
    }
    v
}

/// claude `--allowedTools` rules for the allowlist: each entry lets the agent run
/// `flick <entry> ...` and `flick --json <entry> ...`. Entries are normalized to single
/// spaces; ones with anything but lowercase words (which could smuggle rule syntax) and
/// ones under `DENY` are dropped.
pub fn allowed_rules(allow: &[String]) -> Vec<String> {
    let mut out = vec![];
    for entry in allow {
        let words: Vec<&str> = entry.split_whitespace().collect();
        let ok = |w: &&str| {
            w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        };
        if words.is_empty() || !words.iter().all(ok) {
            continue;
        }
        let entry = words.join(" ");
        if DENY.iter().any(|d| entry == *d || entry.starts_with(&format!("{d} "))) {
            continue;
        }
        out.extend(rules(&entry));
    }
    out
}

fn rules(verb: &str) -> [String; 2] {
    [format!("Bash(flick {verb}:*)"), format!("Bash(flick --json {verb}:*)")]
}

/// `argv` run through the user's login shell, so it gets their PATH and API keys. The
/// argv words are positional parameters of a fixed script, never parsed by the shell.
pub fn shell_wrap(shell: &str, argv: &[String]) -> Vec<String> {
    let mut v = strings(&[shell, "-lc", "exec \"$@\"", "flick-bridge"]);
    v.extend(argv.iter().cloned());
    v
}

/// `argv` in a new terminal window: the configured `terminal` words with `{cwd}` filled,
/// then `argv` (`wezterm start --cwd {cwd} --`, or any launcher that takes a command).
pub fn terminal_argv(terminal: &[String], cwd: &str, argv: &[String]) -> Vec<String> {
    let mut v: Vec<String> = terminal.iter().map(|w| w.replace("{cwd}", cwd)).collect();
    v.extend(argv.iter().cloned());
    v
}

/// A UUID v4 string from 16 random bytes (the caller reads them from /dev/urandom).
pub fn session_id(mut b: [u8; 16]) -> String {
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex = b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    });
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// claude `-p --output-format json`: one `{"type":"result",...}` object (an array of
/// messages under `--verbose`, JSON lines under stream-json; both are accepted).
pub fn parse_claude(stdout: &str) -> Answer {
    let Some(r) = claude_result(stdout) else { return raw(stdout) };
    let text = r.get("result").and_then(Value::as_str).unwrap_or_default().to_string();
    let subtype = r.get("subtype").and_then(Value::as_str).unwrap_or_default();
    let failed =
        r.get("is_error").and_then(Value::as_bool).unwrap_or(false) || subtype.starts_with("error");
    let error =
        failed.then(|| if text.is_empty() { format!("claude: {subtype}") } else { text.clone() });
    Answer {
        text,
        session: r.get("session_id").and_then(Value::as_str).map(String::from),
        cost_usd: r.get("total_cost_usd").and_then(Value::as_f64),
        error,
    }
}

fn claude_result(stdout: &str) -> Option<Value> {
    let is_result = |v: &Value| v.get("type").and_then(Value::as_str) == Some("result");
    match serde_json::from_str::<Value>(stdout.trim()) {
        Ok(Value::Array(items)) => items.into_iter().rev().find(is_result),
        Ok(v) => Some(v).filter(is_result),
        Err(_) => stdout
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(is_result),
    }
}

/// pi `-p --mode json`: JSON lines, a `session` header first, then agent events. The
/// answer is the last assistant `message_end` with text; cost sums every assistant
/// message's `usage.cost.total`.
pub fn parse_pi(stdout: &str) -> Answer {
    let events: Vec<Value> =
        stdout.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).collect();
    if events.is_empty() {
        return raw(stdout);
    }
    let mut a = Answer::default();
    let mut assistant = false;
    for e in &events {
        match e.get("type").and_then(Value::as_str) {
            Some("session") => a.session = e.get("id").and_then(Value::as_str).map(String::from),
            Some("message_end") => {
                let m = &e["message"];
                if m.get("role").and_then(Value::as_str) != Some("assistant") {
                    continue;
                }
                assistant = true;
                if let Some(c) = m.pointer("/usage/cost/total").and_then(Value::as_f64) {
                    a.cost_usd = Some(a.cost_usd.unwrap_or(0.0) + c);
                }
                let text = pi_text(m);
                if !text.is_empty() {
                    a.text = text;
                }
                let stop = m.get("stopReason").and_then(Value::as_str).unwrap_or_default();
                a.error = matches!(stop, "error" | "aborted").then(|| {
                    m.get("errorMessage")
                        .and_then(Value::as_str)
                        .map_or_else(|| format!("pi: request {stop}"), String::from)
                });
            }
            _ => {}
        }
    }
    if !assistant {
        a.error = Some("pi: no assistant message in the output".into());
    }
    a
}

fn pi_text(message: &Value) -> String {
    let parts: Vec<&str> = message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect();
    parts.join("\n").trim().to_string()
}

/// Unparsed output as the answer, trimmed and cut to `RAW_LIMIT` bytes.
fn raw(stdout: &str) -> Answer {
    let t = stdout.trim();
    if t.is_empty() {
        return Answer { error: Some("no output".into()), ..Answer::default() };
    }
    Answer { text: clip(t, RAW_LIMIT).to_string(), ..Answer::default() }
}

/// The longest prefix of `s` that fits in `max` bytes without splitting a char.
pub fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_string()).collect()
}

#[cfg(test)]
mod tests;
