//! Service check verdicts: pure functions from what a check's child process (or connect)
//! returned to `ok`, `warn`, `fail` or `unknown` with a one-line reason. `io.rs` runs the
//! children; nothing here touches the system.

use serde::Serialize;
use std::time::Duration;

use super::run::Exit;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
    /// Not checked yet, or the answer could not be read.
    Unknown,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Unknown => "unknown",
        }
    }
}

/// A check's verdict.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Verdict {
    pub status: Status,
    pub reason: String,
    /// The number the thresholds were compared with: milliseconds for http and tcp, the
    /// first number a command printed.
    pub value: Option<f64>,
}

impl Verdict {
    fn new(status: Status, reason: impl Into<String>) -> Verdict {
        Verdict { status, reason: reason.into(), value: None }
    }

    pub fn fail(reason: impl Into<String>) -> Verdict {
        Verdict::new(Status::Fail, reason)
    }

    pub fn unknown(reason: impl Into<String>) -> Verdict {
        Verdict::new(Status::Unknown, reason)
    }
}

/// `warn`/`fail` thresholds. Higher is worse, unless both are set and `fail` is below
/// `warn` (then lower is worse, e.g. free space).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Limits {
    pub warn: Option<f64>,
    pub fail: Option<f64>,
}

impl Limits {
    /// `value` with `what` (`"12 ms"`) graded against the limits.
    fn grade(self, value: f64, what: String) -> Verdict {
        let lower = matches!((self.warn, self.fail), (Some(w), Some(f)) if f < w);
        let op = if lower { "<=" } else { ">=" };
        let crossed = |limit: Option<f64>| {
            limit.filter(|&l| if lower { value <= l } else { value >= l })
        };
        let (status, reason) = if let Some(l) = crossed(self.fail) {
            (Status::Fail, format!("{what} (fail {op} {})", num(l)))
        } else if let Some(l) = crossed(self.warn) {
            (Status::Warn, format!("{what} (warn {op} {})", num(l)))
        } else {
            (Status::Ok, what)
        };
        Verdict { status, reason, value: Some(value) }
    }
}

/// `n` without a trailing `.0`.
pub fn num(n: f64) -> String {
    format!("{n}")
}

/// The first non-empty line of `text`, without a `curl: ` style prefix.
fn first_line(text: &str, prefix: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.strip_prefix(prefix).unwrap_or(line).to_string())
}

/// Why a child failed: its stderr's first line, else its exit.
fn exit_reason(exit: &Exit, prefix: &str) -> String {
    first_line(&exit.stderr, prefix).unwrap_or_else(|| match exit.code {
        Some(c) => format!("exit {c}"),
        None => "killed by a signal".into(),
    })
}

/// `curl -sS -o /dev/null -w '%{http_code} %{time_total}'`: 2xx and 3xx pass, graded on
/// milliseconds.
pub fn http(run: Result<Exit, String>, limits: Limits) -> Verdict {
    let exit = match run {
        Ok(exit) => exit,
        Err(e) => return Verdict::fail(e),
    };
    if exit.code != Some(0) {
        return Verdict::fail(exit_reason(&exit, "curl: "));
    }
    let mut words = exit.stdout.split_whitespace();
    let code = words.next().and_then(|c| c.parse::<u16>().ok());
    let secs = words.next().and_then(|t| t.parse::<f64>().ok());
    match (code, secs) {
        (Some(code @ 200..=399), Some(secs)) => {
            let ms = (secs * 1000.0).round();
            limits.grade(ms, format!("HTTP {code} in {} ms", num(ms)))
        }
        (Some(code), _) if code >= 100 => Verdict::fail(format!("HTTP {code}")),
        _ => Verdict::unknown(format!("unexpected curl output {:?}", exit.stdout.trim())),
    }
}

/// A TCP connect: the time it took, or why it failed.
pub fn tcp(connect: Result<Duration, String>, limits: Limits) -> Verdict {
    match connect {
        Ok(took) => {
            let ms = (took.as_secs_f64() * 1000.0).round();
            limits.grade(ms, format!("open in {} ms", num(ms)))
        }
        Err(e) => Verdict::fail(e),
    }
}

