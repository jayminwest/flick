//! Cards: structured messages a peer (KOTA) posts as JSON. This is the shared, pure model:
//! the schema (v1), the parser with its caps and degrade rules, the normalized serializer
//! the message store keeps, and the plain-text fallback. The `message` module posts and
//! stores cards, `platform::hud` renders them, and the chat window and status item reuse
//! the same types, so they live in core rather than in a module (plan flick-7da1).
//!
//! `parse` is strict only where nothing sensible can be shown: not JSON, not an object,
//! over `MAX_BYTES`, a missing or invalid `id` or `title`, a `v` that is not a positive
//! integer, or an unknown `state`. Everything else degrades and adds a warning for the
//! sender: an unknown or malformed block becomes a text block, over-cap text is cut, over-cap
//! lists end in a `…N more` item, and an action whose `do` is unknown or refused by policy
//! stays on the card, disabled, with the reason. The local-action vocabulary (`do`) and the
//! values JSON sent back with a press (`action::values_json`) are in `action`.
//!
//! Wire shape (every key not listed is ignored with a warning):
//!
//! ```json
//! {"v": 1, "id": "deploy-42", "title": "Deploy?", "state": "open",
//!  "thread": "ops", "reply_to": "k-123",
//!  "blocks": [{"type": "text", "md": "Ready to ship **v2**."}],
//!  "actions": [{"id": "go", "label": "Ship", "style": "primary"}]}
//! ```

pub mod action;
mod block;
#[cfg(test)]
mod tests;

pub use action::{Do, Input};

use serde::Serialize;
use serde::ser::Serializer;
use serde_json::{Map, Value};

/// The schema version this Flick understands. A card may declare a higher `v`; it is read
/// as v1 with a warning.
pub const VERSION: u64 = 1;
/// Largest card, in bytes of JSON. Over it, `parse` refuses the card.
pub const MAX_BYTES: usize = 16 * 1024;
/// Longest id (card, thread, `reply_to`, block input and action ids), in chars.
pub const ID_MAX: usize = 64;
/// Longest title, in chars; a longer one is cut.
pub const TITLE_MAX: usize = 120;
/// Most blocks; past it the card ends in a `…N more` text block.
pub const BLOCKS_MAX: usize = 24;
/// Longest text block `md`, in chars; a longer one is cut.
pub const MD_MAX: usize = 4000;
/// Most `kv` or `list` items, counting the trailing `…N more` item.
pub const ITEMS_MAX: usize = 20;
/// Most options in a `choice`; extra options are dropped.
pub const OPTIONS_MAX: usize = 12;
/// Most `field` blocks; extra fields are dropped.
pub const FIELDS_MAX: usize = 4;
/// Longest field value, in chars (the initial `value` and what the user types).
pub const INPUT_MAX: usize = 2000;
/// Most actions; extra actions are dropped.
pub const ACTIONS_MAX: usize = 6;
/// Longest action label, in chars; a longer one is cut.
pub const LABEL_MAX: usize = 40;
/// Longest values JSON sent back with a press, in chars (`values_json` refuses more).
pub const VALUES_MAX: usize = 4000;
/// Longest JSON preview shown for a block that degrades to text, in chars.
pub const PREVIEW_MAX: usize = 300;

/// Top-level keys a v1 card may carry.
const KEYS: &[&str] = &["v", "id", "title", "state", "thread", "reply_to", "blocks", "actions"];

/// Where a card came from. A `Remote` card (posted over the network, `Cx::remote`) may not
/// carry a `flick` action that `core::control::net_policy` refuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Local,
    Remote,
}

/// A parsed card and what the parser changed or dropped, for the sender (`--json` reply).
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub card: Card,
    pub warnings: Vec<String>,
}

/// One card. Build it with `parse`; `to_json` gives the normalized JSON that parses back to
/// an equal card.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Card {
    /// `v`: the schema version the sender declared (default 1).
    pub v: u64,
    /// `id`: 1 to 64 of `A-Z a-z 0-9 . _ -`, the message id. Posting the same id again
    /// replaces the card. Required.
    pub id: String,
    /// `title`: the header line, at most `TITLE_MAX` chars. Required, not blank.
    pub title: String,
    /// `state`: `open` (default), `pending` (KOTA is working on a press), `done` or `error`.
    pub state: State,
    /// `thread`: optional conversation id (same charset as `id`) the chat window groups
    /// cards by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// `reply_to`: optional id of a pending message this card answers, as
    /// `message post --reply-to` (same charset as `id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// `blocks`: the body, top to bottom, at most `BLOCKS_MAX`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<Block>,
    /// `actions`: buttons, left to right, at most `ACTIONS_MAX`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
}

/// A card's lifecycle state, set by the sender.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Shown and actionable.
    #[default]
    Open,
    /// A press went to KOTA; actions are disabled until it re-posts the card.
    Pending,
    /// Finished; actions are disabled.
    Done,
    /// Something failed; actions stay enabled so the user can retry.
    Error,
}

