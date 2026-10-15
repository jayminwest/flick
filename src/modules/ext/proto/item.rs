//! Items and their actions in a reply, and the action vocabulary (`do`).
//!
//! Item: `{"key": "a1", "title": "…", "subtitle": "…", "accessory": "…", "icon": …,
//! "keywords": ["…"], "tone": "neutral|ok|warn|error", "actions": [ … ], "tab": "<action key>"}`.
//! Only `key` and `title` are required. Enter runs the first action, ⌘K lists them all, Tab
//! runs the one `tab` names. An item without `actions` gets one: `Open`, `do: "ext"`.
//!
//! Icon: `"star.fill"` or `{"symbol": "star.fill"}` (an SF Symbol name), or
//! `{"file": "/abs/path.png"}` (an absolute path to an image file).
//!
//! Action: `{"key": "open", "title": "Open", "icon": …, "do": …, "after": "hide|stay|root",
//! "confirm": "Delete it?"}`. `do` is the card vocabulary (`core::card::Do`, checked by its
//! policy as a local card's) without `shell` (an extension runs commands itself, in `act`),
//! plus `"ext"` (send `op: act`, the default), `{"push": "<view>"}` and `{"form": "<form>"}`.
//! The card's `"dismiss"` (close the card) hides the launcher, whatever `after` says.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{
    ACTIONS_MAX, ITEMS_MAX, KEY_MAX, KEYWORDS_MAX, SHORT_MAX, TEXT_MAX, TITLE_MAX, hidden, list,
    required, text,
};
use crate::core::card::{Do, Origin, valid_id};
use crate::core::{Icon, Tone};

/// Longest SF Symbol name, in chars.
pub const SYMBOL_MAX: usize = 100;
/// Longest icon file path, in chars.
pub const PATH_MAX: usize = 1024;
/// File extensions an icon file may have (any case).
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "tiff", "heic", "webp", "icns", "pdf"];

/// One item of a reply.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtItem {
    /// The extension's id for it: 1-`KEY_MAX` chars, unique in the reply.
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub accessory: String,
    /// None: the module's default icon.
    pub icon: Option<Icon>,
    pub keywords: Vec<String>,
    pub tone: Tone,
    /// At least one; the first is what Enter does.
    pub actions: Vec<ExtAction>,
    /// The key of the action Tab runs.
    pub tab: Option<String>,
}

/// One action of an item.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtAction {
    /// The extension's name for it (`act` gets it back): 1-64 of A-Z a-z 0-9 . _ -
    pub key: String,
    pub title: String,
    pub icon: Option<Icon>,
    pub run: Run,
    pub after: After,
    /// Ask this first, with the action's title on the confirm button.
    pub confirm: Option<String>,
}

impl ExtAction {
    /// The action an item without `actions` gets.
    fn open() -> ExtAction {
        ExtAction {
            key: "open".into(),
            title: "Open".into(),
            icon: None,
            run: Run::Ext,
            after: After::Hide,
            confirm: None,
        }
    }
}

/// What an action does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    /// Send `op: act` to the extension.
    Ext,
    /// Open the extension's view of this name.
    Push(String),
    /// Open the extension's form of this name.
    Form(String),
    /// A card action, run by Flick; never `Do::Shell`.
    Card(Do),
}

/// What the launcher does as soon as an `ext` or card action starts, before any reply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum After {
    #[default]
    Hide,
    Stay,
    Root,
}

/// The `items` list of a reply.
pub(super) fn parse_all(raw: Option<&Value>, w: &mut Vec<String>) -> Result<Vec<ExtItem>, String> {
    list(raw, "items", ITEMS_MAX, |i| &i.key, one, w)
}

fn one(v: &Value, at: &str, w: &mut Vec<String>) -> Result<ExtItem, String> {
    let o = v.as_object().ok_or("not an object")?;
    let key = o.get("key").and_then(Value::as_str).filter(|k| valid_key(k));
    let key = key.ok_or_else(|| format!("key must be 1-{KEY_MAX} chars, no control chars"))?;
    let title = required(o, "title", TITLE_MAX, at, w)?;
    let subtitle = text(o, "subtitle", TEXT_MAX, at, w)?.unwrap_or_default();
    let accessory = text(o, "accessory", SHORT_MAX, at, w)?.unwrap_or_default();
    let keywords = keywords(o, at, w)?;
    let actions = match o.get("actions") {
        None | Some(Value::Null) => vec![ExtAction::open()],
        raw => list(raw, &format!("{at}.actions"), ACTIONS_MAX, |a| &a.key, action, w)?,
    };
    if actions.is_empty() {
        return Err("no valid action".into());
    }
    let tab = text(o, "tab", KEY_MAX, at, w)?;
    let tab = tab.filter(|t| {
        let known = actions.iter().any(|a| &a.key == t);
        if !known {
            w.push(format!("{at}: tab names no action {t:?}; ignored"));
        }
        known
    });
    Ok(ExtItem {
        key: key.to_string(),
        title,
        subtitle,
        accessory,
        icon: icon_of(o, at, w),
        keywords,
        tone: tone_of(o, at, w),
        actions,
        tab,
    })
}

