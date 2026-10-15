//! The extension protocol, v1: the request Flick writes to an extension's stdin and the one
//! reply it reads from its stdout. Pure: no I/O and no processes (the transport spawns the
//! extension and hands its stdout to `parse`).
//!
//! Request: one JSON object, `{"v":1,"op":"<op>", ...}`:
//!
//! | op | keys | reply |
//! |---|---|---|
//! | `root` | | `items` |
//! | `open` | `view` | a view: `title`, `placeholder`, `filter` (`flick` or `ext`), `items` |
//! | `query` | `view`, `query` | `items` |
//! | `keyword` | `keyword`, `query` | `items` |
//! | `act` | `view` (null in root search), `item`, `action` | `status`, `refresh` |
//! | `form` | `form` | a form: `title`, `submit`, `fields` |
//! | `submit` | `form`, `values` (field key to text) | `status`, `refresh` |
//! | `event` | `event` (`EventKind::name`) | anything; ignored |
//!
//! Any reply may carry `"error": "<text>"`: `parse` returns it as the error.
//!
//! `parse` is strict only where nothing sensible can be shown: over `REPLY_MAX` bytes, not a
//! JSON object, a key of the wrong type at the top. Everything else degrades with a warning:
//! a bad item, action or field is skipped, over-cap lists and text are cut, an unknown icon,
//! tone, `after` or filter falls back to its default. Extension text never carries control
//! characters or invisible direction marks (`hidden`): such a string makes its item, action
//! or field invalid. Keys are never cut: a cut key would name another item.

pub mod form;
pub mod item;
#[cfg(test)]
mod tests;

use form::ExtForm;
use item::ExtItem;

use serde_json::{Map, Value, json};

/// The protocol version this Flick speaks. A reply with another `v` is read as v1.
pub const VERSION: u64 = 1;
/// Largest reply, in bytes. Over it, `parse` refuses the reply.
pub const REPLY_MAX: usize = 1024 * 1024;
/// Most items in a reply; extra items are dropped.
pub const ITEMS_MAX: usize = 500;
/// Most actions on an item; extra actions are dropped.
pub const ACTIONS_MAX: usize = 16;
/// Most fields in a form; extra fields are dropped.
pub const FIELDS_MAX: usize = 6;
/// Longest item key, in chars. A longer key is refused, not cut.
pub const KEY_MAX: usize = 128;
/// Longest title (item, view, form) or action title, in chars; a longer one is cut.
pub const TITLE_MAX: usize = 200;
/// Longest subtitle, status, confirm question or error, in chars; a longer one is cut.
pub const TEXT_MAX: usize = 500;
/// Longest accessory, keyword, placeholder, label or submit label, in chars.
pub const SHORT_MAX: usize = 80;
/// Most keywords on an item; extra keywords are dropped.
pub const KEYWORDS_MAX: usize = 16;
/// Longest form field value, in chars.
pub const VALUE_MAX: usize = 4000;

/// A request kind, as `"op"` spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Root,
    Open,
    Query,
    Keyword,
    Act,
    Form,
    Submit,
    Event,
}

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Op::Root => "root",
            Op::Open => "open",
            Op::Query => "query",
            Op::Keyword => "keyword",
            Op::Act => "act",
            Op::Form => "form",
            Op::Submit => "submit",
            Op::Event => "event",
        }
    }
}

/// What happened in Flick, for an extension that subscribes to events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Started,
    Reloaded,
    Wake,
    Unlocked,
    LauncherOpened,
}

impl EventKind {
    /// Every kind, in a fixed order (a worker's bit for a kind is its index).
    pub const ALL: [EventKind; 5] =
        [EventKind::Started, EventKind::Reloaded, EventKind::Wake, EventKind::Unlocked, EventKind::LauncherOpened];

    pub fn name(self) -> &'static str {
        match self {
            EventKind::Started => "started",
            EventKind::Reloaded => "reloaded",
            EventKind::Wake => "wake",
            EventKind::Unlocked => "unlocked",
            EventKind::LauncherOpened => "launcher_opened",
        }
    }
}

