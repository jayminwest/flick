//! The herdr fleet: every agent on every machine, as the last `agent.list` reply plus the
//! subscription events since. Pure Rust; the transport threads feed it, the views read it.
//!
//! Wire shapes follow `herdr api schema --json` (protocol 22, herdr 0.9.1). Unknown fields
//! are ignored and unknown statuses read as `Unknown`, so a newer herdr still parses.
//!
//! Freshness is per machine: `updated_at` is the last good list, `checked_at` the last
//! attempt, `live` marks a machine kept current by an event stream (the local one). A
//! polled machine asks for a refresh through `needs_refresh` (remote machines refresh
//! while the launcher or the agents view is open, or on an opt-in background interval).

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Longest preview line, in chars, before it is cut with an ellipsis.
pub const PREVIEW_WIDTH: usize = 200;

/// An agent's state as herdr reports it. Declaration order is display order: the ones
/// waiting on me first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// An approval or question UI waits for input.
    Blocked,
    /// Finished and not yet seen.
    Done,
    Idle,
    Working,
    #[default]
    #[serde(other)]
    Unknown,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Blocked => "blocked",
            Status::Done => "done",
            Status::Idle => "idle",
            Status::Working => "working",
            Status::Unknown => "unknown",
        }
    }

    /// Blocked or done: worth a notification when an agent enters it.
    pub fn waits_on_me(self) -> bool {
        matches!(self, Status::Blocked | Status::Done)
    }
}

/// One agent pane on one machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub machine: String,
    pub pane_id: String,
    pub workspace_id: String,
    /// The agent program (`claude`, `pi`, `codex`), when herdr detected one.
    pub kind: Option<String>,
    pub name: Option<String>,
    pub status: Status,
    pub cwd: Option<String>,
    /// Terminal title with escapes stripped.
    pub title: Option<String>,
    pub focused: bool,
    /// herdr's `state_change_seq`.
    pub seq: u64,
    /// When Flick saw the status change (the caller's clock); orders a status group.
    pub changed_at: u64,
}

impl Agent {
    /// The row title: the agent's name, else its program, else its pane id.
    pub fn label(&self) -> &str {
        self.name.as_deref().or(self.kind.as_deref()).unwrap_or(&self.pane_id)
    }
}

/// `AgentInfo` and the agent fields of `PaneInfo` (pane events carry no `state_change_seq`).
#[derive(Deserialize)]
struct AgentInfo {
    pane_id: String,
    workspace_id: String,
    agent: Option<String>,
    display_agent: Option<String>,
    name: Option<String>,
    label: Option<String>,
    #[serde(default)]
    agent_status: Status,
    cwd: Option<String>,
    terminal_title_stripped: Option<String>,
    title: Option<String>,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    state_change_seq: u64,
}

impl AgentInfo {
    fn into_agent(self, machine: &str) -> Agent {
        Agent {
            machine: machine.to_string(),
            pane_id: self.pane_id,
            workspace_id: self.workspace_id,
            kind: self.display_agent.or(self.agent),
            name: self.name.or(self.label),
            status: self.agent_status,
            cwd: self.cwd,
            title: self.terminal_title_stripped.or(self.title),
            focused: self.focused,
            seq: self.state_change_seq,
            changed_at: 0,
        }
    }
}

/// The `result` of a reply `{"id","result"}` (or a bare result), else the reply's error.
pub fn reply_result(reply: &Value) -> Result<&Value, String> {
    if let Some(e) = reply.get("error") {
        let msg = e.get("message").and_then(Value::as_str);
        return Err(msg.map_or_else(|| e.to_string(), str::to_string));
    }
    Ok(reply.get("result").unwrap_or(reply))
}

/// The agents in an `agent.list` reply, tagged with `machine`.
pub fn parse_agents(machine: &str, reply: &Value) -> Result<Vec<Agent>, String> {
    #[derive(Deserialize)]
    struct List {
        agents: Vec<AgentInfo>,
    }
    let list = List::deserialize(reply_result(reply)?)
        .map_err(|e| format!("{machine}: bad agent.list reply: {e}"))?;
    Ok(list.agents.into_iter().map(|a| a.into_agent(machine)).collect())
}

