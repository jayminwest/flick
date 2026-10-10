//! Phone app opens (flick-b418): flick-ios sends one event per app open over the network
//! (`flick activity phone add ...`, docs/remote.md "Phone events"), and reports list them
//! next to this Mac's spans. A remote write path, so every field is checked strictly
//! (`parse`) before anything is stored, and the phone's event id makes a retry store
//! nothing twice (`PhoneOpens::phone_add`). Adding needs no grant: it reads nothing back.
//! Reading opens (`today`, `week`, `spans`) needs the same grant as reading spans.

use std::collections::BTreeMap;
use std::fmt::Write;

use serde::Serialize;

use crate::core::Cx;
use crate::core::track::Subject;

use super::Activity;
use super::report::SpanOut;
use super::rules::Config;
use super::store::PhoneOpens;

const USAGE: &str = "activity phone: usage: activity phone add --id <event id> --device <name> --app <app> --at <unix secs> [--reason <text>] [--minutes <n>]";
/// Longest event id and device name, in characters.
const MAX_ID: usize = 64;
/// Longest app (bundle id or name), in characters.
const MAX_APP: usize = 128;
/// Longest reason, in characters.
const MAX_REASON: usize = 280;
/// Most minutes the phone may grant (a day).
const MAX_MINUTES: i64 = 1440;
/// Oldest open accepted: a phone may hold its queue this long (30 days).
const MAX_AGE: i64 = 30 * 86_400;
/// How far ahead of this Mac's clock an open may be (clock skew).
const MAX_AHEAD: i64 = 300;

/// One app open on a phone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneOpen {
    /// The phone's name, its Tailscale machine name by convention.
    pub device: String,
    /// The phone's id for this event, unique per device.
    pub id: String,
    /// Bundle id or app name, as the phone knows it.
    pub app: String,
    /// When the app opened, unix seconds.
    pub at: i64,
    /// Why, from the phone's intention prompt.
    pub reason: Option<String>,
    /// Minutes the prompt granted.
    pub minutes: Option<i64>,
}

fn bad(flag: &str, why: &str) -> String {
    format!("activity phone: bad {flag}: {why}")
}

/// `--id` and `--device`: 1 to 64 of `A-Z a-z 0-9 . _ -` (and `:` in ids).
fn token(flag: &str, v: &str, extra: &[char]) -> Result<String, String> {
    let ok = |c: char| c.is_ascii_alphanumeric() || ['.', '_', '-'].contains(&c) || extra.contains(&c);
    if v.is_empty() || v.chars().count() > MAX_ID {
        return Err(bad(flag, &format!("needs 1 to {MAX_ID} characters")));
    }
    if !v.chars().all(ok) {
        let extra: String = extra.iter().flat_map(|&c| [' ', c]).collect();
        return Err(bad(flag, &format!("only letters, digits and . _ -{extra} are allowed")));
    }
    Ok(v.to_owned())
}

/// `--app` and `--reason`: 1 to `max` characters, no control characters, no space at either end.
fn text(flag: &str, v: &str, max: usize) -> Result<String, String> {
    if v.is_empty() || v.chars().count() > max {
        return Err(bad(flag, &format!("needs 1 to {max} characters")));
    }
    if v.chars().any(char::is_control) || v.trim() != v {
        return Err(bad(flag, "no control characters or space at either end"));
    }
    Ok(v.to_owned())
}

/// A whole number of decimal digits only (no sign, no fraction) in `range`.
fn number(flag: &str, v: &str, range: std::ops::RangeInclusive<i64>) -> Result<i64, String> {
    let n = (!v.is_empty() && v.len() <= 12 && v.bytes().all(|b| b.is_ascii_digit()))
        .then(|| v.parse::<i64>().ok())
        .flatten();
    match n {
        Some(n) if range.contains(&n) => Ok(n),
        Some(_) if flag == "--at" => Err(bad(flag, "more than 30 days ago or ahead of this Mac's clock")),
        Some(_) => Err(bad(flag, &format!("must be {} to {}", range.start(), range.end()))),
        None => Err(bad(flag, "digits only (unix seconds or minutes)")),
    }
}

