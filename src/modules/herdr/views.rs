//! Items for the root search and the module's two views, built from a fleet snapshot.
//! Pure: no locks, no clock (callers pass `now`), no I/O.
//!
//! Item keys: `agents` (the root item; permanent, it keys the usage table), and in views
//! (`record_use = false`, never stored) `agent/<machine>/<pane id>`, `machine/<machine>`,
//! `reply` (copies the agent's last reply) and `line/<n>` (why there is no reply yet).

use super::io::Preview;
use super::model::{Agent, Fleet, MachineState, Status};
use super::reply;
use crate::core::{Icon, Item, ItemId, Tab};

pub const ID: &str = "herdr";

/// What an item key names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    Agents,
    Agent { machine: &'a str, pane_id: &'a str },
    Machine(&'a str),
    Reply,
    Line,
}

impl Key<'_> {
    pub fn parse(key: &str) -> Option<Key<'_>> {
        if key == "agents" {
            return Some(Key::Agents);
        }
        if let Some(rest) = key.strip_prefix("agent/") {
            let (machine, pane_id) = rest.split_once('/')?;
            return Some(Key::Agent { machine, pane_id });
        }
        if let Some(machine) = key.strip_prefix("machine/") {
            return Some(Key::Machine(machine));
        }
        if key == "reply" {
            return Some(Key::Reply);
        }
        key.starts_with("line/").then_some(Key::Line)
    }
}

pub fn agent_key(machine: &str, pane_id: &str) -> String {
    format!("agent/{machine}/{pane_id}")
}

/// `secs` as a short age: `now`, `42 s`, `3 min`, `2 h`, `5 d`.
pub fn age(secs: u64) -> String {
    match secs {
        0..5 => "now".into(),
        5..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        3600..86_400 => format!("{} h", secs / 3600),
        _ => format!("{} d", secs / 86_400),
    }
}

