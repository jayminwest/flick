//! Module `task`: a minimal task list (title, project, status) and a timer for the one
//! running task, on this machine only. Event driven: no timer, no polling. The timer is a
//! `core::track::Clock` whose subject is the running task: idle ends the task's time row
//! when input stopped, input reopens it; sleep and screen lock pause it, wake and unlock
//! resume it. The open row's `end` advances on every event, so a crash loses at most the
//! time since the last event; quit ends it. The running task survives reload and restart
//! (`task_state`); the time Flick was down is not counted.
//! Every change of the running task posts `Event::TaskChanged`, also at `Started` and, with a
//! task running, at `Reloaded` (for an `activity` the reload enabled), so other modules learn
//! it without reading this module's tables.
//! Launcher: root items Start/Stop/Switch Task, Tasks and Tasks Today, views `task/pick`,
//! `task/list` and `task/today` (`view.rs`, ids there), a task row's ⌘K menu (Start or Stop,
//! Mark Done or Reopen, Rename, Delete; `manage.rs`).
//! Durations are computed when a list refreshes. Table `[task]`: `hotkey` (unbound by
//! default) opens `task/pick`.

mod cli;
mod manage;
mod store;
#[cfg(test)]
mod tests;
mod view;
mod wire;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::Section;
use crate::core::track::{Clock, Input, Op, Span};
use crate::core::{Action, Binding, Cx, Event, Form, Item, ItemId, ListView, Module, Outcome};
use crate::core::store::Store;
use cli::{Listing, Range, Report, Verb};
use store::{MIGRATIONS, Status, Task, TaskStore};
use wire::Env;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    /// Opens `task/pick`.
    hotkey: Option<String>,
}

#[derive(Default)]
pub struct Tasks {
    env: Env,
    settings: Settings,
    /// Runs while a task runs; its subject is the task id.
    clock: Clock<i64>,
    /// Row id of the open `task_time` row.
    open: Option<i64>,
    /// The running task's id.
    running: Option<i64>,
    away: Away,
}

/// Why the machine is not in use; the running task's time pauses while either holds.
#[derive(Clone, Copy, Debug, Default)]
struct Away {
    /// The screen is locked: paused until `Unlocked`.
    locked: bool,
    /// The machine sleeps: paused until `Wake`.
    asleep: bool,
}

impl Away {
    fn any(self) -> bool {
        self.locked || self.asleep
    }
}

/// What a verb that changes tasks did: its status line and the task it is about.
type Changed = Result<(String, Option<i64>), String>;

/// The JSON answer of a verb that changes tasks.
#[derive(Serialize)]
struct Answer<'a> {
    message: &'a str,
    task: Option<Task>,
    running: Option<i64>,
}

impl Tasks {
    /// Persist the clock's ops.
    fn apply(&mut self, input: Input<i64>, store: &Store, now: i64) {
        for op in self.clock.observe(input, now) {
            match op {
                Op::Open { subject, at } => {
                    // A flicker's replacement starts in the past: its end is now.
                    self.open = store.time_open(subject, at);
                    store.set_open_row(self.open);
                    self.touch(store, now);
                }
                Op::Close { at } => {
                    if let Some(id) = self.open.take() {
                        store.time_end(id, at);
                        store.set_open_row(None);
                    }
                }
                Op::Extend { at } => self.touch(store, at),
                Op::Discard => {
                    if let Some(id) = self.open.take() {
                        store.time_delete(id);
                        store.set_open_row(None);
                    }
                }
            }
        }
    }

    /// Advance the open row to `now`.
    fn touch(&self, store: &Store, now: i64) {
        if let Some(id) = self.open {
            store.time_end(id, now);
        }
    }

    /// Take over the stored state after a start or restart: a row left open ends at its last
    /// event (the time down is not counted), and the running task opens a new one.
    fn start_up(&mut self, store: &Store, now: i64) {
        self.clock = Clock::default();
        self.open = None;
        self.away = Away::default();
        if let Some(id) = store.open_row() {
            match store.time_row(id) {
                Some(row) => {
                    self.clock.restore(row.subject, row.start);
                    self.open = Some(id);
                    self.apply(Input::Pause, store, row.end);
                }
                None => store.set_open_row(None),
            }
        }
        self.running = store.running().filter(|&id| store.task_get(id).is_some());
        if let Some(task) = self.running {
            self.apply(Input::Resume(task), store, now);
            (self.env.on_quit)(store);
            (self.env.post)(Event::TaskChanged { task: Some(task) });
        }
    }