/// The output text in an `agent.read` reply (`{"type":"pane_read","read":{"text"}}`).
pub fn parse_read(reply: &Value) -> Result<String, String> {
    let text = reply_result(reply)?.get("read").and_then(|r| r.get("text"));
    text.and_then(Value::as_str).map(str::to_string).ok_or_else(|| "bad agent.read reply".into())
}

/// What a pushed event means for the fleet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// `pane_agent_status_changed` (global) or `pane.agent_status_changed` (per pane).
    Status { pane_id: String, workspace_id: String, status: Status, kind: Option<String> },
    /// `pane_created` / `pane_updated`: the pane's current agent fields.
    Pane(Agent),
    /// `pane_agent_detected` with an agent.
    Detected { pane_id: String, workspace_id: String, kind: String },
    /// `pane_closed`, `pane_exited`, or `pane_agent_detected` that released the agent.
    Gone { pane_id: String },
}

/// A pushed `{"event","data"}` envelope as a `Change`; `None` for events the fleet ignores.
pub fn parse_event(machine: &str, event: &Value) -> Option<Change> {
    let kind = event.get("event")?.as_str()?;
    let data = event.get("data")?;
    let field = |k: &str| data.get(k).and_then(Value::as_str).map(str::to_string);
    let pane_id = field("pane_id");
    match kind.replace('.', "_").as_str() {
        "pane_agent_status_changed" => Some(Change::Status {
            pane_id: pane_id?,
            workspace_id: field("workspace_id").unwrap_or_default(),
            status: Status::deserialize(data.get("agent_status")?).ok()?,
            kind: field("display_agent").or_else(|| field("agent")),
        }),
        "pane_created" | "pane_updated" => {
            let info = AgentInfo::deserialize(data.get("pane")?).ok()?;
            Some(Change::Pane(info.into_agent(machine)))
        }
        "pane_agent_detected" => {
            let released = data.get("released").and_then(Value::as_bool).unwrap_or(false);
            match field("agent") {
                Some(kind) if !released => Some(Change::Detected {
                    pane_id: pane_id?,
                    workspace_id: field("workspace_id").unwrap_or_default(),
                    kind,
                }),
                _ => Some(Change::Gone { pane_id: pane_id? }),
            }
        }
        "pane_closed" | "pane_exited" => Some(Change::Gone { pane_id: pane_id? }),
        _ => None,
    }
}

/// What applying a change did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The visible fleet changed: refresh the open view.
    pub changed: bool,
    /// A pane the fleet does not know got an agent: list again (and subscribe to it).
    pub relist: bool,
}

/// An agent that entered a status that waits on me.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub from: Status,
    pub agent: Agent,
}

/// One machine's agents and how fresh they are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MachineState {
    /// Keyed by pane id.
    pub agents: BTreeMap<String, Agent>,
    /// Last good `agent.list`; `None` before the first.
    pub updated_at: Option<u64>,
    /// Last attempt, good or not.
    pub checked_at: Option<u64>,
    /// The last attempt's error; cleared by a good list. The agents stay as cached rows.
    pub error: Option<String>,
    /// An event stream keeps this machine current; it never needs polling.
    pub live: bool,
}

/// Counts for the root item's subtitle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub blocked: usize,
    pub done: usize,
    pub agents: usize,
    pub machines: usize,
}

/// Every machine's agents. Machines keep the configured order; others follow by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fleet {
    order: Vec<String>,
    machines: BTreeMap<String, MachineState>,
    pending: Vec<Transition>,
}

impl Fleet {
    #[cfg(test)]
    pub fn new<S: Into<String>>(machines: impl IntoIterator<Item = S>) -> Fleet {
        let mut fleet = Fleet::default();
        fleet.set_machines(machines);
        fleet
    }

    /// Keep only `machines`, in this order; new ones start empty (config reload).
    pub fn set_machines<S: Into<String>>(&mut self, machines: impl IntoIterator<Item = S>) {
        self.order = machines.into_iter().map(Into::into).collect();
        self.machines.retain(|m, _| self.order.contains(m));
        for m in &self.order {
            self.machines.entry(m.clone()).or_default();
        }
    }

