//! The task module's launcher side, pure: root items, views `task/pick`, `task/list` and
//! `task/today`, the `#project` query syntax and a task row's action menu. Callers pass the
//! tasks, the seconds per task and the running task. Root item ids are fixed (`task:start`,
//! `task:stop`, `task:switch`, `task:list`, `task:today`); view rows are `task:run/<id>`, `task:new`
//! (the typed query rides as its arg) and `task:project/<name>`, and the views do not
//! record use, so the usage table never grows per task.

use std::collections::HashMap;

use super::cli::{Report, label};
use super::store::{Status, Task};
use crate::core::{Action, Icon, Item, ItemId, ListView, Ranker, Tone};

/// The view that starts or switches to a task.
pub const PICK: &str = "pick";
/// Every task, open ones first.
pub const LIST: &str = "list";
/// Today's totals.
pub const TODAY: &str = "today";
/// Key prefix of a task row: `task:run/<id>`.
const RUN: &str = "run/";
/// Key prefix of a project total in `task/today`: `task:project/<name>` (empty: none).
pub const PROJECT: &str = "project/";
/// Key of the "Start new task" row.
pub const NEW: &str = "new";

/// "1:05" (hours and minutes).
pub fn clock(secs: i64) -> String {
    let secs = secs.max(0);
    format!("{}:{:02}", secs / 3600, secs % 3600 / 60)
}

fn item(key: impl std::fmt::Display, title: impl Into<String>, verb: &'static str, symbol: &'static str) -> Item {
    Item { accessory: "Task".into(), ..Item::new(ItemId::new("task", key), title, verb, Icon::Symbol(symbol)) }
}

/// 'Start Task' while none runs; 'Stop Task: <title> · 0:42' (`secs` today) and 'Switch
/// Task' while one does; always 'Tasks' and 'Tasks Today'.
pub fn root_items(running: Option<(&Task, i64)>) -> Vec<Item> {
    let mut items = match running {
        None => vec![Item {
            keywords: vec!["timer track time".into()],
            ..item("start", "Start Task", "Open", "play.circle")
        }],
        Some((task, secs)) => vec![
            Item {
                subtitle: label(task),
                keywords: vec!["timer track time".into()],
                ..item("stop", format!("Stop Task: {} · {}", task.title, clock(secs)), "Stop", "stop.circle")
            },
            Item {
                keywords: vec!["start timer track time".into()],
                ..item("switch", "Switch Task", "Open", "arrow.triangle.swap")
            },
        ],
    };
    items.push(Item {
        keywords: vec!["list all todo done rename delete".into()],
        ..item(LIST, "Tasks", "Open", "checklist")
    });
    items.push(Item {
        keywords: vec!["time report totals".into()],
        ..item(TODAY, "Tasks Today", "Open", "chart.bar")
    });
    items
}

pub fn pick() -> ListView {
    ListView {
        placeholder: "Start a task… (#project sets its project)".into(),
        footer: "Start Task  ·  ⌘K for more  ·  esc to go back".into(),
        empty: "No tasks; type a title to start one".into(),
        ..ListView::new("task", PICK)
    }
}

pub fn list() -> ListView {
    ListView {
        placeholder: "Search tasks… (#project filters)".into(),
        footer: "Tasks  ·  ⌘K for more  ·  esc to go back".into(),
        empty: "No tasks; run Start Task to add one".into(),
        ..ListView::new("task", LIST)
    }
}

pub fn today() -> ListView {
    ListView {
        placeholder: "Search today's tasks…".into(),
        footer: "Tasks Today  ·  esc to go back".into(),
        empty: "No time tracked today".into(),
        ..ListView::new("task", TODAY)
    }
}

/// The task id of key `run/<id>`.
pub fn task_id(key: &str) -> Option<i64> {
    key.strip_prefix(RUN)?.parse().ok()
}

/// `query` split into a title and the project of a trailing `#word` ("Write plan #flick").
pub fn split_project(query: &str) -> (String, Option<String>) {
    let query = query.trim();
    match query.rsplit_once(char::is_whitespace).map_or(("", query), |(a, b)| (a, b)) {
        (title, word) if word.len() > 1 && word.starts_with('#') => {
            (title.trim().to_owned(), Some(word[1..].to_owned()))
        }
        _ => (query.to_owned(), None),
    }
}

