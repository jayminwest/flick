//! The action vocabulary: what a card button may do on this Mac (`do`), the policy that
//! refuses a `do` (`Do::check`), and the values JSON a press sends back to KOTA.
//!
//! `do` is a closed set, written as an object with exactly one key:
//!
//! | `do` | runs |
//! |---|---|
//! | `{"open_url":"https://…"}` | opens an http(s) link |
//! | `{"open_app":"Safari"}` | opens an app by name or bundle id (not a path) |
//! | `{"copy":"text"}` | copies the literal text |
//! | `{"script":{"name":"deploy","query":"prod"}}` | runs the `[[script.commands]]` entry `name` (`query` optional) |
//! | `{"flick":["task","start","x"]}` | runs one flick request (module, verb, args) |
//! | `{"shell":"make deploy"}` | runs a shell command, only after an in-card confirm |
//! | `{"dismiss":true}` or `"dismiss"` | closes the card |
//!
//! No field values are templated into a `do` (v1). An action without `do` replies to KOTA;
//! `"reply": true` beside a `do` does both.

use serde_json::{Map, Value, json};

use super::{
    ACTIONS_MAX, Action, ID_MAX, INPUT_MAX, Kind, LABEL_MAX, Origin, Style, VALUES_MAX, clip_warn,
    valid_id,
};
use crate::core::control::{self, EVENTS, JSON, REMOTE};

/// Longest `open_url`, in chars.
pub const URL_MAX: usize = 2048;
/// Longest `open_app` name, in chars.
pub const APP_MAX: usize = 256;
/// Longest `shell` command, in chars. A longer one is refused, not cut: the confirm step
/// must show exactly what runs.
pub const SHELL_MAX: usize = 2000;
/// Client flags a `flick` action may not carry: they would retarget the request (another
/// host, the remote marker) or change its reply format.
const CLIENT_FLAGS: &[&str] = &["--host", JSON, REMOTE, "--stdin"];
/// First words the `flick` command line runs in its own process instead of sending to the
/// running Flick (`cli::parse`). A `flick` action runs by re-executing the Flick binary, so
/// these would run outside the control path and its policy (`snapshot` writes any file,
/// `import-raycast` edits the config).
const IN_PROCESS: &[&str] = &["snapshot", "import-raycast", "help", "config"];

/// A local action, from an action's `do`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Do {
    /// Open an http or https URL in the default browser.
    OpenUrl(String),
    /// Open (or bring forward) an app by name or bundle id.
    OpenApp(String),
    /// Put the literal text on the clipboard.
    Copy(String),
    /// Run the configured `[[script.commands]]` entry `name`, with `query` if given.
    Script { name: String, query: Option<String> },
    /// Run one flick request: module, verb, args.
    Flick(Vec<String>),
    /// Run a shell command after the user confirms the exact command in the card.
    Shell(String),
    /// Close the card.
    Dismiss,
}

impl Do {
    /// The `do` value `v`, read but not policy-checked (see `check`).
    pub fn parse(v: &Value) -> Result<Do, String> {
        let (key, arg) = match v {
            Value::String(key) => (key.as_str(), &Value::Null),
            Value::Object(o) if o.len() == 1 => {
                o.iter().next().map_or(("", v), |(k, v)| (k.as_str(), v))
            }
            _ => return Err("do must be an object with one key".into()),
        };
        let text =
            || arg.as_str().map(str::to_string).ok_or_else(|| format!("{key} needs a string"));
        Ok(match key {
            "open_url" => Do::OpenUrl(text()?),
            "open_app" => Do::OpenApp(text()?),
            "copy" => Do::Copy(text()?),
            "shell" => Do::Shell(text()?),
            "script" => {
                let name = arg.get("name").and_then(Value::as_str).ok_or("script needs a name")?;
                let query = match arg.get("query") {
                    None | Some(Value::Null) => None,
                    Some(q) => Some(q.as_str().ok_or("script query is not a string")?.to_string()),
                };
                Do::Script { name: name.to_string(), query }
            }
            "flick" => {
                let words = arg.as_array().ok_or("flick needs a list of words")?;
                let words = words.iter().map(|w| w.as_str().map(str::to_string));
                Do::Flick(words.collect::<Option<_>>().ok_or("flick words must be strings")?)
            }
            "dismiss" => Do::Dismiss,
            _ => return Err(format!("unknown action {key:?}")),
        })
    }