/// One body block, tagged by `type` on the wire.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Block {
    /// `{"type":"text","md":"…"}`: markdown-lite text, at most `MD_MAX` chars.
    Text { md: String },
    /// `{"type":"kv","items":[{"key":"Due","value":"Fri"}]}`: key/value rows, at most
    /// `ITEMS_MAX`. Number and boolean values become strings.
    Kv { items: Vec<Pair> },
    /// `{"type":"list","items":["a","b"],"ordered":false}`: bullet (or numbered) rows, at
    /// most `ITEMS_MAX`. Number and boolean items become strings.
    List {
        items: Vec<String>,
        #[serde(skip_serializing_if = "is_false")]
        ordered: bool,
    },
    /// `{"type":"progress","value":0.4,"label":"Uploading"}`: a bar. `value` is 0 to 1
    /// (clamped); without it the bar is indeterminate.
    Progress {
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// `{"type":"choice","id":"env","label":"Where","options":["prod",{"id":"stg",
    /// "label":"Staging"}],"multi":false,"selected":"prod"}`: pick one (or several with
    /// `multi`) of at most `OPTIONS_MAX` options. A string option is its own id and label.
    /// `selected` is an option id or a list of them. Its value goes back under `id`.
    Choice {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        options: Vec<Opt>,
        #[serde(skip_serializing_if = "is_false")]
        multi: bool,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        selected: Vec<String>,
    },
    /// `{"type":"field","id":"note","label":"Note","placeholder":"…","value":"",
    /// "multiline":false}`: a text input (at most `FIELDS_MAX` per card, `INPUT_MAX` chars).
    /// Its text goes back under `id`.
    Field {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(skip_serializing_if = "String::is_empty")]
        value: String,
        #[serde(skip_serializing_if = "is_false")]
        multiline: bool,
    },
}

/// One `kv` row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Pair {
    pub key: String,
    pub value: String,
}

/// One `choice` option: `id` goes back in the values, `label` is shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Opt {
    pub id: String,
    pub label: String,
}

/// One button. On the wire: `{"id":"go","label":"Ship","style":"primary","do":{…},
/// "reply":true}`.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    /// `id`: same charset as the card id, unique on the card; sent back as `--action-id`.
    pub id: String,
    /// `label`: the button text, at most `LABEL_MAX` chars. Required.
    pub label: String,
    /// `style`: `default`, `primary` (⌘↵) or `destructive`; unknown styles are `default`.
    pub style: Style,
    /// What a press does, from `do` and `reply`.
    pub kind: Kind,
}

/// Button look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    #[default]
    Default,
    Primary,
    Destructive,
}

/// What pressing an action does.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// No `do`: send the press (card id, action id, values JSON) to KOTA.
    Reply,
    /// A local action; with `reply` the press also goes to KOTA.
    Local { run: Do, reply: bool },
    /// A `do` that is unknown, malformed or refused by policy: shown disabled, `reason` as
    /// its tooltip. `raw` is the original `do`, kept so the normalized JSON re-parses the
    /// same way.
    Disabled { raw: Value, reply: bool, reason: String },
}

impl Action {
    /// Whether the button can be pressed.
    pub fn enabled(&self) -> bool {
        !matches!(self.kind, Kind::Disabled { .. })
    }

    /// Whether a press is sent to KOTA.
    pub fn replies(&self) -> bool {
        match self.kind {
            Kind::Reply => true,
            Kind::Local { reply, .. } => reply,
            Kind::Disabled { .. } => false,
        }
    }
}

/// The wire form of an `Action`.
#[derive(Serialize)]
struct WireAction<'a> {
    id: &'a str,
    label: &'a str,
    #[serde(skip_serializing_if = "is_default_style")]
    style: Style,
    #[serde(rename = "do", skip_serializing_if = "Option::is_none")]
    run: Option<Value>,
    #[serde(skip_serializing_if = "is_false")]
    reply: bool,
}

impl Serialize for Action {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (run, reply) = match &self.kind {
            Kind::Reply => (None, false),
            Kind::Local { run, reply } => (Some(run.to_value()), *reply),
            Kind::Disabled { raw, reply, .. } => (Some(raw.clone()), *reply),
        };
        WireAction { id: &self.id, label: &self.label, style: self.style, run, reply }.serialize(s)
    }
}

impl Card {
    /// Whether the card waits on the user: open, with at least one enabled action. The
    /// status item badges these.
    pub fn waits_on_user(&self) -> bool {
        self.state == State::Open && self.actions.iter().any(Action::enabled)
    }

    /// The action ⌘↵ presses: the first enabled `primary` one.
    pub fn primary(&self) -> Option<&Action> {
        self.actions.iter().find(|a| a.enabled() && a.style == Style::Primary)
    }

    /// The initial value of every input (fields and choices), in card order: what
    /// `values_json` sends if the user changes nothing.
    pub fn inputs(&self) -> Vec<(String, Input)> {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Field { id, value, .. } => Some((id.clone(), Input::Text(value.clone()))),
                Block::Choice { id, multi: true, selected, .. } => {
                    Some((id.clone(), Input::Many(selected.clone())))
                }
                Block::Choice { id, selected, .. } => {
                    Some((id.clone(), Input::One(selected.first().cloned())))
                }
                _ => None,
            })
            .collect()
    }
}

