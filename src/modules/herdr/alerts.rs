//! Notifications for agents that start to wait on me. Pure: the module drains the fleet's
//! transitions on the main thread, turns them into `Note`s here and posts them through
//! `Hooks::notify` (`platform::notify`).
//!
//! A note's id is the agent's item id (`herdr:agent/<machine>/<pane id>`): a second note for
//! the same agent replaces the first, and a click hands the id back for a jump. The text
//! holds the agent's name, machine, cwd and terminal title, never its output.

use super::model::{Fleet, Status, Transition};
use super::views::{Key, agent_key};
use crate::core::ItemId;

/// Statuses `notify` may name.
pub const KINDS: [Status; 2] = [Status::Blocked, Status::Done];

/// One notification to post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
}

/// `notify` names as statuses, or the first name that is not `blocked` or `done`.
pub fn kinds(names: &[String]) -> Result<Vec<Status>, String> {
    names
        .iter()
        .map(|n| {
            KINDS.into_iter().find(|k| k.as_str() == n).ok_or_else(|| {
                format!("[herdr]: notify takes \"blocked\" and \"done\", not \"{n}\"")
            })
        })
        .collect()
}

/// The notes for `transitions` (oldest first): one per agent, for its last transition, and
/// only when that status is in `on`, the agent still has it, and it is not the pane I look at
/// (herdr reports it focused while the terminal app is in front).
pub fn notes(transitions: Vec<Transition>, on: &[Status], fleet: &Fleet, terminal_front: bool) -> Vec<Note> {
    let mut picked: Vec<Transition> = vec![];
    for t in transitions {
        picked.retain(|p| p.agent.machine != t.agent.machine || p.agent.pane_id != t.agent.pane_id);
        picked.push(t);
    }
    picked
        .into_iter()
        .filter(|t| on.contains(&t.agent.status))
        .filter_map(|t| {
            let now = fleet.agent(&t.agent.machine, &t.agent.pane_id)?;
            (now.status == t.agent.status && !(now.focused && terminal_front)).then(|| note(t))
        })
        .collect()
}

fn note(t: Transition) -> Note {
    let a = t.agent;
    let verb = if a.status == Status::Done { "is done" } else { "is waiting" };
    let body: Vec<&str> =
        [Some(a.machine.as_str()), a.cwd.as_deref(), a.title.as_deref()].into_iter().flatten().collect();
    Note {
        id: ItemId::new(super::views::ID, agent_key(&a.machine, &a.pane_id)).to_string(),
        title: format!("{} {verb}", a.label()),
        body: body.join(" · "),
    }
}

/// The agent a clicked note's id names: (machine, pane id).
pub fn clicked(id: &str) -> Option<(&str, &str)> {
    let key = id.strip_prefix(super::views::ID)?.strip_prefix(':')?;
    match Key::parse(key)? {
        Key::Agent { machine, pane_id } => Some((machine, pane_id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::herdr::model::Agent;

    fn agent(pane: &str, status: Status, focused: bool) -> Agent {
        Agent {
            machine: "hub".into(),
            pane_id: pane.into(),
            workspace_id: "w1".into(),
            kind: Some("claude".into()),
            name: Some(format!("n-{pane}")),
            status,
            cwd: Some("/src/api".into()),
            title: None,
            focused,
            seq: 1,
            changed_at: 0,
        }
    }

    fn fleet(agents: Vec<Agent>) -> Fleet {
        let mut fleet = Fleet::new(["hub"]);
        fleet.apply_list("hub", agents, 1);
        fleet
    }

    fn moved(a: &Agent) -> Transition {
        Transition { from: Status::Working, agent: a.clone() }
    }

    #[test]
    fn kinds_take_blocked_and_done_only() {
        let names = |n: &[&str]| n.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        assert_eq!(kinds(&names(&["done", "blocked"])), Ok(vec![Status::Done, Status::Blocked]));
        assert_eq!(kinds(&[]), Ok(vec![]));
        assert_eq!(
            kinds(&names(&["blocked", "idle"])),
            Err("[herdr]: notify takes \"blocked\" and \"done\", not \"idle\"".into())
        );
    }

    #[test]
    fn a_waiting_agent_makes_one_note_with_its_item_id() {
        let blocked = agent("p1", Status::Blocked, false);
        let mut done = agent("p2", Status::Done, false);
        done.title = Some("tests pass".into());
        let f = fleet(vec![blocked.clone(), done.clone()]);
        let both = [Status::Blocked, Status::Done];
        let notes = notes(vec![moved(&blocked), moved(&done), moved(&blocked)], &both, &f, false);
        assert_eq!(
            notes,
            [
                Note {
                    id: "herdr:agent/hub/p2".into(),
                    title: "n-p2 is done".into(),
                    body: "hub · /src/api · tests pass".into()
                },
                Note { id: "herdr:agent/hub/p1".into(), title: "n-p1 is waiting".into(), body: "hub · /src/api".into() },
            ]
        );
        assert_eq!(clicked(&notes[0].id), Some(("hub", "p2")));
    }

    #[test]
    fn no_note_when_off_stale_gone_or_in_front_of_me() {
        let blocked = agent("p1", Status::Blocked, true);
        let f = fleet(vec![agent("p1", Status::Blocked, true), agent("p2", Status::Working, false)]);
        let on = [Status::Blocked];
        // Not asked for.
        assert!(notes(vec![moved(&agent("p1", Status::Done, false))], &on, &f, false).is_empty());
        // Already working again, or gone.
        assert!(notes(vec![moved(&agent("p2", Status::Blocked, false))], &on, &f, false).is_empty());
        assert!(notes(vec![moved(&agent("p9", Status::Blocked, false))], &on, &f, false).is_empty());
        // Focused in herdr: a note only while the terminal is not in front.
        assert!(notes(vec![moved(&blocked)], &on, &f, true).is_empty());
        assert_eq!(notes(vec![moved(&blocked)], &on, &f, false).len(), 1);
    }

    #[test]
    fn clicked_ids_name_an_agent_or_nothing() {
        assert_eq!(clicked("herdr:agent/local/w1:p1"), Some(("local", "w1:p1")));
        assert_eq!(clicked("herdr:agents"), None);
        assert_eq!(clicked("herdr:bogus"), None);
        assert_eq!(clicked("herdrx:agent/a/b"), None);
        assert_eq!(clicked("other:agent/a/b"), None);
    }
}
