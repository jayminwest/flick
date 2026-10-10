//! The pure presence model (100% coverage floor): parse one round's two sources, find the
//! KOTA pane, map the round to a `State`, and debounce it into `Presence`. No I/O, no clock:
//! callers pass the time.
//!
//! Sources, per round: `herdr [--machine <m>] agent list` (JSON `{"id","result":{"agents"}}`
//! on stdout; an error is a line on stderr) and kota-dash's `/ok` (a JSON object of check
//! name to boolean, e.g. `{"health": true, "queue": true, ...}`).
//!
//! State of one round (`observe`):
//! - herdr answered and a pane matches: its status (`working` thinking, `blocked` blocked,
//!   `idle`/`done` idle, else unknown); degraded instead when `/ok` failed or a check is
//!   false, unless the pane is blocked (an approval waiting outranks a failing check).
//! - herdr answered and no pane matches: down (the server is there, KOTA is not).
//! - herdr failed, `/ok` answered: down when `health` is false, else degraded (`herdr`
//!   failing): the server is up, the pane cannot be seen.
//! - both failed: offline. The laptop cannot tell a dead server from its own lost network,
//!   so this is never called down.
//!
//! Debounce (`Presence::apply`): down and offline commit only on the second failing round
//! in a row (or at once when the state already is one of them); the first one keeps the
//! last state and asks for a quick retry (`RETRY_SECS`). Failing rounds within
//! `WAKE_GRACE` of a wake keep the last state, stale.

use serde::Deserialize;
use serde_json::Value;

use super::run::Exit;

/// The poll interval while KOTA is thinking or blocked, and after an ask.
pub const FAST_SECS: u64 = 15;
/// The retry after a first failing round.
pub const RETRY_SECS: u64 = 15;
/// Failing rounds this soon after a wake keep the last state, marked stale.
pub const WAKE_GRACE: u64 = 30;

/// What the menu bar shows for KOTA.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    Thinking,
    Blocked,
    Idle,
    Degraded,
    Down,
    Offline,
    #[default]
    Unknown,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Thinking => "thinking",
            State::Blocked => "blocked",
            State::Idle => "idle",
            State::Degraded => "degraded",
            State::Down => "down",
            State::Offline => "offline",
            State::Unknown => "unknown",
        }
    }

    /// Down or offline: needs two rounds in a row.
    pub fn failing(self) -> bool {
        matches!(self, State::Down | State::Offline)
    }

    /// The state herdr's `agent_status` means.
    fn of_status(status: &str) -> State {
        match status {
            "working" => State::Thinking,
            "blocked" => State::Blocked,
            "idle" | "done" => State::Idle,
            _ => State::Unknown,
        }
    }
}

/// One herdr pane with an agent, as `agent list` reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pane {
    pub id: String,
    pub agent: String,
    pub status: String,
    pub cwd: String,
    pub name: String,
    pub focused: bool,
    /// `terminal_title_stripped`: Claude Code puts its current task there.
    pub title: String,
}

#[derive(Deserialize)]
struct Info {
    pane_id: String,
    agent: Option<String>,
    agent_status: Option<String>,
    cwd: Option<String>,
    name: Option<String>,
    focused: Option<bool>,
    terminal_title_stripped: Option<String>,
}

#[derive(Deserialize)]
struct List {
    agents: Vec<Info>,
}

/// The panes in `herdr agent list`'s stdout. Unknown fields are ignored; a JSON `error`
/// is its message.
pub fn parse_agents(stdout: &str) -> Result<Vec<Pane>, String> {
    let reply: Value = serde_json::from_str(stdout.trim()).map_err(|e| format!("bad herdr reply: {e}"))?;
    if let Some(e) = reply.get("error") {
        return Err(e.get("message").and_then(Value::as_str).map_or_else(|| e.to_string(), str::to_string));
    }
    let list = List::deserialize(reply.get("result").unwrap_or(&reply)).map_err(|e| format!("bad herdr reply: {e}"))?;
    let text = |v: Option<String>| v.unwrap_or_default();
    Ok(list
        .agents
        .into_iter()
        .map(|a| Pane {
            id: a.pane_id,
            agent: text(a.agent),
            status: text(a.agent_status),
            cwd: text(a.cwd),
            name: text(a.name),
            focused: a.focused.unwrap_or(false),
            title: text(a.terminal_title_stripped),
        })
        .collect())
}

