//! What a chat ask sends to KOTA (flick-2943): the question with a `[context]` block of the
//! chips the user kept, and the ssh argv that runs kota-ask with `--id` and `--thread`.
//!
//! The text goes on ssh's stdin, never in an argv (ssh would re-parse it through a remote
//! shell): `/usr/bin/ssh -o BatchMode=yes -o ConnectTimeout=8 <host> <kota_ask> --id <req>
//! --thread <t>`, the same call as the kota module's quick ask (copied, not imported) plus
//! `--thread`.
//!
//! The block, after the question and a blank line; each item at most once, in the order
//! given, inside `CONTEXT_MAX` bytes:
//!
//! ```text
//! [context]
//! app: Safari (com.apple.Safari)
//! window: Inbox - Fastmail
//! screenshot: ~/.cache/flick/attach/k1abc-1.png
//! selection:
//!   every line of the text,
//!   indented by two spaces
//! clipboard (cut):
//!   …
//! [/context]
//! ```
//!
//! One-line items are flattened to one line and cut to `LINE_MAX` bytes. Text items are cut
//! to `ITEM_MAX` bytes and to what is left of `CONTEXT_MAX`; a cut one says `(cut)`, one with
//! no room left says `(omitted: context limit)`. Indenting keeps text from closing the
//! block early. The screenshot path is relative to the remote home (`attach.rs`).

use std::fmt::Write as _;
use std::time::Duration;

use super::attach;
use crate::core::card::valid_id;
use crate::modules::message::text;

/// Budget of the ssh child (its own `ConnectTimeout` is 8 s).
pub const BUDGET: Duration = Duration::from_secs(20);
/// The longest question, in characters (kota-ask caps prompts at 2000).
pub const QUESTION_MAX: usize = 2000;
/// Bytes of a one-line item (app, window, screenshot).
pub const LINE_MAX: usize = 256;
/// Bytes of one text item (selection, clipboard), as indented.
pub const ITEM_MAX: usize = 4 * 1024;
/// Bytes of the whole `[context]` block, both tags included.
pub const CONTEXT_MAX: usize = 8 * 1024;
/// Bytes kota-ask reads from stdin (`head -c 16384`); a composed ask always fits.
pub const STDIN_MAX: usize = 16 * 1024;
/// The default ssh target and kota-ask path, as the kota module's (`[kota] ssh`, `kota_ask`).
pub const DEFAULT_HOST: &str = "jaymin@mbp-server";
pub const DEFAULT_KOTA_ASK: &str = ".dotfiles/home/.local/bin/kota-ask";

/// The least of a cut text item worth sending, in bytes.
const MIN_CUT: usize = 64;

const OPEN: &str = "[context]\n";
const CLOSE: &str = "[/context]";

/// One piece of context the user attached; each shows as a removable chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// The front app at summon.
    App { name: String, bundle: Option<String> },
    /// Its focused window's title.
    Window(String),
    /// The selected text in it.
    Selection(String),
    /// The clipboard text (⌘⇧V).
    Clipboard(String),
    /// A screenshot already uploaded: its path under the remote home (`attach::path`).
    Screenshot(String),
}

/// A chip above the input: its label and SF Symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub label: String,
    pub symbol: &'static str,
}

/// Characters of a chip label.
const CHIP_MAX: usize = 32;

impl Item {
    pub fn chip(&self) -> Chip {
        let short = |s: &str| text::preview(s, CHIP_MAX);
        let (label, symbol) = match self {
            Item::App { name, .. } => (short(name), "app.dashed"),
            Item::Window(title) => (short(title), "macwindow"),
            Item::Selection(s) => (format!("“{}”", text::preview(s, CHIP_MAX - 2)), "text.quote"),
            Item::Clipboard(_) => ("Clipboard".into(), "doc.on.clipboard"),
            Item::Screenshot(_) => ("Screenshot".into(), "camera.viewfinder"),
        };
        Chip { label, symbol }
    }