/// One call to an extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// The root search items.
    Root,
    /// Open the view `view`.
    Open { view: String },
    /// The items of `view` for the search text `query` (a view with `filter: "ext"`).
    Query { view: String, query: String },
    /// The rows for `<keyword> <query>` typed in root search.
    Keyword { keyword: String, query: String },
    /// Run action `action` of item `item`, shown in `view` (None: root search).
    Act { view: Option<String>, item: String, action: String },
    /// The form `form`.
    Form { form: String },
    /// Submit the form `form` with each field's text.
    Submit { form: String, values: Vec<(String, String)> },
    /// Something happened in Flick.
    Event(EventKind),
}

impl Request {
    pub fn op(&self) -> Op {
        match self {
            Request::Root => Op::Root,
            Request::Open { .. } => Op::Open,
            Request::Query { .. } => Op::Query,
            Request::Keyword { .. } => Op::Keyword,
            Request::Act { .. } => Op::Act,
            Request::Form { .. } => Op::Form,
            Request::Submit { .. } => Op::Submit,
            Request::Event(_) => Op::Event,
        }
    }

    /// The JSON the extension reads on stdin.
    pub fn to_json(&self) -> String {
        let mut o = Map::new();
        o.insert("v".into(), json!(VERSION));
        o.insert("op".into(), json!(self.op().name()));
        let keys = match self {
            Request::Root => vec![],
            Request::Open { view } => vec![("view", json!(view))],
            Request::Form { form } => vec![("form", json!(form))],
            Request::Query { view, query } => vec![("view", json!(view)), ("query", json!(query))],
            Request::Keyword { keyword, query } => {
                vec![("keyword", json!(keyword)), ("query", json!(query))]
            }
            Request::Act { view, item, action } => {
                vec![("view", json!(view)), ("item", json!(item)), ("action", json!(action))]
            }
            Request::Submit { form, values } => {
                let values: Map<String, Value> =
                    values.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
                vec![("form", json!(form)), ("values", Value::Object(values))]
            }
            Request::Event(kind) => vec![("event", json!(kind.name()))],
        };
        o.extend(keys.into_iter().map(|(k, v)| (k.to_string(), v)));
        Value::Object(o).to_string()
    }
}

/// How a view's list is searched: by Flick over the items it has (`Flick`), or by the
/// extension, with an `op: query` per keystroke (`Ext`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    Flick,
    Ext,
}

/// The reply to `open`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub title: Option<String>,
    pub placeholder: Option<String>,
    pub filter: Filter,
    pub items: Vec<ExtItem>,
}

/// The reply to `act` or `submit` that did not fail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Done {
    /// A line for the footer or a toast.
    pub status: Option<String>,
    /// Refetch the root items and the open view.
    pub refresh: bool,
}

/// A parsed reply, by the op it answers.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// `root`, `query`, `keyword`.
    Items(Vec<ExtItem>),
    View(View),
    Form(ExtForm),
    /// `act`, `submit`.
    Done(Done),
    /// `event`.
    Ack,
}

/// A reply and what the parser changed or dropped.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub reply: Reply,
    pub warnings: Vec<String>,
}