/// kota-dash `/ok`: each boolean member as `(name, ok)`. Other members are ignored.
pub fn parse_ok(body: &str) -> Result<Vec<(String, bool)>, String> {
    let bad = |e: String| format!("bad /ok reply: {e}");
    let reply: Value = serde_json::from_str(body.trim()).map_err(|e| bad(e.to_string()))?;
    let checks: Vec<(String, bool)> = reply
        .as_object()
        .ok_or_else(|| bad("not an object".into()))?
        .iter()
        .filter_map(|(k, v)| v.as_bool().map(|ok| (k.clone(), ok)))
        .collect();
    if checks.is_empty() {
        return Err(bad("no checks".into()));
    }
    Ok(checks)
}

/// The message in a child's stderr: a JSON error's message, else its first non-empty
/// line without a leading `error: `; `fallback` when there is none.
fn stderr_message(stderr: &str, fallback: String) -> String {
    let Some(line) = stderr.lines().map(str::trim).find(|l| !l.is_empty()) else { return fallback };
    if let Ok(reply) = serde_json::from_str::<Value>(line)
        && let Some(msg) = reply.pointer("/error/message").and_then(Value::as_str)
    {
        return msg.to_string();
    }
    match line.get(..7) {
        Some(head) if head.eq_ignore_ascii_case("error: ") => line[7..].to_string(),
        _ => line.to_string(),
    }
}

/// The herdr child's result as panes.
pub fn herdr_result(out: Result<Exit, String>) -> Result<Vec<Pane>, String> {
    let exit = out?;
    if exit.code != Some(0) {
        return Err(stderr_message(&exit.stderr, format!("herdr exited with {:?}", exit.code)));
    }
    parse_agents(&exit.stdout)
}

/// The curl child's result as checks.
pub fn dash_result(out: Result<Exit, String>) -> Result<Vec<(String, bool)>, String> {
    let exit = out?;
    if exit.code != Some(0) {
        return Err(stderr_message(&exit.stderr, format!("curl exited with {:?}", exit.code)));
    }
    parse_ok(&exit.stdout)
}

/// `a` and `b` name the same directory, give or take a trailing `/`.
fn same_dir(a: &str, b: &str) -> bool {
    let trim = |p: &str| if p.len() > 1 { p.strip_suffix('/').unwrap_or(p).to_string() } else { p.to_string() };
    !a.is_empty() && trim(a) == trim(b)
}

/// The KOTA pane: agent `claude` in directory `cwd`. When several match, the one named
/// `name` (if set) wins, then the focused one, then the lowest pane id (shorter first, so
/// `p2` comes before `p10`).
pub fn find_pane<'a>(panes: &'a [Pane], cwd: &str, name: &str) -> Option<&'a Pane> {
    let unnamed = |p: &Pane| name.is_empty() || p.name != name;
    panes
        .iter()
        .filter(|p| p.agent == "claude" && same_dir(&p.cwd, cwd))
        .min_by_key(|p| (unnamed(p), !p.focused, p.id.len(), p.id.as_str()))
}

/// One round's raw results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Round {
    pub herdr: Result<Vec<Pane>, String>,
    pub dash: Result<Vec<(String, bool)>, String>,
}

/// What one round saw, before the debounce.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub state: State,
    pub pane: Option<Pane>,
    /// Names of false checks, plus `dash` when `/ok` failed and `herdr` when herdr did.
    pub failing: Vec<String>,
    /// The sources' error messages, prefixed `herdr: ` and `dash: `.
    pub errors: Vec<String>,
}

