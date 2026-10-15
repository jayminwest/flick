//! The `ext` verbs.
//!
//! - `ext ls [--json]`: each configured extension and how its last call went (op, ok or the
//!   error, and how long it took), copied from its worker without waiting on a call.
//! - `ext refresh <name>`: ask for its root items again, and for its view on the screen.
//! - `ext open <name> <view>`: view `<name>/<view>`, like a press of a hotkey bound to it.
//! - `ext call <name> <json>|--stdin`: a raw call, for debugging. The request text goes to
//!   the extension as it is (it must be a JSON object; nothing is added), on its worker, and
//!   the answer is the extension's stdout, unparsed, once it exits (`core::later`): a reply
//!   `proto` would refuse still shows. With `--json` a reply that is JSON is sent as JSON.
//!
//! `ext open` and `ext call` are in `NET_DENIED`: a peer must not pop the launcher on this
//! Mac or make it run an extension with a request of its choosing.

use serde_json::Value;

use super::Ext;
use super::worker::{Last, Worker};
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
                self.spec(name).ok_or_else(|| format!("ext: no extension \"{name}\""))?;
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
                    ("call", [text]) => self.raw(name, text),
                    _ => usage(),
                }
            }
            [v, ..] if matches!(v.as_str(), "ls" | "refresh" | "open" | "call") => usage(),
            _ => Err(unknown_verb("ext", args)),
        }
    }

    /// Queue raw request `text` on configured extension `name`'s worker; the verb answers
    /// with its reply.
    fn raw(&mut self, name: &str, text: &str) -> Result<String, String> {
        if !matches!(serde_json::from_str(text), Ok(Value::Object(_))) {
            return Err("ext call: the request must be a JSON object".into());
        }
        let reply = later::answer_later();
        self.worker(name)?.raw(text.to_string(), reply);
        Ok(format!("Called {name}"))
    }

    /// One line per extension, `<name>\t<op>\tok|error\t<ms> ms[\t<error>]` (`<name>\t-` before
    /// its first call), or with `json` an array of `{name, last}`.
    fn list(&self, json: bool) -> String {
        let rows = self.specs.iter().map(|s| {
            let last = self.workers.iter().find(|w| w.spec().name == s.name).and_then(Worker::last);
            (s.name.as_str(), last)
        });
        if json {
            let rows: Vec<String> = rows.map(|(name, last)| row_json(name, last.as_ref())).collect();
            return format!("[{}]", rows.join(","));
        }
        let lines: Vec<String> = rows.map(|(name, last)| line(name, last.as_ref())).collect();
        if lines.is_empty() { "ext: no extensions (add [[ext.extensions]] to config.toml)".into() } else { lines.join("\n") }
    }
}

fn line(name: &str, last: Option<&Last>) -> String {
    let Some(last) = last else { return format!("{name}\t-") };
    let ms = last.took.as_millis();
    match &last.error {
        None => format!("{name}\t{}\tok\t{ms} ms", last.op),
        Some(e) => format!("{name}\t{}\terror\t{ms} ms\t{e}", last.op),
    }
}

/// `{"name":…,"last":null|{"op":…,"ok":…,"error":…,"ms":…}}`.
fn row_json(name: &str, last: Option<&Last>) -> String {
    let text = |s: &str| Value::from(s).to_string();
    let last = last.map_or_else(
        || "null".into(),
        |l| {
            let error = l.error.as_deref().map_or_else(|| "null".into(), text);
            let ok = l.error.is_none();
            format!(r#"{{"op":"{}","ok":{ok},"error":{error},"ms":{}}}"#, l.op, l.took.as_millis())
        },
    );
    format!(r#"{{"name":{},"last":{last}}}"#, text(name))
}

#[cfg(test)]
mod tests;
