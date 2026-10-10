//! Quick ask (flick-039d): send a question to KOTA and show a pending KOTA card that
//! KOTA's reply replaces. `kota ask <text>` and the `kota/ask` launcher view both start
//! `send` on a thread.
//!
//! 1. The module makes the request id itself (`new_id`: `k`, then the time in ms and a
//!    counter, base 36), so the pending card and KOTA always agree on it (flick-7b52).
//! 2. `<this binary> message post --pending --id <id> --title KOTA -- <text>` (a client
//!    call to this Flick, answered by the main thread while this thread waits; 2 s, one
//!    retry when Flick is busy). The kota module never imports `message`.
//! 3. The text on stdin to `ssh -o BatchMode=yes -o ConnectTimeout=8 <ssh> <kota_ask>
//!    --id <id>` (15 s). The text is never in an argv: ssh would re-parse it through a
//!    remote shell.
//! 4. If ssh fails: `message post --reply-to <id> --title KOTA -- "KOTA ask failed: …"`,
//!    so the hourglass does not stay, and the ask's answer is the error.
//!
//! A failed pending post (step 2) does not stop the ask: KOTA's reply then shows as a
//! plain message. The module keeps the last `HISTORY` asks in memory only.

use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::io::Hooks;
use super::run::Exit;
use super::settings::Settings;

/// Budget of each `message post` self-call.
pub const POST_BUDGET: Duration = Duration::from_secs(2);
/// Pause before the one retry of a self-call that found Flick busy.
const RETRY_PAUSE: Duration = Duration::from_millis(250);
/// Budget of the ssh child (its own `ConnectTimeout` is 8 s).
pub const SSH_BUDGET: Duration = Duration::from_secs(15);
/// The longest question, in characters (kota-ask caps prompts at 2000).
pub const MAX_CHARS: usize = 2000;
/// Asks remembered for the view.
pub const HISTORY: usize = 8;

/// Where an ask is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Sending,
    /// kota-ask's answer (its last line, e.g. `queued for KOTA (…)`).
    Sent(String),
    Failed(String),
}

/// One ask, remembered for the view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub id: String,
    pub text: String,
    /// Unix seconds.
    pub at: u64,
    pub status: Status,
}

/// The question to send: `text` trimmed. `Err` for an empty or too long one.
pub fn check(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("usage: flick kota ask <text>".into());
    }
    if text.chars().count() > MAX_CHARS {
        return Err(format!("kota ask: over {MAX_CHARS} characters"));
    }
    Ok(text.to_string())
}

/// A fresh request id: `k`, the time in ms and a counter, base 36 (a valid card id).
pub fn new_id() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    id_from(ms, N.fetch_add(1, Ordering::Relaxed))
}

fn id_from(ms: u128, n: u32) -> String {
    format!("k{}{}", base36(ms), base36(u128::from(n % 36)))
}

fn base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = vec![];
    loop {
        out.push(char::from(DIGITS[(n % 36) as usize]));
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.iter().rev().collect()
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| (*w).to_string()).collect()
}

/// `message post --pending` of the question, as this binary's client call.
pub fn pending_argv(exe: &str, id: &str, text: &str) -> Vec<String> {
    words(&[exe, "message", "post", "--pending", "--id", id, "--title", "KOTA", "--", text])
}

/// The reply that replaces the pending card when the ask failed.
pub fn failed_argv(exe: &str, id: &str, reason: &str) -> Vec<String> {
    let body = format!("KOTA ask failed: {reason}");
    words(&[exe, "message", "post", "--reply-to", id, "--title", "KOTA", "--", &body])
}

/// ssh to kota-ask; the question goes on stdin.
pub fn ssh_argv(s: &Settings, id: &str) -> Vec<String> {
    words(&["/usr/bin/ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", &s.ssh, &s.kota_ask, "--id", id])
}

/// The last non-empty line of `text`.
fn last_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).rfind(|l| !l.is_empty())
}

/// A child's outcome: `Ok` with its last stdout line when it exited 0, else why not.
fn outcome(what: &str, out: Result<Exit, String>) -> Result<String, String> {
    let exit = out.map_err(|e| format!("{what}: {e}"))?;
    if exit.code == Some(0) {
        return Ok(last_line(&exit.stdout).unwrap_or_default().to_string());
    }
    Err(match (last_line(&exit.stderr), exit.code) {
        (Some(line), _) => line.to_string(),
        (None, Some(code)) => format!("{what} exited {code}"),
        (None, None) => format!("{what} was killed"),
    })
}

/// A self-call to this Flick, retried once when it was busy.
fn post(argv: &[String], hooks: Hooks) -> Result<String, String> {
    let first = outcome("flick", (hooks.run)(argv, POST_BUDGET));
    match first {
        Err(e) if e.contains("busy") => {
            thread::sleep(RETRY_PAUSE);
            outcome("flick", (hooks.run)(argv, POST_BUDGET))
        }
        other => other,
    }
}

/// Send question `text` as request `id` (steps 2 to 4 above). Runs on a thread. `Ok` is
/// kota-ask's answer; `Err` why the ask failed.
pub fn send(id: &str, text: &str, s: &Settings, hooks: Hooks) -> Result<String, String> {
    let exe = (hooks.exe)();
    let placeholder = exe.as_deref().map_err(String::clone).and_then(|exe| post(&pending_argv(exe, id, text), hooks));
    let sent = outcome("ssh", (hooks.feed)(&ssh_argv(s, id), text, SSH_BUDGET));
    match (sent, placeholder) {
        (Ok(answer), Ok(_)) => Ok(answer),
        (Ok(answer), Err(e)) => Ok(format!("{answer} (no pending card: {e})")),
        (Err(why), _) => {
            if let Ok(exe) = &exe {
                // Best effort: the error is the ask's answer either way.
                let _ = post(&failed_argv(exe, id, &why), hooks);
            }
            Err(why)
        }
    }
}

/// Remember `ask` first, at most `HISTORY`.
pub fn remember(asks: &mut Vec<Ask>, ask: Ask) {
    asks.insert(0, ask);
    asks.truncate(HISTORY);
}

/// Set the status of ask `id`, if it is still remembered.
pub fn settle(asks: &mut [Ask], id: &str, result: &Result<String, String>) {
    if let Some(a) = asks.iter_mut().find(|a| a.id == id) {
        a.status = match result {
            Ok(answer) => Status::Sent(answer.clone()),
            Err(why) => Status::Failed(why.clone()),
        };
    }
}

#[cfg(test)]
mod tests;
