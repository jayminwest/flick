//! Block parsing and the plain-text form of each block. A block that is not an object, has
//! an unknown `type` or has bad fields degrades to a text block (`degrade`) with a warning.

use serde_json::{Map, Value};

use super::{
    BLOCKS_MAX, Block, FIELDS_MAX, INPUT_MAX, ITEMS_MAX, MD_MAX, OPTIONS_MAX, Opt, PREVIEW_MAX,
    Pair, clip, clip_warn, valid_id,
};

/// What parsing one block needs to know about the blocks before it.
#[derive(Default)]
struct Seen {
    /// Input (choice and field) ids so far; they must be unique.
    inputs: Vec<String>,
    /// Field blocks kept so far.
    fields: usize,
}

/// Parse `raw` blocks; past `BLOCKS_MAX` the rest collapse into one `…N more` text block.
pub(super) fn parse_all(raw: &[Value], w: &mut Vec<String>) -> Vec<Block> {
    let keep = if raw.len() > BLOCKS_MAX { BLOCKS_MAX - 1 } else { raw.len() };
    let mut seen = Seen::default();
    let mut blocks: Vec<Block> =
        raw[..keep].iter().enumerate().filter_map(|(i, v)| one(i, v, &mut seen, w)).collect();
    if keep < raw.len() {
        let more = raw.len() - keep;
        w.push(format!("more than {BLOCKS_MAX} blocks; last {more} dropped"));
        blocks.push(Block::Text { md: format!("…{more} more") });
    }
    blocks
}

/// Block `i`; `None` when it is dropped (a field over `FIELDS_MAX`).
fn one(i: usize, v: &Value, seen: &mut Seen, w: &mut Vec<String>) -> Option<Block> {
    let Value::Object(o) = v else {
        w.push(format!("blocks[{i}]: not an object; shown as text"));
        return Some(degrade("block", v));
    };
    let kind = o.get("type").and_then(Value::as_str).unwrap_or("?");
    let at = format!("blocks[{i}] ({kind})");
    let parsed = match kind {
        "text" => text(o, &at, w),
        "kv" => kv(o, &at, w),
        "list" => list(o, &at, w),
        "progress" => progress(o, &at, w),
        "choice" => choice(o, seen, &at, w),
        "field" if seen.fields >= FIELDS_MAX => {
            w.push(format!("{at}: more than {FIELDS_MAX} fields; dropped"));
            return None;
        }
        "field" => field(o, seen, &at, w),
        _ => Err("unknown type".into()),
    };
    Some(parsed.unwrap_or_else(|why| {
        w.push(format!("{at}: {why}; shown as text"));
        degrade(kind, v)
    }))
}

/// A text block standing in for a block Flick cannot show: `[<kind>] ` and the block's
/// `md` or `text` string, else a compact JSON preview of at most `PREVIEW_MAX` chars.
fn degrade(kind: &str, v: &Value) -> Block {
    let body = ["md", "text"].iter().find_map(|k| v.get(k).and_then(Value::as_str));
    let body = body.map_or_else(|| clip(&v.to_string(), PREVIEW_MAX).0, str::to_string);
    Block::Text { md: clip(&format!("[{kind}] {body}"), MD_MAX).0 }
}

fn text(o: &Map<String, Value>, at: &str, w: &mut Vec<String>) -> Result<Block, String> {
    let md = req_str(o, "md")?;
    Ok(Block::Text { md: clip_warn(md, MD_MAX, &format!("{at}: md"), w) })
}

fn kv(o: &Map<String, Value>, at: &str, w: &mut Vec<String>) -> Result<Block, String> {
    let items = req_array(o, "items")?
        .iter()
        .map(|item| {
            let key = item.get("key").and_then(scalar).ok_or("item without a key string")?;
            let value = item.get("value").and_then(scalar).ok_or("item without a value")?;
            Ok(Pair { key, value })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let items = cap(items, at, w, |n| Pair { key: format!("…{n} more"), value: String::new() });
    Ok(Block::Kv { items })
}

fn list(o: &Map<String, Value>, at: &str, w: &mut Vec<String>) -> Result<Block, String> {
    let items = req_array(o, "items")?
        .iter()
        .map(|item| scalar(item).ok_or("item is not a string"))
        .collect::<Result<Vec<_>, _>>()?;
    let items = cap(items, at, w, |n| format!("…{n} more"));
    Ok(Block::List { items, ordered: opt_bool(o, "ordered")? })
}

fn progress(o: &Map<String, Value>, at: &str, w: &mut Vec<String>) -> Result<Block, String> {
    let value = match o.get("value") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_f64().ok_or("value is not a number")?),
    };
    let value = value.map(|v| {
        if !(0.0..=1.0).contains(&v) {
            w.push(format!("{at}: value {v} clamped to 0..1"));
        }
        v.clamp(0.0, 1.0)
    });
    Ok(Block::Progress { value, label: opt_str(o, "label")? })
}