    /// Machine names in display order.
    pub fn machine_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.order.iter().map(String::as_str).collect();
        let rest = self.machines.keys().filter(|m| !self.order.contains(m));
        names.extend(rest.map(String::as_str));
        names
    }

    pub fn machine(&self, machine: &str) -> Option<&MachineState> {
        self.machines.get(machine)
    }

    pub fn agent(&self, machine: &str, pane_id: &str) -> Option<&Agent> {
        self.machines.get(machine)?.agents.get(pane_id)
    }

    /// Mark `machine` as kept current by an event stream (or not, when the stream drops).
    pub fn set_live(&mut self, machine: &str, live: bool) {
        self.machines.entry(machine.to_string()).or_default().live = live;
    }

    /// Whether a polled `machine` is due: never checked, or last checked `min_age` seconds
    /// ago or more. A live machine never is.
    pub fn needs_refresh(&self, machine: &str, now: u64, min_age: u64) -> bool {
        self.machines.get(machine).is_none_or(|m| {
            !m.live && m.checked_at.is_none_or(|t| now.saturating_sub(t) >= min_age)
        })
    }

    /// Replace `machine`'s agents with a fresh `agent.list`, read at `now`. Status changes
    /// against the previous list become transitions; the first list for a machine and
    /// agents seen for the first time make none.
    pub fn apply_list(&mut self, machine: &str, agents: Vec<Agent>, now: u64) -> Applied {
        let state = self.machines.entry(machine.to_string()).or_default();
        let first = state.updated_at.is_none();
        let mut old = std::mem::take(&mut state.agents);
        let mut changed = first || state.error.is_some();
        for mut agent in agents {
            agent.machine = machine.to_string();
            let prev = old.remove(&agent.pane_id);
            agent.changed_at = match &prev {
                Some(p) if p.status == agent.status => p.changed_at,
                _ => now,
            };
            if let Some(p) = &prev
                && p.status != agent.status
                && agent.status.waits_on_me()
            {
                self.pending.push(Transition { from: p.status, agent: agent.clone() });
            }
            changed |= prev.as_ref() != Some(&agent);
            state.agents.insert(agent.pane_id.clone(), agent);
        }
        changed |= !old.is_empty();
        state.updated_at = Some(now);
        state.checked_at = Some(now);
        state.error = None;
        Applied { changed, relist: false }
    }

    /// A failed read of `machine` at `now`. Its agents stay as cached rows.
    pub fn apply_error(&mut self, machine: &str, error: impl Into<String>, now: u64) -> Applied {
        let state = self.machines.entry(machine.to_string()).or_default();
        let error = Some(error.into());
        let changed = state.error != error;
        state.error = error;
        state.checked_at = Some(now);
        Applied { changed, relist: false }
    }

    /// Apply one pushed change for `machine` at `now`.
    pub fn apply_event(&mut self, machine: &str, change: Change, now: u64) -> Applied {
        let state = self.machines.entry(machine.to_string()).or_default();
        let (pane_id, update) = match change {
            Change::Gone { pane_id } => {
                let changed = state.agents.remove(&pane_id).is_some();
                return Applied { changed, relist: false };
            }
            Change::Detected { pane_id, workspace_id, kind } => {
                (pane_id.clone(), Update::Detected { pane_id, workspace_id, kind })
            }
            Change::Status { pane_id, workspace_id, status, kind } => {
                (pane_id.clone(), Update::Status { pane_id, workspace_id, status, kind })
            }
            Change::Pane(agent) => (agent.pane_id.clone(), Update::Pane(agent)),
        };
        let Some(prev) = state.agents.get_mut(&pane_id) else {
            // Unknown pane: show what the event says until the next list fills it in.
            let Some(mut agent) = update.into_new(machine) else {
                return Applied::default();
            };
            agent.changed_at = now;
            state.agents.insert(pane_id, agent);
            return Applied { changed: true, relist: true };
        };
        let before = prev.clone();
        update.merge_into(prev);
        if prev.status != before.status {
            prev.changed_at = now;
            if prev.status.waits_on_me() {
                self.pending.push(Transition { from: before.status, agent: prev.clone() });
            }
        }
        Applied { changed: *prev != before, relist: false }
    }

    /// Transitions since the last call, oldest first.
    #[cfg_attr(not(test), expect(dead_code, reason = "notifications call it (flick-adda)"))]
    pub fn take_transitions(&mut self) -> Vec<Transition> {
        std::mem::take(&mut self.pending)
    }

    /// Every agent in display order: blocked, done, idle, working, unknown; within a
    /// status the most recent change first, then the higher herdr seq, then machine order.
    pub fn ordered(&self) -> Vec<&Agent> {
        let names = self.machine_names();
        let rank = |m: &str| names.iter().position(|n| *n == m);
        let mut all: Vec<&Agent> = self.machines.values().flat_map(|m| m.agents.values()).collect();
        all.sort_by(|a, b| {
            (a.status, std::cmp::Reverse(a.changed_at), std::cmp::Reverse(a.seq))
                .cmp(&(b.status, std::cmp::Reverse(b.changed_at), std::cmp::Reverse(b.seq)))
                .then_with(|| rank(&a.machine).cmp(&rank(&b.machine)))
                .then_with(|| a.pane_id.cmp(&b.pane_id))
        });
        all
    }

    pub fn summary(&self) -> Summary {
        let agents = || self.machines.values().flat_map(|m| m.agents.values());
        Summary {
            blocked: agents().filter(|a| a.status == Status::Blocked).count(),
            done: agents().filter(|a| a.status == Status::Done).count(),
            agents: agents().count(),
            machines: self.machines.len(),
        }
    }
}