    /// Its key in the block, its text, and whether that is one line (else a text item).
    fn parts(&self) -> (&'static str, String, bool) {
        match self {
            Item::App { name, bundle: Some(b) } => ("app", format!("{name} ({b})"), true),
            Item::App { name, bundle: None } => ("app", name.clone(), true),
            Item::Window(t) => ("window", t.clone(), true),
            Item::Screenshot(p) => ("screenshot", format!("~/{p}"), true),
            Item::Selection(s) => ("selection", s.clone(), false),
            Item::Clipboard(s) => ("clipboard", s.clone(), false),
        }
    }
}

/// The question to send: trimmed, not empty, at most `QUESTION_MAX` characters.
pub fn check(question: &str) -> Result<String, String> {
    let q = question.trim();
    if q.is_empty() {
        return Err("Type a question first".into());
    }
    if q.chars().count() > QUESTION_MAX {
        return Err(format!("Over {QUESTION_MAX} characters"));
    }
    Ok(q.to_string())
}

/// What goes on kota-ask's stdin: the checked question, then the block if any item has text.
pub fn compose(question: &str, items: &[Item]) -> Result<String, String> {
    let q = check(question)?;
    Ok(match block(items) {
        Some(b) => format!("{q}\n\n{b}"),
        None => q,
    })
}

/// The `[context]` block of `items`; `None` when none has text.
pub fn block(items: &[Item]) -> Option<String> {
    let mut s = String::from(OPEN);
    for item in items {
        let room = CONTEXT_MAX - s.len() - CLOSE.len();
        s.push_str(&render(item, room));
    }
    (s.len() > OPEN.len()).then(|| s + CLOSE)
}

/// One item's lines, at most `room` bytes: the item, cut if need be, else the omitted line
/// if that fits, else "" (also for an item without text).
fn render(item: &Item, room: usize) -> String {
    let (key, value, one_line) = item.parts();
    if value.trim().is_empty() {
        return String::new();
    }
    let text = if one_line { Some(line(key, &value)) } else { lines(key, &value, ITEM_MAX.min(room)) };
    match text {
        Some(t) if t.len() <= room => t,
        _ => {
            let omitted = format!("{key}: (omitted: context limit)\n");
            if omitted.len() <= room { omitted } else { String::new() }
        }
    }
}

/// `key: value` on one line, at most `LINE_MAX` bytes.
fn line(key: &str, value: &str) -> String {
    let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{}\n", clip(&format!("{key}: {flat}"), LINE_MAX - 1))
}

/// `key:` and the text indented, in `cap` bytes; cut and marked when it does not fit, `None`
/// when less than `MIN_CUT` bytes of it would.
fn lines(key: &str, value: &str, cap: usize) -> Option<String> {
    let body = value.trim_end().lines().fold(String::new(), |mut b, l| {
        let _ = writeln!(b, "  {l}");
        b
    });
    let whole = format!("{key}:\n");
    if whole.len() + body.len() <= cap {
        return Some(whole + &body);
    }
    let head = format!("{key} (cut):\n");
    let cut = clip(&body, cap.saturating_sub(head.len() + 1)).trim_end();
    (cut.len() >= MIN_CUT).then(|| format!("{head}{cut}\n"))
}

/// `s` cut to at most `max` bytes on a character boundary.
fn clip(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// ssh's argv up to the remote command: batch mode, an 8 s connect timeout, `host`.
pub fn ssh(host: &str) -> Result<Vec<String>, String> {
    if host.is_empty() || host.starts_with('-') || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("{host:?}: not an ssh host"));
    }
    Ok(["/usr/bin/ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", host].map(String::from).to_vec())
}

/// The argv of ask `id` in thread `thread`: kota-ask at `kota_ask` (relative to the remote
/// home, or `~/…`) on `host`. The text goes on stdin (`compose`).
pub fn argv(host: &str, kota_ask: &str, id: &str, thread: &str) -> Result<Vec<String>, String> {
    if !attach::remote_word(kota_ask) {
        return Err(format!("{kota_ask:?}: kota-ask must be a path of A-Z a-z 0-9 . _ / ~ -"));
    }
    for (what, v) in [("request", id), ("thread", thread)] {
        if !valid_id(v) {
            return Err(format!("{v:?}: a {what} id is 1-64 of A-Z a-z 0-9 . _ -"));
        }
    }
    let mut argv = ssh(host)?;
    argv.extend([kota_ask, "--id", id, "--thread", thread].map(String::from));
    Ok(argv)
}

#[cfg(test)]
mod tests;