/// What `launchctl print` says about a job: its top-level `key = value` lines (one tab in;
/// nested blocks such as coalitions have their own `state`).
#[derive(Debug, Default, PartialEq, Eq)]
struct Job<'a> {
    state: Option<&'a str>,
    pid: Option<&'a str>,
    last_exit: Option<&'a str>,
    signal: Option<&'a str>,
}

fn job(text: &str) -> Job<'_> {
    let mut j = Job::default();
    for line in text.lines() {
        let Some(line) = line.strip_prefix('\t') else { continue };
        if line.starts_with('\t') {
            continue;
        }
        let Some((key, value)) = line.split_once(" = ") else { continue };
        let slot = match key {
            "state" => &mut j.state,
            "pid" => &mut j.pid,
            "last exit code" => &mut j.last_exit,
            "last terminating signal" => &mut j.signal,
            _ => continue,
        };
        *slot = Some(value.trim());
    }
    j
}

/// `launchctl print gui/<uid>/<label>`: running is ok; running after a bad exit or a kill
/// warns; not running is ok after exit 0 (a periodic job) and fails otherwise; a label
/// launchd does not know fails.
pub fn launchd(run: Result<Exit, String>, domain: &str) -> Verdict {
    let exit = match run {
        Ok(exit) => exit,
        Err(e) => return Verdict::fail(e),
    };
    if exit.code != Some(0) {
        let text = format!("{}\n{}", exit.stderr, exit.stdout);
        if text.contains("Could not find service") {
            return Verdict::fail(format!("not loaded in {domain}"));
        }
        return Verdict::fail(exit_reason(&exit, ""));
    }
    let j = job(&exit.stdout);
    let bad_exit = j.last_exit.filter(|c| *c != "0" && *c != "(never exited)");
    let past = match (bad_exit, j.signal) {
        (Some(code), _) => Some(format!("last exit {code}")),
        (None, Some(sig)) => Some(format!("last killed: {sig}")),
        (None, None) => None,
    };
    let pid = j.pid.map_or(String::new(), |p| format!(" (pid {p})"));
    match (j.state, past) {
        (Some("running"), None) => Verdict::new(Status::Ok, format!("running{pid}")),
        (Some("running"), Some(past)) => Verdict::new(Status::Warn, format!("running{pid}; {past}")),
        (Some("not running"), None) => {
            let last = j.last_exit.map_or(String::new(), |c| format!("; last exit {c}"));
            Verdict::new(Status::Ok, format!("not running{last}"))
        }
        (Some("not running"), Some(past)) => Verdict::fail(format!("not running; {past}")),
        (Some(state), _) => Verdict::new(Status::Warn, format!("state {state}")),
        (None, _) => Verdict::unknown("no state in launchctl output"),
    }
}

/// `pgrep -x <name>`: exit 0 with pids, 1 when none match.
pub fn process(run: Result<Exit, String>) -> Verdict {
    let exit = match run {
        Ok(exit) => exit,
        Err(e) => return Verdict::fail(e),
    };
    match exit.code {
        Some(0) => {
            let pids: Vec<&str> = exit.stdout.split_whitespace().collect();
            Verdict::new(Status::Ok, format!("running (pid {})", pids.join(", ")))
        }
        Some(1) => Verdict::fail("not running"),
        _ => Verdict::unknown(exit_reason(&exit, "pgrep: ")),
    }
}

/// A configured command: exit 0, then the first number on stdout graded. Without limits
/// any exit 0 passes.
pub fn command(run: Result<Exit, String>, limits: Limits) -> Verdict {
    let exit = match run {
        Ok(exit) => exit,
        Err(e) => return Verdict::fail(e),
    };
    if exit.code != Some(0) {
        return Verdict::fail(match (exit.code, first_line(&exit.stderr, "")) {
            (Some(code), Some(line)) => format!("exit {code}: {line}"),
            _ => exit_reason(&exit, ""),
        });
    }
    let value = exit.stdout.split_whitespace().find_map(|w| w.parse::<f64>().ok().filter(|v| v.is_finite()));
    match value {
        Some(v) => limits.grade(v, num(v)),
        None if limits.warn.is_none() && limits.fail.is_none() => {
            Verdict::new(Status::Ok, first_line(&exit.stdout, "").unwrap_or_else(|| "exit 0".into()))
        }
        None => Verdict::unknown("no number in the output"),
    }
}

#[cfg(test)]
mod tests;