/// A task row: Enter starts (or switches to) it.
fn task_row(task: &Task, secs: i64, running: Option<i64>) -> Item {
    let (verb, accessory) = match running {
        Some(id) if id == task.id => ("Keep Running", format!("Running · {}", clock(secs))),
        Some(_) => ("Switch to Task", clock(secs)),
        None => ("Start Task", clock(secs)),
    };
    let accessory = if task.status == Status::Done { format!("Done · {accessory}") } else { accessory };
    Item {
        subtitle: task.project.as_ref().map(|p| format!("#{p}")).unwrap_or_default(),
        accessory,
        keywords: task.project.iter().cloned().collect(),
        ..Item::new(ItemId::new("task", format!("{RUN}{}", task.id)), &task.title, verb, Icon::Symbol("circle"))
    }
}

/// `task/pick` for `query`: `tasks` (not done, in store order) ranked by the title part,
/// only in the typed project when there is one; the running one first while the query is
/// empty. Above them 'Start new task "<title>"' unless a task has that title already.
pub fn pick_items(
    query: &str,
    tasks: &[Task],
    secs: &HashMap<i64, i64>,
    running: Option<i64>,
    ranker: &mut Ranker,
) -> Vec<Item> {
    let (title, project) = split_project(query);
    let in_project = |t: &Task| {
        project.as_ref().is_none_or(|p| t.project.as_ref().is_some_and(|tp| tp.to_lowercase().starts_with(&p.to_lowercase())))
    };
    let rows: Vec<Item> = tasks
        .iter()
        .filter(|t| in_project(t))
        .map(|t| task_row(t, secs.get(&t.id).copied().unwrap_or(0), running))
        .collect();
    let order: HashMap<String, f64> = rows
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let first = title.is_empty() && task_id(item.id.key()) == running && running.is_some();
            (item.id.to_string(), if first { 1e3 } else { -(i as f64) * 1e-3 })
        })
        .collect();
    let mut items = ranker.rank(&title, rows, |i| order[i.id.as_str()]);
    let exists = tasks.iter().any(|t| {
        t.title.eq_ignore_ascii_case(&title) && project.as_ref().is_none_or(|p| t.project.as_ref() == Some(p))
    });
    if !title.is_empty() && !exists {
        let new = Item {
            subtitle: project.map(|p| format!("#{p}")).unwrap_or_default(),
            ..Item::new(
                ItemId::new("task", NEW).with_arg(query.trim()),
                format!("Start new task \"{title}\""),
                "Start Task",
                Icon::Symbol("plus.circle"),
            )
        };
        items.insert(0, new);
    }
    items
}

/// `task/list` for `query`: every task, open ones then done ones (store order otherwise,
/// the running one first), each with its time today. Ranked by the title part and only in
/// the typed project when there is one; open tasks stay above done ones.
pub fn list_items(
    query: &str,
    tasks: &[Task],
    secs: &HashMap<i64, i64>,
    running: Option<i64>,
    ranker: &mut Ranker,
) -> Vec<Item> {
    let (title, project) = split_project(query);
    let in_project = |t: &&Task| {
        project.as_ref().is_none_or(|p| t.project.as_ref().is_some_and(|tp| tp.to_lowercase().starts_with(&p.to_lowercase())))
    };
    let mut tasks: Vec<&Task> = tasks.iter().filter(in_project).collect();
    tasks.sort_by_key(|t| (t.status == Status::Done, Some(t.id) != running));
    let done: HashMap<String, bool> = tasks
        .iter()
        .map(|t| (ItemId::new("task", format!("{RUN}{}", t.id)).to_string(), t.status == Status::Done))
        .collect();
    let rows: Vec<Item> = tasks
        .iter()
        .map(|t| {
            let (symbol, tone) = match t.status {
                Status::Done => ("checkmark.circle", Tone::Ok),
                _ if Some(t.id) == running => ("play.circle", Tone::Ok),
                _ => ("circle", Tone::Neutral),
            };
            Item { icon: Icon::Symbol(symbol), tone, ..task_row(t, secs.get(&t.id).copied().unwrap_or(0), running) }
        })
        .collect();
    let order: HashMap<String, f64> =
        rows.iter().enumerate().map(|(i, item)| (item.id.to_string(), -(i as f64) * 1e-3)).collect();
    let mut items = ranker.rank(&title, rows, |i| order[i.id.as_str()]);
    items.sort_by_key(|i| done[i.id.as_str()]);
    items
}