    /// Count the running task again, unless the machine is still away.
    fn resume(&mut self, store: &Store, now: i64) {
        if let Some(task) = self.running.filter(|_| !self.away.any()) {
            self.apply(Input::Resume(task), store, now);
        }
    }

    /// Run task `task`, switching from the running one; the status line.
    fn start(&mut self, task: &Task, store: &Store, now: i64) -> String {
        if self.running == Some(task.id) {
            return format!("Already running: {}", cli::label(task));
        }
        let switched = self.running.replace(task.id).is_some();
        store.task_set_status(task.id, Status::Doing, now);
        store.set_running(Some(task.id));
        // Stopped: count again now. Away: the clock waits for wake or unlock.
        let input = if self.clock.is_paused() && !self.away.any() {
            Input::Resume(task.id)
        } else {
            Input::Focus(task.id)
        };
        self.apply(input, store, now);
        (self.env.on_quit)(store);
        (self.env.post)(Event::TaskChanged { task: Some(task.id) });
        format!("{} {}", if switched { "Switched to" } else { "Started" }, cli::label(task))
    }

    /// Stop the running task; its id.
    fn stop(&mut self, store: &Store, now: i64) -> Option<i64> {
        let task = self.running.take()?;
        self.apply(Input::Pause, store, now);
        store.set_running(None);
        (self.env.post)(Event::TaskChanged { task: None });
        Some(task)
    }

    /// `start <target>`: a target that matches nothing becomes a new task.
    fn start_target(&mut self, target: &str, project: Option<&str>, store: &Store, now: i64) -> Changed {
        let id = match cli::resolve(target, &store.task_list(true), project)? {
            Some(id) => id,
            None => store.task_add(target, project, now)?,
        };
        let task = store.task_get(id).ok_or("task: the task is gone")?;
        Ok((self.start(&task, store, now), Some(id)))
    }

    fn done(&mut self, target: &str, store: &Store, now: i64) -> Changed {
        let id = cli::resolve(target, &store.task_list(true), None)?
            .ok_or_else(|| format!("task: no task matches \"{target}\""))?;
        if self.running == Some(id) {
            self.stop(store, now);
        }
        store.task_set_status(id, Status::Done, now);
        Ok(("Done".into(), Some(id)))
    }

    /// Answer a verb that changed task `id`: the status line (with the task's label after
    /// "Added", "Done", "Reopened" and "Renamed"), or with `json` the JSON document.
    fn answer(&self, changed: Changed, json: bool, store: &Store) -> Result<String, String> {
        let (message, id) = changed?;
        let task = id.and_then(|id| store.task_get(id));
        if json {
            return self::json(&Answer { message: &message, task, running: self.running });
        }
        Ok(match task {
            Some(t) if ["Added", "Done", "Reopened", "Renamed"].contains(&message.as_str()) => {
                format!("{message} {}: {}", t.id, cli::label(&t))
            }
            _ => message,
        })
    }

    /// Today's range and time rows clipped to it, the open row counted up to `now`.
    fn today(&self, store: &Store, now: i64) -> Option<(Range, Vec<Span<i64>>)> {
        let offset = (self.env.utc_offset)(now);
        let range = Range::new("today", cli::parse_range("today", now, offset)?, offset);
        let mut spans = store.times(range.from, range.to);
        if let Some(row) = self.open.and_then(|id| store.time_row(id)) {
            spans.push(Span { start: row.end, end: now, subject: row.subject });
        }
        let spans = cli::clip(spans, range.from, range.to);
        Some((range, spans))
    }

    /// Seconds per task today, counted up to `now`.
    fn today_secs(&self, store: &Store, now: i64) -> HashMap<i64, i64> {
        let mut secs = HashMap::new();
        for s in self.today(store, now).map(|(_, spans)| spans).unwrap_or_default() {
            *secs.entry(s.subject).or_insert(0) += s.secs();
        }
        secs
    }

    fn ls(&self, all: bool, project: Option<&str>, store: &Store, now: i64) -> Listing {
        let spans = self.today(store, now).map(|(_, spans)| spans).unwrap_or_default();
        let tasks: Vec<Task> = store
            .task_list(all)
            .into_iter()
            .filter(|t| project.is_none_or(|p| t.project.as_deref() == Some(p)))
            .collect();
        let running = self.running.and_then(|id| store.task_get(id));
        let since = self.clock.open().map(|(_, start)| start);
        Listing::new(&tasks, &spans, running.as_ref().map(|t| (t, since)))
    }