/// The extension's stdout `out`, as the reply to `op`. `Err` is the reason it cannot be
/// used: the reply's own `error`, or why it is malformed.
pub fn parse(op: Op, out: &[u8]) -> Result<Parsed, String> {
    if out.len() > REPLY_MAX {
        return Err(format!("reply is {} bytes, max {REPLY_MAX}", out.len()));
    }
    let v: Value = serde_json::from_slice(out).map_err(|e| format!("reply is not JSON: {e}"))?;
    let Value::Object(o) = v else {
        return Err("reply is not a JSON object".into());
    };
    let mut warnings = vec![];
    let w = &mut warnings;
    if let Some(error) = text(&o, "error", TEXT_MAX, "reply", w)?.filter(|e| !e.trim().is_empty()) {
        return Err(error);
    }
    if let Some(v) = o.get("v").filter(|v| v.as_u64() != Some(VERSION)) {
        w.push(format!("v {v} is not {VERSION}; read as v{VERSION}"));
    }
    let reply = match op {
        Op::Root | Op::Query | Op::Keyword => Reply::Items(item::parse_all(o.get("items"), w)?),
        Op::Open => Reply::View(View {
            title: text(&o, "title", TITLE_MAX, "view", w)?,
            placeholder: text(&o, "placeholder", SHORT_MAX, "view", w)?,
            filter: match o.get("filter").and_then(Value::as_str) {
                None | Some("flick") => Filter::Flick,
                Some("ext") => Filter::Ext,
                Some(f) => {
                    w.push(format!("unknown filter {f:?}; flick used"));
                    Filter::Flick
                }
            },
            items: item::parse_all(o.get("items"), w)?,
        }),
        Op::Form => Reply::Form(form::parse(&o, w)?),
        Op::Act | Op::Submit => Reply::Done(Done {
            status: text(&o, "status", TEXT_MAX, "reply", w)?,
            refresh: flag(&o, "refresh", "reply", w),
        }),
        Op::Event => Reply::Ack,
    };
    Ok(Parsed { reply, warnings })
}

/// A char that extension text may not hold: a control char (C0, DEL, C1) or an invisible
/// format char that could disguise text (zero-width, direction marks and overrides, line
/// and paragraph separators).
pub fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// `hidden`, except for newline and tab: for a multiline field value.
fn hidden_multiline(c: char) -> bool {
    hidden(c) && !matches!(c, '\n' | '\t')
}

/// The optional string `key` of `o` (absent or null: None), cut to `max` chars with a
/// warning. `Err` when it is not a string or holds a `hidden` char.
fn text(
    o: &Map<String, Value>,
    key: &str,
    max: usize,
    at: &str,
    w: &mut Vec<String>,
) -> Result<Option<String>, String> {
    text_where(o, key, max, at, w, hidden)
}

/// `text`, refusing the chars `bad` accepts.
fn text_where(
    o: &Map<String, Value>,
    key: &str,
    max: usize,
    at: &str,
    w: &mut Vec<String>,
    bad: fn(char) -> bool,
) -> Result<Option<String>, String> {
    let s = match o.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => s,
        Some(_) => return Err(format!("{key} is not a string")),
    };
    if s.chars().any(bad) {
        return Err(format!("{key} has a control character"));
    }
    if s.chars().count() <= max {
        return Ok(Some(s.clone()));
    }
    w.push(format!("{at}: {key} cut to {max} chars"));
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    Ok(Some(out))
}

/// The optional string `key` of `o` that must not be blank (a title or label).
fn required(
    o: &Map<String, Value>,
    key: &str,
    max: usize,
    at: &str,
    w: &mut Vec<String>,
) -> Result<String, String> {
    let s = text(o, key, max, at, w)?.filter(|s| !s.trim().is_empty());
    s.ok_or_else(|| format!("no {key}"))
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

/// The list `raw` (absent or null: empty), read entry by entry with `one`. An entry that
/// `one` refuses, one whose `key` repeats an earlier one, and those past `max` are dropped
/// with a warning. `Err` only when `raw` is not a list.
fn list<T>(
    raw: Option<&Value>,
    what: &str,
    max: usize,
    key: fn(&T) -> &str,
    one: fn(&Value, &str, &mut Vec<String>) -> Result<T, String>,
    w: &mut Vec<String>,
) -> Result<Vec<T>, String> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(format!("{what} is not a list")),
    };
    let mut out: Vec<T> = Vec::new();
    for (i, v) in raw.iter().enumerate() {
        let at = format!("{what}[{i}]");
        if out.len() == max {
            w.push(format!("more than {max} {what}; {at} and later dropped"));
            break;
        }
        match one(v, &at, w) {
            Ok(t) if out.iter().any(|u| key(u) == key(&t)) => {
                w.push(format!("{at}: duplicate key {:?}; skipped", key(&t)));
            }
            Ok(t) => out.push(t),
            Err(why) => w.push(format!("{at}: {why}; skipped")),
        }
    }
    Ok(out)
}