/// `just now` or `<age> ago`.
pub fn ago(secs: u64) -> String {
    match age(secs).as_str() {
        "now" => "just now".into(),
        a => format!("{a} ago"),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `2 waiting · 7 agents · 4 machines`; waiting is blocked plus done.
pub fn summary_line(fleet: &Fleet) -> String {
    let s = fleet.summary();
    let mut parts = vec![];
    if s.blocked + s.done > 0 {
        parts.push(format!("{} waiting", s.blocked + s.done));
    }
    parts.push(plural(s.agents, "agent", "agents"));
    parts.push(plural(s.machines, "machine", "machines"));
    parts.join(" · ")
}

/// The root item, `Herdr Agents`.
pub fn root_item(fleet: &Fleet) -> Item {
    Item {
        subtitle: summary_line(fleet),
        accessory: "herdr".into(),
        keywords: vec!["herdr agents claude codex pi waiting blocked".into()],
        ..Item::new(ItemId::new(ID, "agents"), "Herdr Agents", "Show Agents", Icon::Symbol("terminal"))
    }
}

pub fn status_icon(status: Status) -> Icon {
    Icon::Symbol(match status {
        Status::Blocked => "exclamationmark.bubble",
        Status::Done => "checkmark.circle",
        Status::Idle => "pause.circle",
        Status::Working => "gearshape",
        Status::Unknown => "questionmark.circle",
    })
}

/// The last path component of `cwd`, `~` for the home directory.
fn base(cwd: &str) -> &str {
    let trimmed = cwd.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((_, last)) if !last.is_empty() => last,
        _ => trimmed,
    }
}

/// `<status> · <cwd basename> · <terminal title>`, skipping what is missing.
pub fn agent_subtitle(agent: &Agent) -> String {
    let mut parts = vec![agent.status.as_str().to_string()];
    parts.extend(agent.cwd.as_deref().map(base).filter(|b| !b.is_empty()).map(str::to_string));
    parts.extend(agent.title.clone().filter(|t| !t.is_empty()));
    parts.join(" · ")
}

/// One row: `<name or kind> · <machine>`. Enter jumps, Tab shows its output (action
/// `output`).
pub fn agent_item(agent: &Agent, now: u64) -> Item {
    let mut keywords = vec![agent.machine.clone(), agent.status.as_str().to_string()];
    keywords.extend(agent.kind.clone());
    keywords.extend(agent.cwd.clone());
    keywords.extend(agent.title.clone());
    Item {
        subtitle: agent_subtitle(agent),
        accessory: if agent.changed_at == 0 { String::new() } else { age(now.saturating_sub(agent.changed_at)) },
        keywords,
        tab: Tab::Act("output"),
        ..Item::new(
            ItemId::new(ID, agent_key(&agent.machine, &agent.pane_id)),
            format!("{} · {}", agent.label(), agent.machine),
            "Jump to Agent",
            status_icon(agent.status),
        )
    }
}

/// A machine's freshness: `herdr not running, last read 3 min ago`, `loading…`.
pub fn machine_note(state: &MachineState, now: u64) -> Option<String> {
    let last = state.updated_at.map(|t| format!("last read {}", ago(now.saturating_sub(t))));
    match (&state.error, state.updated_at, last) {
        (Some(e), _, Some(last)) => Some(format!("{e}, {last}")),
        (Some(e), _, None) => Some(e.clone()),
        (None, None, _) if !state.live => Some("loading…".into()),
        _ => None,
    }
}

/// A row per machine with an error or no list yet, after the agents.
pub fn machine_items(fleet: &Fleet, now: u64) -> Vec<Item> {
    fleet
        .machine_names()
        .into_iter()
        .filter_map(|m| {
            let state = fleet.machine(m)?;
            let note = machine_note(state, now)?;
            let icon = if state.error.is_some() { "exclamationmark.triangle" } else { "hourglass" };
            Some(Item {
                subtitle: note.clone(),
                keywords: vec![note],
                ..Item::new(ItemId::new(ID, format!("machine/{m}")), m, "Show Error", Icon::Symbol(icon))
            })
        })
        .collect()
}

/// View `agents`: every agent, waiting first, then the machines that need a note.
pub fn agents_items(fleet: &Fleet, now: u64) -> Vec<Item> {
    let mut items: Vec<Item> = fleet.ordered().into_iter().map(|a| agent_item(a, now)).collect();
    items.extend(machine_items(fleet, now));
    items
}

/// The reply `preview` holds for `agent`, if it has loaded.
pub fn loaded_reply<'a>(agent: &Agent, preview: Option<&'a Preview>) -> Option<&'a str> {
    let mine = preview.filter(|p| p.machine == agent.machine && p.pane_id == agent.pane_id)?;
    match &mine.reply {
        Some(Ok(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
}

/// View `agent`: `Jump` and `Copy Reply` (or why there is no reply), and the reply's
/// wrapped tail for `ListView::text`.
pub fn detail(agent: Option<&Agent>, preview: Option<&Preview>, now: u64) -> (Vec<Item>, String) {
    let Some(agent) = agent else {
        return (vec![], String::new());
    };
    let jump = Item {
        title: format!("Jump to {}", agent.label()),
        tab: Tab::None,
        ..agent_item(agent, now)
    };
    let line = |text: String, icon: &'static str| {
        Item::new(ItemId::new(ID, "line/0"), text, "Jump to Agent", Icon::Symbol(icon))
    };
    let mine = preview.filter(|p| p.machine == agent.machine && p.pane_id == agent.pane_id);
    let (second, text) = match (mine.and_then(|p| p.reply.as_ref()), loaded_reply(agent, preview)) {
        (_, Some(text)) => {
            let lines = text.lines().count();
            let copy = Item {
                subtitle: format!("{lines} line{}", if lines == 1 { "" } else { "s" }),
                ..Item::new(ItemId::new(ID, "reply"), "Copy Reply", "Copy Reply", Icon::Symbol("doc.on.doc"))
            };
            (copy, reply::tail(text, reply::WIDTH, reply::ROWS))
        }
        (None, _) => (line("Loading output…".into(), "hourglass"), String::new()),
        (Some(Err(e)), _) => (line(e.clone(), "exclamationmark.triangle"), String::new()),
        (Some(Ok(_)), None) => (line("No reply".into(), "text.alignleft"), String::new()),
    };
    (vec![jump, second], text)
}

#[cfg(test)]
mod tests;
