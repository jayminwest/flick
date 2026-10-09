//! `flick herdr ls|jump|status`: text and `--json` answers from a fleet snapshot. Pure.

use super::io::Info;
use super::model::{Agent, Fleet};
use super::views::{agent_subtitle, ago, machine_note};
use serde_json::{Value, json};
use std::path::Path;

/// Where the module talks to herdr, for `status`.
pub struct Paths<'a> {
    pub socket: &'a Path,
    pub herdr: &'a Path,
}

/// The agent `spec` names: `<machine>/<pane id or name>`, or a pane id or name alone when
/// exactly one machine has it.
pub fn resolve<'a>(fleet: &'a Fleet, spec: &str) -> Result<&'a Agent, String> {
    let (machine, target) = match spec.split_once('/') {
        Some((m, t)) => (Some(m), t),
        None => (None, spec),
    };
    if let Some(m) = machine
        && fleet.machine(m).is_none()
    {
        return Err(format!("herdr: no machine \"{m}\""));
    }
    let agents = fleet.ordered();
    let on = |a: &&Agent| machine.is_none_or(|m| a.machine == m);
    if let Some(a) = agents.iter().copied().filter(on).find(|a| a.pane_id == target) {
        return Ok(a);
    }
    let named: Vec<&Agent> =
        agents.into_iter().filter(on).filter(|a| a.name.as_deref() == Some(target)).collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("herdr: no agent \"{spec}\"")),
        _ => Err(format!("herdr: \"{spec}\" names {} agents; use <machine>/<pane id>", named.len())),
    }
}

fn agent_json(a: &Agent) -> Value {
    json!({
        "machine": a.machine,
        "pane_id": a.pane_id,
        "workspace_id": a.workspace_id,
        "kind": a.kind,
        "name": a.name,
        "status": a.status.as_str(),
        "cwd": a.cwd,
        "title": a.title,
        "focused": a.focused,
        "seq": a.seq,
    })
}

/// `{"machines": {<name>: {live, updated_at, checked_at, error, agents: [...]}}, "agents":
/// [...]}`; `agents` is in display order.
pub fn ls_json(fleet: &Fleet) -> Value {
    let ordered = fleet.ordered();
    let mut machines = serde_json::Map::new();
    for m in fleet.machine_names() {
        let Some(state) = fleet.machine(m) else { continue };
        let agents: Vec<Value> =
            ordered.iter().filter(|a| a.machine == m).map(|a| agent_json(a)).collect();
        machines.insert(
            m.to_string(),
            json!({
                "live": state.live,
                "updated_at": state.updated_at,
                "checked_at": state.checked_at,
                "error": state.error,
                "agents": agents,
            }),
        );
    }
    let agents: Vec<Value> = ordered.into_iter().map(agent_json).collect();
    json!({ "machines": machines, "agents": agents })
}

/// One line per agent, `<machine>/<pane id>  <status>  <label>  <subtitle>`, waiting
/// first, then a line per machine that has an error or no list yet.
pub fn ls_text(fleet: &Fleet, now: u64) -> String {
    let mut lines: Vec<String> = fleet
        .ordered()
        .into_iter()
        .map(|a| {
            let id = format!("{}/{}", a.machine, a.pane_id);
            format!("{id:<24} {:<8} {}  {}", a.status.as_str(), a.label(), agent_subtitle(a))
        })
        .collect();
    for m in fleet.machine_names() {
        if let Some(note) = fleet.machine(m).and_then(|s| machine_note(s, now)) {
            lines.push(format!("{m}: {note}"));
        }
    }
    if lines.is_empty() {
        return "no agents".into();
    }
    lines.join("\n")
}

/// Per machine: how it is kept current, when it was last read, and its error; then the
/// local server's version and the last jump error.
pub fn status_text(fleet: &Fleet, info: &Info, paths: &Paths, now: u64) -> String {
    let mut lines = vec![];
    for m in fleet.machine_names() {
        let Some(s) = fleet.machine(m) else { continue };
        let how = if s.live { "live" } else if m == super::io::LOCAL { "not connected" } else { "polled" };
        let read = s.updated_at.map_or("never read".into(), |t| format!("read {}", ago(now.saturating_sub(t))));
        let n = s.agents.len();
        let agents = if n == 1 { "agent" } else { "agents" };
        let error = s.error.as_ref().map_or(String::new(), |e| format!(" · error: {e}"));
        lines.push(format!("{m:<16} {how} · {n} {agents} · {read}{error}"));
    }
    if let Some(p) = &info.pong {
        lines.push(format!("herdr {} (protocol {})", p.version, p.protocol));
    }
    lines.push(format!("socket {}", paths.socket.display()));
    lines.push(format!("cli    {}", paths.herdr.display()));
    if let Some(e) = &info.discovery {
        lines.push(format!("machine list: {e}"));
    }
    if let Some(e) = &info.jump {
        lines.push(format!("last jump: {e}"));
    }
    lines.join("\n")
}

/// `status --json`.
pub fn status_json(fleet: &Fleet, info: &Info, paths: &Paths) -> Value {
    let mut machines = serde_json::Map::new();
    for m in fleet.machine_names() {
        let Some(s) = fleet.machine(m) else { continue };
        machines.insert(
            m.to_string(),
            json!({
                "live": s.live,
                "agents": s.agents.len(),
                "updated_at": s.updated_at,
                "checked_at": s.checked_at,
                "error": s.error,
            }),
        );
    }
    json!({
        "machines": machines,
        "herdr": info.pong.as_ref().map(|p| json!({ "version": p.version, "protocol": p.protocol })),
        "socket": paths.socket.display().to_string(),
        "cli": paths.herdr.display().to_string(),
        "discovery_error": info.discovery,
        "jump_error": info.jump,
    })
}

#[cfg(test)]
mod tests;