fn valid_key(k: &str) -> bool {
    !k.trim().is_empty() && k.chars().count() <= KEY_MAX && !k.chars().any(hidden)
}

fn keywords(o: &Map<String, Value>, at: &str, w: &mut Vec<String>) -> Result<Vec<String>, String> {
    let raw = match o.get("keywords") {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(Value::Array(a)) => a,
        Some(_) => return Err("keywords is not a list".into()),
    };
    if raw.len() > KEYWORDS_MAX {
        w.push(format!("{at}: more than {KEYWORDS_MAX} keywords; the rest dropped"));
    }
    let words = raw.iter().take(KEYWORDS_MAX).map(|k| k.as_str().map(str::to_string));
    let words: Vec<String> = words.collect::<Option<_>>().ok_or("keywords must be strings")?;
    let bad = words.iter().any(|k| k.chars().count() > SHORT_MAX || k.chars().any(hidden));
    if bad {
        return Err(format!("keywords must be up to {SHORT_MAX} chars, no control chars"));
    }
    Ok(words)
}

fn action(v: &Value, at: &str, w: &mut Vec<String>) -> Result<ExtAction, String> {
    let o = v.as_object().ok_or("not an object")?;
    let key = o.get("key").and_then(Value::as_str).filter(|k| valid_id(k));
    let key = key.ok_or("key must be 1-64 of A-Z a-z 0-9 . _ -")?.to_string();
    let title = required(o, "title", TITLE_MAX, at, w)?;
    let run = match o.get("do") {
        None | Some(Value::Null) => Run::Ext,
        Some(raw) => run_of(raw)?,
    };
    let after = match o.get("after") {
        None | Some(Value::Null) => After::Hide,
        Some(a) => match a.as_str() {
            Some("hide") => After::Hide,
            Some("stay") => After::Stay,
            Some("root") => After::Root,
            _ => {
                w.push(format!("{at}: unknown after {a}; hide used"));
                After::Hide
            }
        },
    };
    let confirm = text(o, "confirm", TEXT_MAX, at, w)?.filter(|c| !c.trim().is_empty());
    Ok(ExtAction { key, title, icon: icon_of(o, at, w), run, after, confirm })
}

/// The `do` value `raw`, read and policy-checked.
pub fn run_of(raw: &Value) -> Result<Run, String> {
    let pair = match raw {
        Value::String(k) => Some((k.as_str(), &Value::Null)),
        Value::Object(o) if o.len() == 1 => o.iter().next().map(|(k, v)| (k.as_str(), v)),
        _ => None,
    };
    match pair {
        Some(("ext", _)) => Ok(Run::Ext),
        Some(("push", v)) => name_of(v, "push").map(Run::Push),
        Some(("form", v)) => name_of(v, "form").map(Run::Form),
        Some(("shell", _)) => Err("shell is not allowed: an extension runs commands in act".into()),
        _ => {
            let run = Do::parse(raw)?;
            run.check(Origin::Local)?;
            Ok(Run::Card(run))
        }
    }
}

/// A view or form name: 1-64 of A-Z a-z 0-9 . _ -
fn name_of(v: &Value, what: &str) -> Result<String, String> {
    let name = v.as_str().filter(|n| valid_id(n)).map(str::to_string);
    name.ok_or_else(|| format!("{what} needs a name of 1-64 of A-Z a-z 0-9 . _ -"))
}

/// The `icon` key: None, with a warning, when it is not one Flick can show.
fn icon_of(o: &Map<String, Value>, at: &str, w: &mut Vec<String>) -> Option<Icon> {
    let raw = o.get("icon").filter(|v| !v.is_null())?;
    let icon = match raw {
        Value::String(s) => symbol(s),
        Value::Object(map) if map.len() == 1 => match map.iter().next() {
            Some((kind, Value::String(name))) if kind == "symbol" => symbol(name),
            Some((kind, Value::String(path))) if kind == "file" => file(path),
            _ => None,
        },
        _ => None,
    };
    if icon.is_none() {
        w.push(format!("{at}: icon {raw} is not a symbol name or an image path; ignored"));
    }
    icon
}

fn symbol(s: &str) -> Option<Icon> {
    let ok = (1..=SYMBOL_MAX).contains(&s.len())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.');
    ok.then(|| Icon::symbol(s.to_string()))
}

/// An absolute path to an image file, by its extension. Whether it exists is the
/// module's check (I/O).
fn file(s: &str) -> Option<Icon> {
    let path = Path::new(s);
    let image = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGES.contains(&e.to_ascii_lowercase().as_str()));
    let ok = image && path.is_absolute() && s.chars().count() <= PATH_MAX && !s.chars().any(hidden);
    ok.then(|| Icon::File(PathBuf::from(s)))
}

fn tone_of(o: &Map<String, Value>, at: &str, w: &mut Vec<String>) -> Tone {
    match o.get("tone") {
        None | Some(Value::Null) => Tone::Neutral,
        Some(t) => match t.as_str() {
            Some("neutral") => Tone::Neutral,
            Some("ok") => Tone::Ok,
            Some("warn") => Tone::Warn,
            Some("error") => Tone::Error,
            _ => {
                w.push(format!("{at}: unknown tone {t}; neutral used"));
                Tone::Neutral
            }
        },
    }
}
