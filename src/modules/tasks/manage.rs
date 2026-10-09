//! Managing tasks rather than timing them: reopen, rename and delete, from the CLI
//! (`task reopen|rename|rm`) and from a task row's ⌘K menu (Reopen Task, form
//! `rename/<id>`, a destructive confirmation with token `delete/<id>`).

use super::cli::{self, label};
use super::store::{Status, Task, TaskStore};
use super::{Changed, Tasks, view};
use crate::core::store::Store;
use crate::core::{Action, Confirm, ConfirmRow, Field, Form, Outcome};

/// Form name prefix: `rename/<id>`.
const RENAME: &str = "rename/";
/// Confirmation token prefix: `delete/<id>`.
const DELETE: &str = "delete/";

/// The task `target` names among every task, done ones too.
fn find(target: &str, store: &Store) -> Result<Task, String> {
    cli::resolve(target, &store.task_list(true), None)?
        .and_then(|id| store.task_get(id))
        .ok_or_else(|| format!("task: no task matches \"{target}\""))
}

/// A typed project: `#` and blanks dropped, empty for none.
fn project(text: &str) -> Option<String> {
    Some(text.trim().trim_start_matches('#').trim().to_owned()).filter(|p| !p.is_empty())
}

pub(super) fn reopen(target: &str, store: &Store, now: i64) -> Changed {
    let task = find(target, store)?;
    store.task_set_status(task.id, Status::Todo, now);
    Ok(("Reopened".into(), Some(task.id)))
}

/// `project`: `None` keeps the task's project, `Some("")` clears it.
pub(super) fn rename(target: &str, title: &str, project: Option<&str>, store: &Store, now: i64) -> Changed {
    let task = find(target, store)?;
    let project = project.map_or(task.project, self::project);
    store.task_rename(task.id, title.trim(), project.as_deref(), now)?;
    Ok(("Renamed".into(), Some(task.id)))
}

/// Form `rename/<id>`, filled with that task.
pub(super) fn rename_form(name: &str, store: &Store) -> Option<Form> {
    let task = store.task_get(name.strip_prefix(RENAME)?.parse().ok()?)?;
    let fields = vec![
        Field::new("title", "Title").required().value(task.title).placeholder("Task title"),
        Field::new("project", "Project").value(task.project.unwrap_or_default()).placeholder("Optional: no project"),
    ];
    Some(Form { fields, submit_label: "Rename Task".into(), ..Form::new("task", name, "Rename Task") })
}

pub(super) fn submit_rename(form: &Form, store: &Store, now: i64) -> Result<String, String> {
    let id = form.name.strip_prefix(RENAME).ok_or("task: not a task form")?;
    let field = |key| form.value(key).unwrap_or_default();
    let (message, id) = rename(id, field("title"), Some(field("project")), store, now)?;
    let task = id.and_then(|id| store.task_get(id)).ok_or("task: the task is gone")?;
    Ok(format!("{message} {}", label(&task)))
}

impl Tasks {

    /// Delete a task and its time, stopping it first when it runs.
    pub(super) fn remove(&mut self, target: &str, store: &Store, now: i64) -> Changed {
        let task = find(target, store)?;
        if self.running == Some(task.id) {
            self.stop(store, now);
        }
        store.task_delete(task.id);
        Ok((format!("Deleted {}: {}", task.id, label(&task)), None))
    }

    /// The ⌘K menu of task row `task`; empty when the task is gone.
    pub(super) fn task_actions(&self, task: i64, store: &Store) -> Vec<Action> {
        store.task_get(task).map_or_else(Vec::new, |t| view::actions(self.running == Some(t.id), t.status == Status::Done))
    }

    /// Actions `reopen`, `rename` and `delete` on task row `task`.
    pub(super) fn manage_act(&self, task: i64, key: &str, store: &Store, now: i64) -> Outcome {
        let Some(t) = store.task_get(task) else { return Outcome::Stay(Some(format!("task: no task {task}"))) };
        match key {
            "reopen" => {
                let changed = reopen(&task.to_string(), store, now);
                Outcome::Stay(Some(self.answer(changed, false, store).unwrap_or_else(|e| e)))
            }
            "rename" => Outcome::Form { module: "task", name: format!("{RENAME}{task}") },
            "delete" => Outcome::Confirm(Confirm {
                rows: vec![ConfirmRow {
                    subtitle: "Its tracked time is deleted too".into(),
                    accessory: t.status.as_str().into(),
                    ..ConfirmRow::new(label(&t))
                }],
                label: "Delete Task".into(),
                destructive: true,
                ..Confirm::new("task", format!("{DELETE}{task}"), format!("Delete task \"{}\"?", t.title))
            }),
            _ => Outcome::Stay(None),
        }
    }

    pub(super) fn delete_confirmed(&mut self, token: &str, store: &Store, now: i64) -> Outcome {
        let Some(id) = token.strip_prefix(DELETE) else { return Outcome::Stay(None) };
        Outcome::Stay(Some(self.remove(id, store, now).map_or_else(|e| e, |(message, _)| message)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_projects_drop_the_hash() {
        assert_eq!(project(" #kota "), Some("kota".into()));
        assert_eq!((project(""), project(" # ")), (None, None));
    }
}