/// The open in `add`'s flags, checked against this Mac's clock `now`. Each flag once,
/// in any order; `--id`, `--device`, `--app` and `--at` are required.
pub fn parse(args: &[String], now: i64) -> Result<PhoneOpen, String> {
    let mut flags: BTreeMap<&str, &str> = BTreeMap::new();
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let known = ["--id", "--device", "--app", "--at", "--reason", "--minutes"];
        let Some(&flag) = known.iter().find(|k| **k == flag.as_str()) else {
            return Err(format!("activity phone: unknown argument \"{flag}\"; {USAGE}"));
        };
        let Some(value) = rest.next() else { return Err(bad(flag, "needs a value")) };
        if flags.insert(flag, value).is_some() {
            return Err(bad(flag, "given twice"));
        }
    }
    let need = |flag: &str| flags.get(flag).copied().ok_or_else(|| bad(flag, "missing"));
    Ok(PhoneOpen {
        id: token("--id", need("--id")?, &[':'])?,
        device: token("--device", need("--device")?, &[])?,
        app: text("--app", need("--app")?, MAX_APP)?,
        at: number("--at", need("--at")?, now.saturating_sub(MAX_AGE)..=now.saturating_add(MAX_AHEAD))?,
        reason: flags.get("--reason").map(|v| text("--reason", v, MAX_REASON)).transpose()?,
        minutes: flags.get("--minutes").map(|v| number("--minutes", v, 1..=MAX_MINUTES)).transpose()?,
    })
}

/// `phone add --json`.
#[derive(Serialize)]
struct Added<'a> {
    id: &'a str,
    device: &'a str,
    stored: bool,
}

impl Activity {
    /// `phone add ...`: store one open. Allowed for remote callers without the grant.
    pub(super) fn phone(&self, args: &[String], cx: &Cx) -> Result<String, String> {
        let [verb, flags @ ..] = args else { return Err(USAGE.into()) };
        if verb != "add" {
            return Err(USAGE.into());
        }
        let now = (self.env.now)();
        let open = parse(flags, now)?;
        let stored = cx.store.phone_add(&open, now)?;
        if cx.json {
            let added = Added { id: &open.id, device: &open.device, stored };
            return serde_json::to_string(&added).map_err(|e| format!("activity: {e}"));
        }
        let what = if stored { "Stored" } else { "Already stored" };
        Ok(format!("{what} phone open {} from {}", open.id, open.device))
    }
}

/// Opens of one app.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct AppOpens {
    pub name: String,
    pub opens: usize,
}

/// The phone part of a `today`/`week` report.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct PhoneSummary {
    /// Opens in the range, all devices.
    pub opens: usize,
    /// Opens per app, most first.
    pub by_app: Vec<AppOpens>,
}

impl PhoneSummary {
    pub fn new(opens: &[PhoneOpen]) -> PhoneSummary {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for o in opens {
            *counts.entry(&o.app).or_default() += 1;
        }
        let mut by_app: Vec<AppOpens> =
            counts.into_iter().map(|(name, opens)| AppOpens { name: name.to_owned(), opens }).collect();
        by_app.sort_by_key(|a| std::cmp::Reverse(a.opens));
        PhoneSummary { opens: opens.len(), by_app }
    }

    /// The report's `Phone opens` section; empty without opens.
    pub fn text(&self) -> String {
        let mut out = String::new();
        if self.opens > 0 {
            let _ = write!(out, "\nPhone opens ({})", self.opens);
        }
        for a in &self.by_app {
            let _ = write!(out, "\n  {:>8}  {}", a.opens, a.name);
        }
        out
    }
}

/// `forget today|all|app <app> --yes` for phone opens (`today`: the local day's start); how
/// many. Other requests delete nothing.
pub fn forget(what: &[String], store: &crate::core::store::Store, today: i64) -> usize {
    match what {
        [w] if w == "today" => store.phone_forget_since(today),
        [w] if w == "all" => store.phone_forget_since(i64::MIN),
        [w, app] if w == "app" => store.phone_forget_app(app),
        _ => 0,
    }
}

/// The phone part of `forget`'s reply: empty when no open was deleted.
pub fn forgotten(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => " and 1 phone open".into(),
        n => format!(" and {n} phone opens"),
    }
}

/// `opens` as span list entries: no duration, `source` "phone", rules applied to the app.
pub fn span_outs<'a>(opens: &'a [PhoneOpen], rules: &'a Config) -> Vec<SpanOut<'a>> {
    opens
        .iter()
        .map(|o| {
            let (category, project) = rules.classify(&Subject::new(&o.app, &o.app, None, None));
            SpanOut {
                source: "phone",
                device: Some(&o.device),
                reason: o.reason.as_deref(),
                minutes: o.minutes,
                ..SpanOut::new(o.at, o.at, &o.app, &o.app, (category, project))
            }
        })
        .collect()
}