    /// Start task `id` from the launcher and hide it.
    fn start_id(&mut self, id: i64, store: &Store, now: i64) -> Outcome {
        match store.task_get(id) {
            Some(task) => {
                self.start(&task, store, now);
                Outcome::Hide
            }
            None => Outcome::Stay(Some(format!("task: no task {id}"))),
        }
    }

    /// 'Start new task': the task with exactly this title and project, else a new one.
    fn start_new(&mut self, query: &str, store: &Store, now: i64) -> Outcome {
        let (title, project) = view::split_project(query);
        let same = store.task_list(true).into_iter().find(|t| t.title == title && t.project == project);
        match same.map_or_else(|| store.task_add(&title, project.as_deref(), now), |t| Ok(t.id)) {
            Ok(id) => self.start_id(id, store, now),
            Err(e) => Outcome::Stay(Some(e)),
        }
    }

    /// Stop the running task; the status line.
    fn stop_status(&mut self, store: &Store, now: i64) -> String {
        match self.stop(store, now).and_then(|id| store.task_get(id)) {
            Some(t) => format!("Stopped {}", cli::label(&t)),
            None => "No task running".into(),
        }
    }

    fn report(&self, range: &str, project: Option<&str>, store: &Store, now: i64) -> Result<Report, String> {
        let offset = (self.env.utc_offset)(now);
        let days = cli::parse_range(range, now, offset).ok_or_else(|| {
            format!("task: bad range \"{range}\" (today, week, YYYY-MM-DD or YYYY-MM-DD..YYYY-MM-DD)")
        })?;
        let range = Range::new(range, days, offset);
        let spans = cli::clip(store.times(range.from, range.to), range.from, range.to);
        Ok(Report::new(range, &spans, &store.task_list(true), project))
    }
}

fn json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("task: {e}"))
}