/// `task/today`: the running task first (counted to now), then each task by its time;
/// below them the total per project. Filtered and ranked by `query`, in that order.
pub fn today_items(query: &str, report: &Report, running: Option<&Task>, ranker: &mut Ranker) -> Vec<Item> {
    let run_id = running.map(|t| t.id);
    let mut rows: Vec<Item> = vec![];
    if let Some(task) = running.filter(|t| !report.tasks.iter().any(|r| r.id == t.id)) {
        rows.push(task_row(task, 0, run_id));
    }
    let mut totals: Vec<_> = report.tasks.iter().collect();
    totals.sort_by_key(|t| Some(t.id) != run_id);
    for t in totals {
        let task = Task { id: t.id, title: t.title.clone(), project: t.project.clone(), status: Status::Doing };
        rows.push(task_row(&task, t.secs, run_id));
    }
    for p in &report.projects {
        let name = p.project.as_deref().unwrap_or_default();
        let title = if name.is_empty() { "No project".to_owned() } else { format!("#{name}") };
        rows.push(Item {
            subtitle: "Project total".into(),
            accessory: clock(p.secs),
            ..Item::new(ItemId::new("task", format!("{PROJECT}{name}")), title, "Total", Icon::Symbol("folder"))
        });
    }
    let order: HashMap<String, f64> =
        rows.iter().enumerate().map(|(i, item)| (item.id.to_string(), -(i as f64))).collect();
    ranker.rank(query, rows, |i| order[i.id.as_str()] * 1e-3)
}

/// The action menu of a task row: Start Task (Stop Task on the running one), Mark Done
/// (Reopen Task on a done one), Rename Task and Delete Task.
pub fn actions(running: bool, done: bool) -> Vec<Action> {
    vec![
        if running {
            Action::new("stop", "Stop Task", Icon::Symbol("stop.circle"))
        } else {
            Action::new("start", "Start Task", Icon::Symbol("play.circle"))
        },
        if done {
            Action::new("reopen", "Reopen Task", Icon::Symbol("arrow.uturn.backward.circle"))
        } else {
            Action::new("done", "Mark Done", Icon::Symbol("checkmark.circle"))
        },
        Action::new("rename", "Rename Task", Icon::Symbol("pencil")),
        Action::new("delete", "Delete Task", Icon::Symbol("trash")),
    ]
}

#[cfg(test)]
mod tests {
    use crate::modules::tasks::cli::{ProjectTotal, Range, TaskTotal};
    use super::*;

    fn task(id: i64, title: &str, project: Option<&str>) -> Task {
        Task { id, title: title.into(), project: project.map(String::from), status: Status::Todo }
    }

    fn ids(items: &[Item]) -> Vec<String> {
        items.iter().map(|i| i.id.to_string()).collect()
    }

    #[test]
    fn clocks_and_keys() {
        assert_eq!((clock(-1), clock(42 * 60 + 59), clock(3600 + 5 * 60)), ("0:00".into(), "0:42".into(), "1:05".into()));
        assert_eq!((task_id("run/7"), task_id("run/x"), task_id("today")), (Some(7), None, None));
    }

    #[test]
    fn a_trailing_hash_word_is_the_project() {
        let s = |q: &str| split_project(q);
        assert_eq!(s(" Review PR #kota "), ("Review PR".into(), Some("kota".into())));
        assert_eq!(s("#kota"), (String::new(), Some("kota".into())));
        assert_eq!(s("Fix #12 now"), ("Fix #12 now".into(), None));
        assert_eq!(s("Fix #"), ("Fix #".into(), None));
        assert_eq!(s("Mail"), ("Mail".into(), None));
    }

    #[test]
    fn root_items_follow_the_running_task() {
        assert_eq!(ids(&root_items(None)), ["task:start", "task:list", "task:today"]);
        let t = task(3, "Review PR", Some("kota"));
        let items = root_items(Some((&t, 180)));
        assert_eq!(ids(&items), ["task:stop", "task:switch", "task:list", "task:today"]);
        assert_eq!((items[0].title.as_str(), items[0].subtitle.as_str()), ("Stop Task: Review PR · 0:03", "Review PR #kota"));
    }