/// An event's update to one pane.
enum Update {
    Status { pane_id: String, workspace_id: String, status: Status, kind: Option<String> },
    Detected { pane_id: String, workspace_id: String, kind: String },
    Pane(Agent),
}

impl Update {
    /// A row for a pane the fleet does not know; `None` for a pane with no agent.
    fn into_new(self, machine: &str) -> Option<Agent> {
        let blank = |pane_id, workspace_id, status, kind| Agent {
            machine: machine.to_string(),
            pane_id,
            workspace_id,
            kind,
            name: None,
            status,
            cwd: None,
            title: None,
            focused: false,
            seq: 0,
            changed_at: 0,
        };
        match self {
            Update::Status { pane_id, workspace_id, status, kind } => {
                Some(blank(pane_id, workspace_id, status, kind))
            }
            Update::Detected { pane_id, workspace_id, kind } => {
                Some(blank(pane_id, workspace_id, Status::Unknown, Some(kind)))
            }
            Update::Pane(agent) => agent.kind.is_some().then_some(agent),
        }
    }

    fn merge_into(self, agent: &mut Agent) {
        match self {
            Update::Status { status, kind, .. } => {
                agent.status = status;
                agent.kind = kind.or(agent.kind.take());
            }
            Update::Detected { kind, .. } => agent.kind = Some(kind),
            Update::Pane(new) => {
                agent.kind = new.kind.or(agent.kind.take());
                agent.name = new.name.or(agent.name.take());
                agent.status = new.status;
                agent.cwd = new.cwd.or(agent.cwd.take());
                agent.title = new.title.or(agent.title.take());
                agent.focused = new.focused;
            }
        }
    }
}

/// The last `lines` non-empty lines of `text` (output herdr already stripped of escapes),
/// trailing space and control characters removed, each cut to `PREVIEW_WIDTH` chars.
pub fn preview(text: &str, lines: usize) -> Vec<String> {
    let clean = |line: &str| -> String {
        let line: String = line.chars().map(|c| if c == '\t' { ' ' } else { c }).collect();
        let line: String = line.chars().filter(|c| !c.is_control()).collect();
        let line = line.trim_end();
        if line.chars().count() <= PREVIEW_WIDTH {
            return line.to_string();
        }
        let cut: String = line.chars().take(PREVIEW_WIDTH - 1).collect();
        format!("{}…", cut.trim_end())
    };
    let mut out: Vec<String> =
        text.lines().rev().map(clean).filter(|l| !l.is_empty()).take(lines).collect();
    out.reverse();
    out
}

#[cfg(test)]
mod tests;
