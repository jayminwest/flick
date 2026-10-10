//! The remote-use grant: may a caller that can send its output to a remote model
//! (`Cx::remote`, an agent session) read activity? Off until the user allows it with
//! `flick activity remote allow [<minutes>|always]` or the root item `activity:remote`. The
//! grant ends at `remote_until` in `activity_state`, checked on each call (no timer). Remote
//! callers never grant, deny, record or forget, and see window titles only with
//! `[activity] remote_titles = true`, URLs and domains only with `remote_urls = true`. A guard against accidents, not against a hostile agent:
//! the gate is a request word.

use serde::Serialize;

use crate::core::store::Store;
use crate::core::{Cx, Icon, Item, ItemId};

use super::Activity;
use super::report::{Report, SpanOut};
use crate::core::track::local_time;
use super::store::Spans;

pub const REFUSED: &str = "activity: remote use not permitted; the user can run `flick activity remote allow` or choose 'Allow Agents to Read Activity' in Flick";
const AGENT: &str = "activity: not allowed from an agent session (--remote)";
const USAGE: &str = "activity: usage: activity remote allow [<minutes>|always] | deny | status";
/// Length of a grant without minutes, and of the root item's.
const MINUTES: i64 = 60;
/// `remote_until` of a grant without an end.
const ALWAYS: i64 = i64::MAX;

/// What a remote caller may do with `args`: Err refuses it; `Ok(true)` is a read of activity,
/// which a live grant allows.
pub fn gate(args: &[String], store: &Store, now: i64) -> Result<bool, String> {
    match args.first().map(String::as_str) {
        Some("today" | "week" | "spans") if live(store, now) => Ok(true),
        Some("today" | "week" | "spans") => Err(REFUSED.into()),
        Some("on" | "off" | "forget") => Err(AGENT.into()),
        Some("remote") if args.get(1).is_some_and(|v| v != "status") => Err(AGENT.into()),
        _ => Ok(false),
    }
}

fn live(store: &Store, now: i64) -> bool {
    store.remote_until() > now
}

/// Time left on a live grant ending at `until`.
fn left(until: i64, now: i64) -> String {
    match (until - now) / 60 {
        _ if until == ALWAYS => "until revoked".into(),
        0 => "<1 min left".into(),
        m => format!("{m} min left"),
    }
}

/// `remote status --json`. `until` is null without a grant and for one without an end.
#[derive(Debug, Serialize)]
struct Status {
    granted: bool,
    until: Option<i64>,
    last_read: Option<i64>,
}

impl Activity {
    /// `remote allow [<minutes>|always] | deny | status`. Remote callers reach only `status`
    /// (`gate`).
    pub(super) fn remote(&self, args: &[String], cx: &Cx) -> Result<String, String> {
        let now = (self.env.now)();
        let until = match args {
            [v] if v == "allow" => now + MINUTES * 60,
            [v, a] if v == "allow" && a == "always" => ALWAYS,
            [v, m] if v == "allow" => match m.parse::<i64>() {
                Ok(m) if m > 0 => now.saturating_add(m.saturating_mul(60)),
                _ => return Err(USAGE.into()),
            },
            [v] if v == "deny" => 0,
            [v] if v == "status" => return self.remote_status(cx, now),
            _ => return Err(USAGE.into()),
        };
        cx.store.set_remote_until(until);
        Ok(grant_text(until, now))
    }

    fn remote_status(&self, cx: &Cx, now: i64) -> Result<String, String> {
        let until = cx.store.remote_until();
        let granted = until > now;
        let last = cx.store.remote_last();
        if cx.json {
            let until = Some(until).filter(|&u| granted && u != ALWAYS);
            let status = Status { granted, until, last_read: last };
            return serde_json::to_string(&status).map_err(|e| format!("activity: {e}"));
        }
        let offset = (self.env.utc_offset)(now);
        let last = last.map_or_else(|| "never".into(), |t| local_time(t, offset));
        Ok(format!("{}\nlast read: {last}", grant_text(if granted { until } else { 0 }, now)))
    }

    /// A remote reply leaves out window titles without `remote_titles`, and domains without
    /// `remote_urls`.
    pub(super) fn redact_report(&self, cx: &Cx, r: &mut Report) {
        if cx.remote && !self.config.remote_titles {
            r.top_titles.clear();
        }
        if cx.remote && !self.config.remote_urls {
            r.top_domains.clear();
        }
    }

    /// A remote reply leaves out window titles without `remote_titles`, and URLs without
    /// `remote_urls`.
    pub(super) fn redact_spans(&self, cx: &Cx, list: &mut [SpanOut]) {
        for s in list {
            if cx.remote && !self.config.remote_titles {
                s.title = None;
            }
            if cx.remote && !self.config.remote_urls {
                s.url = None;
            }
        }
    }

    /// The root item `activity:remote`: allow for an hour, or revoke a live grant.
    pub(super) fn remote_item(&self, store: &Store) -> Item {
        let now = (self.env.now)();
        let until = store.remote_until();
        let (title, symbol) = if until > now {
            (format!("Revoke Agent Activity Access · {}", left(until, now)), "lock")
        } else {
            ("Allow Agents to Read Activity (1 h)".into(), "lock.open")
        };
        Item {
            subtitle: "Activity".into(),
            accessory: "Command".into(),
            keywords: vec!["agent remote permission privacy".into()],
            ..Item::new(ItemId::new("activity", "remote"), title, "Run Command", Icon::Symbol(symbol))
        }
    }

    /// The root item's action; the status line.
    pub(super) fn toggle_remote(&self, store: &Store) -> String {
        let now = (self.env.now)();
        let until = if live(store, now) { 0 } else { now + MINUTES * 60 };
        store.set_remote_until(until);
        grant_text(until, now)
    }
}

fn grant_text(until: i64, now: i64) -> String {
    if until > now {
        format!("Agents may read activity · {}", left(until, now))
    } else {
        "Agents may not read activity".into()
    }
}