    /// Whether this action may run for a card from `origin`. `Err` is the reason the button
    /// is disabled. Dispatch calls it again at press time.
    pub fn check(&self, origin: Origin) -> Result<(), String> {
        match self {
            Do::OpenUrl(url) => {
                let lower = url.to_ascii_lowercase();
                let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
                let ok = rest.is_some_and(|r| !r.is_empty())
                    && url.chars().count() <= URL_MAX
                    && !url.chars().any(|c| c.is_whitespace() || c.is_control());
                ok.then_some(()).ok_or_else(|| "open_url: only http and https links".into())
            }
            Do::OpenApp(app) => {
                let ok = !app.trim().is_empty()
                    && app.chars().count() <= APP_MAX
                    && !app.chars().any(char::is_control);
                if !ok {
                    return Err("open_app: empty or invalid app name".into());
                }
                // `workspace::find_app` also takes a path; a card may only name an app.
                let path = app.contains('/') || app.trim_start().starts_with('~');
                (!path)
                    .then_some(())
                    .ok_or_else(|| "open_app: a name or bundle id, not a path".into())
            }
            Do::Script { name, .. } if name.trim().is_empty() => Err("script: empty name".into()),
            Do::Flick(words) => flick(words, origin),
            Do::Shell(cmd) if cmd.trim().is_empty() => Err("shell: empty command".into()),
            Do::Shell(cmd) if cmd.chars().count() > SHELL_MAX => {
                Err(format!("shell: command over {SHELL_MAX} chars"))
            }
            Do::Copy(_) | Do::Dismiss | Do::Script { .. } | Do::Shell(_) => Ok(()),
        }
    }

    /// Whether a press must first show an in-card confirm (the exact command and a Run
    /// button). Only `shell`, always, whatever the origin.
    pub fn needs_confirm(&self) -> bool {
        matches!(self, Do::Shell(_))
    }

    /// The wire form, as `Do::parse` reads it.
    pub fn to_value(&self) -> Value {
        match self {
            Do::OpenUrl(s) => json!({ "open_url": s }),
            Do::OpenApp(s) => json!({ "open_app": s }),
            Do::Copy(s) => json!({ "copy": s }),
            Do::Script { name, query: None } => json!({ "script": { "name": name } }),
            Do::Script { name, query: Some(q) } => {
                json!({ "script": { "name": name, "query": q } })
            }
            Do::Flick(words) => json!({ "flick": words }),
            Do::Shell(s) => json!({ "shell": s }),
            Do::Dismiss => json!({ "dismiss": true }),
        }
    }
}

/// The policy for `flick` words: a module first (not a command the CLI runs itself), no
/// client flags, no `events` stream, and for a remote card, nothing `control::net_policy` refuses (so a card cannot launder a verb
/// denied over the network).
fn flick(words: &[String], origin: Origin) -> Result<(), String> {
    let first = words.first().map_or("", String::as_str);
    if first.is_empty() || first.starts_with('-') {
        return Err("flick: the first word must be a module".into());
    }
    if first == EVENTS {
        return Err("flick: events is a stream, not an action".into());
    }
    if IN_PROCESS.contains(&first) {
        return Err(format!("flick: {first} is not a module"));
    }
    if let Some(flag) =
        words.iter().find(|w| CLIENT_FLAGS.contains(&w.as_str()) || w.starts_with("--host="))
    {
        return Err(format!("flick: {flag} is not allowed in an action"));
    }
    match origin {
        Origin::Remote => control::net_policy(words).map_err(|e| format!("flick: {e}")),
        Origin::Local => Ok(()),
    }
}