    #[test]
    fn pick_offers_a_new_task_unless_one_has_that_title() {
        let tasks = [task(1, "Mail", None), task(2, "Write plan", Some("flick")), task(3, "Review", Some("kota"))];
        let secs = HashMap::from([(2, 120)]);
        let mut ranker = Ranker::new();
        let mut pick = |q: &str, running| pick_items(q, &tasks, &secs, running, &mut ranker);
        // Empty query: store order, the running task first.
        assert_eq!(ids(&pick("", None)), ["task:run/1", "task:run/2", "task:run/3"]);
        let items = pick("", Some(2));
        assert_eq!(ids(&items), ["task:run/2", "task:run/1", "task:run/3"]);
        assert_eq!((items[0].accessory.as_str(), items[0].verb, items[1].verb), ("Running · 0:02", "Keep Running", "Switch to Task"));
        let items = pick("Review PR #kota", None);
        assert_eq!(ids(&items), ["task:new"]);
        assert_eq!((items[0].title.as_str(), items[0].subtitle.as_str()), ("Start new task \"Review PR\"", "#kota"));
        assert_eq!(items[0].id.arg(), Some("Review PR #kota"));
        assert_eq!(ids(&pick("rev #ko", None)), ["task:new", "task:run/3"]);
        assert_eq!(ids(&pick("#flick", None)), ["task:run/2"]);
        assert_eq!(ids(&pick("mail", None)), ["task:run/1"]);
        assert_eq!(ids(&pick("Write plan #kota", None))[0], "task:new");
        assert_eq!(pick("Mail", None)[0].verb, "Start Task");
    }

    #[test]
    fn today_lists_the_running_task_first_then_projects() {
        let range = Range::new("today", (0, 0), 0);
        let report = Report {
            range,
            tasks: vec![
                TaskTotal { id: 1, title: "Mail".into(), project: None, secs: 600 },
                TaskTotal { id: 2, title: "Write".into(), project: Some("flick".into()), secs: 60 },
            ],
            projects: vec![
                ProjectTotal { project: None, secs: 600 },
                ProjectTotal { project: Some("flick".into()), secs: 60 },
            ],
            total_secs: 660,
        };
        let mut ranker = Ranker::new();
        let write = task(2, "Write", Some("flick"));
        let items = today_items("", &report, Some(&write), &mut ranker);
        assert_eq!(ids(&items), ["task:run/2", "task:run/1", "task:project/", "task:project/flick"]);
        assert_eq!((items[0].accessory.as_str(), items[2].title.as_str(), items[3].title.as_str()), ("Running · 0:01", "No project", "#flick"));
        // A task started just now has no time yet but still leads.
        let fresh = task(5, "New", None);
        assert_eq!(ids(&today_items("", &report, Some(&fresh), &mut ranker))[..2], ["task:run/5", "task:run/1"]);
        assert_eq!(ids(&today_items("mail", &report, None, &mut ranker)), ["task:run/1"]);
    }

    #[test]
    fn list_shows_open_tasks_then_done_ones() {
        let mut done = task(4, "Mail", None, );
        done.status = Status::Done;
        let tasks = [task(1, "Write plan", Some("flick")), done, task(3, "Review", Some("kota"))];
        let secs = HashMap::from([(4, 60)]);
        let mut ranker = Ranker::new();
        let mut list = |q: &str, running| list_items(q, &tasks, &secs, running, &mut ranker);
        assert_eq!(ids(&list("", None)), ["task:run/1", "task:run/3", "task:run/4"]);
        let items = list("", Some(3));
        assert_eq!(ids(&items), ["task:run/3", "task:run/1", "task:run/4"]);
        assert_eq!(items.iter().map(|i| &i.icon).collect::<Vec<_>>(), [&Icon::Symbol("play.circle"), &Icon::Symbol("circle"), &Icon::Symbol("checkmark.circle")]);
        assert_eq!((items[0].accessory.as_str(), items[2].accessory.as_str()), ("Running · 0:00", "Done · 0:01"));
        assert_eq!(items.iter().map(|i| i.tone).collect::<Vec<_>>(), [Tone::Ok, Tone::Neutral, Tone::Ok]);
        // A done task that matches better still comes after the open ones.
        assert_eq!(ids(&list("ma", None)), ["task:run/4"]);
        assert_eq!(ids(&list("#kota", None)), ["task:run/3"]);
        assert!(list("zzz", None).is_empty());
        assert!(list_items("", &[], &secs, None, &mut ranker).is_empty());
    }

    #[test]
    fn actions_follow_running_and_done() {
        let keys = |running, done| actions(running, done).into_iter().map(|a| a.key).collect::<Vec<_>>();
        assert_eq!(keys(false, false), ["start", "done", "rename", "delete"]);
        assert_eq!(keys(true, false), ["stop", "done", "rename", "delete"]);
        assert_eq!(keys(false, true), ["start", "reopen", "rename", "delete"]);
    }
}
