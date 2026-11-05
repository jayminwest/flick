//! The `ext` verbs.
//!
//! - `ext ls [--json]`: each configured extension, the call it runs now (op, for how long)
//!   and how its last call went (op, ok or the error, how long it took and how long ago it
//!   ended), copied from its worker without waiting on a call.
//! - `ext refresh <name>`: ask for its root items again, and for its view on the screen.
//! - `ext open <name> <view>`: show view `<name>/<view>`, like a press of a hotkey bound to
//!   it, except that a view already on the screen stays (a hotkey would hide it).
//! - `ext call <name> <json>|--stdin`: a raw call, for debugging. The request text goes to
//!   the extension as it is (it must be a JSON object; nothing is added), on its worker, and
//!   the answer is the extension's stdout, unparsed, once it exits (`core::later`): a reply
//!   `proto` would refuse still shows. With `--json` a reply that is JSON is sent as JSON.
//!   The socket waits `later::MAX_WAIT` past the call's own budget, for the calls queued
//!   before it, so a `timeout_ms` over a minute still answers.
//!
//! `ext refresh`, `ext open` and `ext call` are in `NET_DENIED`: a peer must not make this
//! Mac spawn extension processes, pop the launcher on it, or run an extension with a request
//! of its choosing.

use std::time::Duration;

use serde_json::Value;

use super::Ext;
use super::proto::Op;
use super::worker::{Last, Running, Worker};
use crate::core::later;
use crate::core::unknown_verb;

pub const VERBS: &str = "ext ls | ext refresh <name> | ext open <name> <view> | ext call <name> <json>|--stdin";

impl Ext {
    /// Run verb `args` (from the verb on); `json`: the request asked for `--json`.
    pub(super) fn run_verb(&mut self, args: &[String], json: bool) -> Result<String, String> {
        let usage = || Err(format!("usage: flick {VERBS}"));
        match args {
            [v] if v == "ls" => Ok(self.list(json)),
            [v, name, rest @ ..] if matches!(v.as_str(), "refresh" | "open" | "call") => {
                // A raw call gets the budget of `act` (`transport::raw`).
                let budget = self.spec(name).ok_or_else(|| format!("ext: no extension \"{name}\""))?.budget(Op::Act);
                match (v.as_str(), rest) {
                    ("refresh", []) => {
                        self.refetch(name);
                        Ok(format!("Asked {name} again"))
                    }
                    ("open", [view]) if !view.is_empty() && !view.contains('/') => {
                        let view = format!("{name}/{view}");
                        (self.hooks.open)("ext", &view);
                        Ok(format!("Opening {view}"))
                    }
                    ("call", [text]) => self.raw(name, text, budget),
                    _ => usage(),
                }
            }
            [v, ..] if matches!(v.as_str(), "ls" | "refresh" | "open" | "call") => usage(),
            _ => Err(unknown_verb("ext", args)),
        }
    }

    /// Queue raw request `text` on configured extension `name`'s worker; the verb answers
    /// with its reply, waited for up to `budget` (the call's own) plus `later::MAX_WAIT`.
    fn raw(&mut self, name: &str, text: &str, budget: Duration) -> Result<String, String> {
        if !matches!(serde_json::from_str(text), Ok(Value::Object(_))) {
            return Err("ext call: the request must be a JSON object".into());
        }
        let reply = later::answer_within(budget + later::MAX_WAIT);
        self.worker(name)?.raw(text.to_string(), reply);
        Ok(format!("Called {name}"))
    }

    /// One line per extension (see `line`), or with `json` an array of `{name, running, last}`.
    fn list(&self, json: bool) -> String {
        let rows = self.specs.iter().map(|s| {
            let worker = self.workers.iter().find(|w| w.spec().name == s.name);
            (s.name.as_str(), worker.and_then(Worker::running), worker.and_then(Worker::last))
        });
        if json {
            let rows: Vec<String> = rows.map(|(name, now, last)| row_json(name, now, last.as_ref())).collect();
            return format!("[{}]", rows.join(","));
        }
        let lines: Vec<String> = rows.map(|(name, now, last)| line(name, now, last.as_ref())).collect();
        if lines.is_empty() { "ext: no extensions (add [[ext.extensions]] to config.toml)".into() } else { lines.join("\n") }
    }
}

/// `<name>\t<op>\trunning\t<ms> ms` while a call runs (`<ms>` so far), else
/// `<name>\t<op>\tok|error\t<ms> ms\t<s> s ago[\t<error>]` for the last call, `<name>\t-`
/// before the first.
fn line(name: &str, now: Option<Running>, last: Option<&Last>) -> String {
    if let Some(now) = now {
        return format!("{name}\t{}\trunning\t{} ms", now.op, now.started.elapsed().as_millis());
    }
    let Some(last) = last else { return format!("{name}\t-") };
    let (ms, ago) = (last.took.as_millis(), last.ended.elapsed().as_secs());
    match &last.error {
        None => format!("{name}\t{}\tok\t{ms} ms\t{ago} s ago", last.op),
        Some(e) => format!("{name}\t{}\terror\t{ms} ms\t{ago} s ago\t{e}", last.op),
    }
}

/// `{"name":…,"running":null|{"op":…,"ms":…},"last":null|{"op":…,"ok":…,"error":…,"ms":…,"ago_ms":…}}`.
fn row_json(name: &str, now: Option<Running>, last: Option<&Last>) -> String {
    let text = |s: &str| Value::from(s).to_string();
    let now = now.map_or_else(|| "null".into(), |r| format!(r#"{{"op":"{}","ms":{}}}"#, r.op, r.started.elapsed().as_millis()));
    let last = last.map_or_else(
        || "null".into(),
        |l| {
            let error = l.error.as_deref().map_or_else(|| "null".into(), text);
            let (ok, ago) = (l.error.is_none(), l.ended.elapsed().as_millis());
            format!(r#"{{"op":"{}","ok":{ok},"error":{error},"ms":{},"ago_ms":{ago}}}"#, l.op, l.took.as_millis())
        },
    );
    format!(r#"{{"name":{},"running":{now},"last":{last}}}"#, text(name))
}

#[cfg(test)]
mod tests;