/// Parse the `actions` array: invalid entries and those past `ACTIONS_MAX` are dropped
/// with a warning; a bad `do` keeps the action, disabled.
pub(super) fn parse_all(raw: &[Value], origin: Origin, w: &mut Vec<String>) -> Vec<Action> {
    let mut actions: Vec<Action> = Vec::new();
    for (i, v) in raw.iter().enumerate() {
        let at = format!("actions[{i}]");
        if actions.len() == ACTIONS_MAX {
            w.push(format!("more than {ACTIONS_MAX} actions; {at} and later dropped"));
            break;
        }
        match one(v, origin, &at, w) {
            Ok(a) if actions.iter().any(|b| b.id == a.id) => {
                w.push(format!("{at}: duplicate id {:?}; dropped", a.id));
            }
            Ok(a) => actions.push(a),
            Err(why) => w.push(format!("{at}: {why}; dropped")),
        }
    }
    actions
}

fn one(v: &Value, origin: Origin, at: &str, w: &mut Vec<String>) -> Result<Action, String> {
    let Value::Object(o) = v else {
        return Err("not an object".into());
    };
    let id = o.get("id").and_then(Value::as_str).filter(|id| valid_id(id));
    let id = id.ok_or_else(|| format!("id must be 1-{ID_MAX} of A-Z a-z 0-9 . _ -"))?.to_string();
    let label = o.get("label").and_then(Value::as_str).map(str::trim).filter(|l| !l.is_empty());
    let label = clip_warn(label.ok_or("no label")?, LABEL_MAX, &format!("{at}: label"), w);
    let style = match o.get("style") {
        None | Some(Value::Null) => Style::Default,
        Some(s) => style_of(s).unwrap_or_else(|| {
            w.push(format!("{at}: unknown style {s}; default used"));
            Style::Default
        }),
    };
    let reply = flag(o, "reply", at, w);
    let kind = match o.get("do") {
        None | Some(Value::Null) => Kind::Reply,
        Some(raw) => match Do::parse(raw).and_then(|run| run.check(origin).map(|()| run)) {
            Ok(run) => Kind::Local { run, reply },
            Err(reason) => {
                w.push(format!("{at}: {reason}; shown disabled"));
                Kind::Disabled { raw: raw.clone(), reply, reason }
            }
        },
    };
    Ok(Action { id, label, style, kind })
}

fn style_of(v: &Value) -> Option<Style> {
    match v.as_str()? {
        "default" => Some(Style::Default),
        "primary" => Some(Style::Primary),
        "destructive" => Some(Style::Destructive),
        _ => None,
    }
}

/// An optional boolean key; anything but true/false/null is false, with a warning.
fn flag(o: &Map<String, Value>, key: &str, at: &str, w: &mut Vec<String>) -> bool {
    match o.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            w.push(format!("{at}: {key} is not true or false; false used"));
            false
        }
    }
}

/// The current value of one input, for `values_json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A field's text (cut to `INPUT_MAX` chars when sent).
    Text(String),
    /// A single choice: the selected option id, or none.
    One(Option<String>),
    /// A `multi` choice: the selected option ids.
    Many(Vec<String>),
}

impl Input {
    fn to_value(&self) -> Value {
        match self {
            Input::Text(s) => Value::String(s.chars().take(INPUT_MAX).collect()),
            Input::One(s) => s.as_ref().map_or(Value::Null, |s| Value::String(s.clone())),
            Input::Many(ids) => json!(ids),
        }
    }
}

/// The values JSON a press sends to KOTA on stdin: one object keyed by input id, a string
/// for a field, an option id (or null) for a choice, a list of option ids for a `multi`
/// choice, e.g. `{"env":"prod","note":"ship it","tags":["a","b"]}`. `Err` when it is over
/// `VALUES_MAX` chars: the press is not sent.
pub fn values_json(inputs: &[(String, Input)]) -> Result<String, String> {
    let map: Map<String, Value> = inputs.iter().map(|(k, v)| (k.clone(), v.to_value())).collect();
    let json = Value::Object(map).to_string();
    let n = json.chars().count();
    if n > VALUES_MAX {
        return Err(format!("values are {n} chars, max {VALUES_MAX}"));
    }
    Ok(json)
}

#[cfg(test)]
mod tests;