/// Parse a card posted from `origin`. `Err` is a structural error (see the module docs):
/// show nothing and tell the sender.
pub fn parse(json: &str, origin: Origin) -> Result<Parsed, String> {
    if json.len() > MAX_BYTES {
        return Err(format!("card is {} bytes, max {MAX_BYTES}", json.len()));
    }
    let value: Value = serde_json::from_str(json).map_err(|e| format!("card is not JSON: {e}"))?;
    let Value::Object(obj) = value else {
        return Err("card must be a JSON object".into());
    };
    let mut warnings = Vec::new();
    let w = &mut warnings;
    let id = match obj.get("id") {
        Some(Value::String(id)) if valid_id(id) => id.clone(),
        Some(_) => return Err(format!("id must be 1-{ID_MAX} of A-Z a-z 0-9 . _ -")),
        None => return Err("card has no id".into()),
    };
    let title = match obj.get("title") {
        Some(Value::String(t)) if !t.trim().is_empty() => {
            clip_warn(t.trim(), TITLE_MAX, "title", w)
        }
        _ => return Err("card needs a non-empty title string".into()),
    };
    let v = match obj.get("v") {
        None => VERSION,
        Some(n) => n.as_u64().filter(|v| *v >= 1).ok_or("v must be a positive integer")?,
    };
    if v > VERSION {
        w.push(format!("v {v} is newer than this Flick (v{VERSION}); read as v{VERSION}"));
    }
    let state = match obj.get("state") {
        None => State::Open,
        Some(s) => {
            s.as_str().and_then(state_of).ok_or("state must be open, pending, done or error")?
        }
    };
    let thread = opt_id(&obj, "thread", w);
    let reply_to = opt_id(&obj, "reply_to", w);
    let blocks = array(&obj, "blocks", w).map_or_else(Vec::new, |raw| block::parse_all(raw, w));
    let actions =
        array(&obj, "actions", w).map_or_else(Vec::new, |raw| action::parse_all(raw, origin, w));
    for key in obj.keys().filter(|k| !KEYS.contains(&k.as_str())) {
        w.push(format!("unknown key {key:?} ignored"));
    }
    let card = Card { v, id, title, state, thread, reply_to, blocks, actions };
    Ok(Parsed { card, warnings })
}

/// The normalized JSON of `card`: what the message store keeps. `parse` of it (with the
/// same origin) gives back an equal card.
pub fn to_json(card: &Card) -> String {
    serde_json::to_string(card).unwrap_or_default()
}

/// The card as plain text, one line per row: the title, each block, then the action labels
/// in brackets. History rows, `ls`, notifications and the chat transcript use it.
pub fn plain(card: &Card) -> String {
    let mut lines = vec![card.title.clone()];
    for b in &card.blocks {
        block::plain(b, &mut lines);
    }
    if !card.actions.is_empty() {
        let labels: Vec<String> = card.actions.iter().map(|a| format!("[{}]", a.label)).collect();
        lines.push(labels.join(" "));
    }
    lines.join("\n")
}

/// Whether `id` is a valid card, thread, input or action id: 1 to `ID_MAX` of
/// `A-Z a-z 0-9 . _ -` (the message id space).
pub fn valid_id(id: &str) -> bool {
    (1..=ID_MAX).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `s` cut to `max` chars, the last one `…`, and whether it was cut.
fn clip(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    (out, true)
}

/// `clip`, with a warning naming `what` when it cut.
fn clip_warn(s: &str, max: usize, what: &str, w: &mut Vec<String>) -> String {
    let (out, cut) = clip(s, max);
    if cut {
        w.push(format!("{what} cut to {max} chars"));
    }
    out
}

fn state_of(s: &str) -> Option<State> {
    match s {
        "open" => Some(State::Open),
        "pending" => Some(State::Pending),
        "done" => Some(State::Done),
        "error" => Some(State::Error),
        _ => None,
    }
}

/// An optional id-valued key: absent or null is `None`; an invalid one is dropped with a
/// warning.
fn opt_id(obj: &Map<String, Value>, key: &str, w: &mut Vec<String>) -> Option<String> {
    match obj.get(key)? {
        Value::Null => None,
        Value::String(s) if valid_id(s) => Some(s.clone()),
        _ => {
            w.push(format!("{key} is not a valid id; ignored"));
            None
        }
    }
}

/// An optional array-valued key; anything else is ignored with a warning.
fn array<'a>(obj: &'a Map<String, Value>, key: &str, w: &mut Vec<String>) -> Option<&'a [Value]> {
    match obj.get(key)? {
        Value::Array(a) => Some(a),
        Value::Null => None,
        _ => {
            w.push(format!("{key} must be an array; ignored"));
            None
        }
    }
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if passes &T")]
fn is_false(b: &bool) -> bool {
    !*b
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if passes &T")]
fn is_default_style(s: &Style) -> bool {
    *s == Style::Default
}