impl Module for Tasks {
    fn id(&self) -> &'static str {
        "task"
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.settings = table.get::<Settings>()?;
        Ok(())
    }

    fn items(&mut self, cx: &mut Cx) -> Vec<Item> {
        let now = (self.env.now)();
        let running = self.running.and_then(|id| cx.store.task_get(id));
        let secs = running.as_ref().map_or(0, |t| self.today_secs(cx.store, now).get(&t.id).copied().unwrap_or(0));
        view::root_items(running.as_ref().map(|t| (t, secs)))
    }

    fn open(&mut self, name: &str, _cx: &mut Cx) -> Option<ListView> {
        match name {
            view::PICK => Some(view::pick()),
            view::LIST => Some(view::list()),
            view::TODAY => Some(view::today()),
            _ => None,
        }
    }

    fn refresh(&mut self, list: &mut ListView, cx: &mut Cx) {
        let (store, now) = (cx.store, (self.env.now)());
        list.items = if list.name == view::PICK || list.name == view::LIST {
            let secs = self.today_secs(store, now);
            let all = list.name == view::LIST;
            let items = if all { view::list_items } else { view::pick_items };
            items(cx.query, &store.task_list(all), &secs, self.running, cx.ranker)
        } else {
            let running = self.running.and_then(|id| store.task_get(id));
            self.today(store, now)
                .map(|(range, spans)| Report::new(range, &spans, &store.task_list(true), None))
                .map(|r| view::today_items(cx.query, &r, running.as_ref(), cx.ranker))
                .unwrap_or_default()
        };
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        let (store, now) = (cx.store, (self.env.now)());
        match id.key() {
            "start" | "switch" => Outcome::Push(view::pick()),
            view::LIST => Outcome::Push(view::list()),
            view::TODAY => Outcome::Push(view::today()),
            "stop" => Outcome::Stay(Some(self.stop_status(store, now))),
            view::NEW => self.start_new(id.arg().unwrap_or_default(), store, now),
            key => match view::task_id(key) {
                Some(task) => self.start_id(task, store, now),
                None => Outcome::Stay(None),
            },
        }
    }

    fn actions(&mut self, id: &ItemId, cx: &mut Cx) -> Vec<Action> {
        view::task_id(id.key()).map_or_else(Vec::new, |task| self.task_actions(task, cx.store))
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        let (store, now) = (cx.store, (self.env.now)());
        let Some(task) = view::task_id(id.key()) else { return Outcome::Stay(None) };
        match key {
            "start" => self.start_id(task, store, now),
            "stop" => Outcome::Stay(Some(self.stop_status(store, now))),
            "done" => {
                let changed = self.done(&task.to_string(), store, now);
                Outcome::Stay(Some(self.answer(changed, false, store).unwrap_or_else(|e| e)))
            }
            key => self.manage_act(task, key, store, now),
        }
    }

    fn form(&mut self, name: &str, cx: &mut Cx) -> Option<Form> {
        manage::rename_form(name, cx.store)
    }

    fn submit(&mut self, form: &Form, cx: &mut Cx) -> Result<String, String> {
        manage::submit_rename(form, cx.store, (self.env.now)())
    }

    fn confirmed(&mut self, token: &str, cx: &mut Cx) -> Outcome {
        self.delete_confirmed(token, cx.store, (self.env.now)())
    }

    /// `hotkey` binds key `pick`.
    fn hotkeys(&self) -> Vec<Binding> {
        let spec = self.settings.hotkey.as_ref().filter(|s| !s.trim().is_empty());
        spec.map(|spec| Binding { spec: spec.clone(), key: Ok(view::PICK.into()) }).into_iter().collect()
    }

    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        (key == view::PICK).then(view::pick)
    }

    /// `TaskChanged` (from the launcher, the CLI or a restart) makes the views stale. At
    /// `Reloaded` the running task is posted again, unchanged, so a module the reload just
    /// started (`activity`) tags its spans with it (flick-4bb4).
    fn on_event(&mut self, event: Event, cx: &mut Cx) -> bool {
        let (store, now) = (cx.store, (self.env.now)());
        match event {
            Event::TaskChanged { .. } => {
                self.touch(store, now);
                return true;
            }
            Event::Started => self.start_up(store, now),
            Event::Reloaded => {
                self.touch(store, now);
                if let Some(task) = self.running {
                    (self.env.post)(Event::TaskChanged { task: Some(task) });
                }
            }
            Event::Sleep | Event::Locked => {
                self.away.asleep |= event == Event::Sleep;
                self.away.locked |= event == Event::Locked;
                self.apply(Input::Pause, store, now);
            }
            Event::Wake => {
                self.away.asleep = false;
                self.resume(store, now);
            }
            Event::Unlocked if self.away.locked => {
                self.away.locked = false;
                self.resume(store, now);
            }
            Event::Idle { secs } => self.apply(Input::Idle { secs }, store, now),
            Event::Active => self.apply(Input::Active, store, now),
            _ => self.touch(store, now),
        }
        false
    }

    fn verbs(&self) -> &'static str {
        "task start|switch <id|title> [--project P] | task stop | task ls [--all] [--project P] | task add <title> [--project P] | task done|reopen|rm <id|title> | task rename <id|title> <new title> [--project P] | task report [today|week|<date>[..<date>]] [--project P]"
    }

    /// `--json` (`cx.json`) makes every verb answer with JSON.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        let verb = cli::parse(args)?;
        let (store, now) = (cx.store, (self.env.now)());
        self.touch(store, now);
        match verb {
            Verb::Ls { all, project } => {
                let listing = self.ls(all, project.as_deref(), store, now);
                if cx.json { json(&listing) } else { Ok(listing.text()) }
            }
            Verb::Report { range, project } => {
                let report = self.report(&range, project.as_deref(), store, now)?;
                if cx.json { json(&report) } else { Ok(report.text()) }
            }
            Verb::Start { target, project } => {
                let changed = self.start_target(&target, project.as_deref(), store, now);
                self.answer(changed, cx.json, store)
            }
            Verb::Stop => {
                let task = self.running;
                let message = self.stop_status(store, now);
                self.answer(Ok((message, task)), cx.json, store)
            }
            Verb::Add { title, project } => {
                let id = store.task_add(&title, project.as_deref(), now)?;
                self.answer(Ok(("Added".into(), Some(id))), cx.json, store)
            }
            Verb::Done { target } => {
                let changed = self.done(&target, store, now);
                self.answer(changed, cx.json, store)
            }
            Verb::Reopen { target } => self.answer(manage::reopen(&target, store, now), cx.json, store),
            Verb::Rename { target, title, project } => {
                let changed = manage::rename(&target, &title, project.as_deref(), store, now);
                self.answer(changed, cx.json, store)
            }
            Verb::Rm { target } => {
                let changed = self.remove(&target, store, now);
                self.answer(changed, cx.json, store)
            }
        }
    }
}