fn choice(
    o: &Map<String, Value>,
    seen: &mut Seen,
    at: &str,
    w: &mut Vec<String>,
) -> Result<Block, String> {
    let id = input_id(o, seen)?;
    let mut options: Vec<Opt> = Vec::new();
    for raw in req_array(o, "options")? {
        let opt = match raw {
            Value::String(s) => Opt { id: s.clone(), label: s.clone() },
            Value::Object(_) => {
                let id = raw.get("id").and_then(Value::as_str).ok_or("option without an id")?;
                let label = raw.get("label").and_then(Value::as_str).unwrap_or(id);
                Opt { id: id.to_string(), label: label.to_string() }
            }
            _ => return Err("option is not a string or object".into()),
        };
        if opt.id.is_empty() || options.iter().any(|o| o.id == opt.id) {
            return Err(format!("empty or duplicate option id {:?}", opt.id));
        }
        options.push(opt);
    }
    if options.is_empty() {
        return Err("no options".into());
    }
    if options.len() > OPTIONS_MAX {
        w.push(format!(
            "{at}: more than {OPTIONS_MAX} options; last {} dropped",
            options.len() - OPTIONS_MAX
        ));
        options.truncate(OPTIONS_MAX);
    }
    let multi = opt_bool(o, "multi")?;
    let mut selected = match o.get("selected") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .map(|s| s.as_str().map(str::to_string).ok_or("selected is not a string"))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("selected is not a string or list".into()),
    };
    let before = selected.len();
    selected.retain(|s| options.iter().any(|o| o.id == *s));
    if !multi {
        selected.truncate(1);
    }
    if selected.len() < before {
        w.push(format!("{at}: unknown or extra selected options dropped"));
    }
    seen.inputs.push(id.clone());
    Ok(Block::Choice { id, label: opt_str(o, "label")?, options, multi, selected })
}

fn field(
    o: &Map<String, Value>,
    seen: &mut Seen,
    at: &str,
    w: &mut Vec<String>,
) -> Result<Block, String> {
    let id = input_id(o, seen)?;
    let label = opt_str(o, "label")?;
    let placeholder = opt_str(o, "placeholder")?;
    let value = opt_str(o, "value")?.unwrap_or_default();
    let value = clip_warn(&value, INPUT_MAX, &format!("{at}: value"), w);
    let multiline = opt_bool(o, "multiline")?;
    seen.inputs.push(id.clone());
    seen.fields += 1;
    Ok(Block::Field { id, label, placeholder, value, multiline })
}

/// The `id` of a choice or field: valid and not used by an earlier input.
fn input_id(o: &Map<String, Value>, seen: &Seen) -> Result<String, String> {
    let id = o.get("id").and_then(Value::as_str).filter(|id| valid_id(id));
    let id = id.ok_or("missing or invalid id")?;
    if seen.inputs.iter().any(|s| s == id) {
        return Err(format!("duplicate input id {id:?}"));
    }
    Ok(id.to_string())
}

/// `items` cut to `ITEMS_MAX`, the last one a `…N more` item made by `more(n)`.
fn cap<T>(
    mut items: Vec<T>,
    at: &str,
    w: &mut Vec<String>,
    more: impl FnOnce(usize) -> T,
) -> Vec<T> {
    if items.len() > ITEMS_MAX {
        let n = items.len() - (ITEMS_MAX - 1);
        w.push(format!("{at}: more than {ITEMS_MAX} items; last {n} folded into one"));
        items.truncate(ITEMS_MAX - 1);
        items.push(more(n));
    }
    items
}

/// A string, number or boolean as text.
fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn req_str<'a>(o: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    o.get(key).and_then(Value::as_str).ok_or_else(|| format!("{key} is not a string"))
}

fn req_array<'a>(o: &'a Map<String, Value>, key: &str) -> Result<&'a [Value], String> {
    o.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{key} is not a list"))
}

fn opt_str(o: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{key} is not a string")),
    }
}

fn opt_bool(o: &Map<String, Value>, key: &str) -> Result<bool, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{key} is not true or false")),
    }
}

/// Append `b` as plain-text lines.
pub(super) fn plain(b: &Block, lines: &mut Vec<String>) {
    match b {
        Block::Text { md } => lines.push(md.clone()),
        Block::Kv { items } => {
            lines.extend(items.iter().map(|p| format!("{}: {}", p.key, p.value)));
        }
        Block::List { items, ordered } => lines.extend(
            items
                .iter()
                .enumerate()
                .map(|(i, s)| if *ordered { format!("{}. {s}", i + 1) } else { format!("• {s}") }),
        ),
        Block::Progress { value, label } => {
            let pct =
                value.map_or_else(|| "…".to_string(), |v| format!("{}%", (v * 100.0).round()));
            lines.push(format!("{}: {pct}", label.as_deref().unwrap_or("Progress")));
        }
        Block::Choice { id, label, options, selected, .. } => {
            let shown: Vec<&str> = options
                .iter()
                .filter(|o| selected.is_empty() || selected.contains(&o.id))
                .map(|o| o.label.as_str())
                .collect();
            let sep = if selected.is_empty() { " / " } else { ", " };
            lines.push(format!("{}: {}", label.as_deref().unwrap_or(id), shown.join(sep)));
        }
        Block::Field { id, label, value, .. } => {
            lines.push(
                format!("{}: {value}", label.as_deref().unwrap_or(id)).trim_end().to_string(),
            );
        }
    }
}