/// Map a round to a state; `cwd` and `name` find the KOTA pane (`find_pane`).
pub fn observe(round: &Round, cwd: &str, name: &str) -> Seen {
    let mut seen = Seen::default();
    match &round.dash {
        Ok(checks) => seen.failing.extend(checks.iter().filter(|(_, ok)| !ok).map(|(n, _)| n.clone())),
        Err(e) => {
            seen.failing.push("dash".into());
            seen.errors.push(format!("dash: {e}"));
        }
    }
    let unhealthy = round.dash.as_ref().is_ok_and(|c| c.iter().any(|(n, ok)| n == "health" && !ok));
    seen.state = match &round.herdr {
        Err(e) => {
            seen.errors.insert(0, format!("herdr: {e}"));
            match &round.dash {
                Err(_) => State::Offline,
                Ok(_) if unhealthy => State::Down,
                Ok(_) => {
                    seen.failing.insert(0, "herdr".into());
                    State::Degraded
                }
            }
        }
        Ok(panes) => match find_pane(panes, cwd, name) {
            None => State::Down,
            Some(pane) => {
                seen.pane = Some(pane.clone());
                match State::of_status(&pane.status) {
                    State::Blocked => State::Blocked,
                    _ if !seen.failing.is_empty() => State::Degraded,
                    s => s,
                }
            }
        },
    };
    seen
}

/// A committed change of state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    pub from: State,
    pub to: State,
}

/// The debounced presence the menu bar and `kota status` show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presence {
    pub state: State,
    /// When `state` began, unix seconds; `None` before the first round.
    pub since: Option<u64>,
    /// The data is from before a sleep or lock and not confirmed since.
    pub stale: bool,
    /// The round behind `state`.
    pub seen: Seen,
    /// When the last counted round ended, unix seconds.
    pub checked_at: Option<u64>,
    /// The latest round's errors, also of a round that did not commit.
    pub errors: Vec<String>,
    /// The last counted round failed (down or offline) without committing yet.
    strike: bool,
}

impl Presence {
    /// Count a round that ended at `at`. `grace`: within `WAKE_GRACE` of a wake. Returns
    /// the transition when the state changed.
    pub fn apply(&mut self, seen: Seen, at: u64, grace: bool) -> Option<Transition> {
        self.errors.clone_from(&seen.errors);
        if seen.state.failing() {
            if grace {
                return None;
            }
            self.checked_at = Some(at);
            if !self.strike && !self.state.failing() {
                self.strike = true;
                return None;
            }
        }
        self.strike = false;
        self.stale = false;
        self.checked_at = Some(at);
        let from = self.state;
        self.state = seen.state;
        self.seen = seen;
        if from != self.state || self.since.is_none() {
            self.since = Some(at);
        }
        (from != self.state).then_some(Transition { from, to: self.state })
    }

    /// Sleep or lock: the data may be old by the time anyone looks. A failing round from
    /// before does not count toward the next one.
    pub fn mark_stale(&mut self) {
        self.stale = true;
        self.strike = false;
    }

    /// A first failing round waits for its confirmation.
    pub fn pending(&self) -> bool {
        self.strike
    }
}

/// Whether `now` is within `WAKE_GRACE` of the wake at `woke_at`.
pub fn in_grace(now: u64, woke_at: Option<u64>) -> bool {
    woke_at.is_some_and(|w| now < w.saturating_add(WAKE_GRACE))
}

/// Seconds from the last round to the next: `None` with `poll_secs` 0 (rounds only on
/// demand); `RETRY_SECS` while a failure waits for confirmation; `FAST_SECS` while KOTA
/// is thinking or blocked or `fast` holds (after an ask, the kota view open); else
/// `poll_secs`.
pub fn next_in(p: &Presence, poll_secs: u64, fast: bool) -> Option<u64> {
    if poll_secs == 0 {
        return None;
    }
    if p.pending() {
        return Some(RETRY_SECS.min(poll_secs));
    }
    let busy = fast || matches!(p.state, State::Thinking | State::Blocked);
    Some(if busy { FAST_SECS.min(poll_secs) } else { poll_secs })
}

#[cfg(test)]
mod tests;
