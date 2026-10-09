//! Module `activity`: records which app is in front as time spans, on this machine only,
//! and reports them. Event driven: no timer, no polling. Recording is off until turned on
//! (`flick activity on`, the root item or the hotkey); the flag lives in `activity_state`,
//! not in config. Window titles are never read unless `titles = true`. Excluded apps leave
//! a gap. The open span is a row whose `end` advances on every event while recording, so a
//! crash loses at most the time since the last event.
//! While recording, a menu bar indicator shows; with `titles = true` the front app's window
//! is followed for title changes (`Event::WindowChanged`), and only then. Sleep and screen
//! lock pause recording; wake and unlock resume it. Quit closes the open span.
//! Spans carry the running task's id, learned from `Event::TaskChanged` (activity never
//! reads the task module's tables); a task change splits the open span.
//! Agent sessions (`Cx::remote`) read reports only with the user's grant (`remote`).
//! Ids are `activity:record`, `activity:today`, `activity:remote` and
//! `activity:row/<kind>/<name>` (view rows).

mod remote;
mod report;
mod rules;
mod store;
#[cfg(test)]
mod tests;
mod wire;

use crate::core::track::{Clock, Input, Op, Span, Subject};
use crate::core::{Binding, Cx, Event, Icon, Item, ItemId, ListView, Module, Outcome, unknown_verb};
use crate::core::store::Store;
use report::{TaskReport, Report, clip, parse_since, span_list, span_text};
use rules::Config;
use store::{MIGRATIONS, Spans};
use wire::Env;

#[derive(Default)]
pub struct Activity {
    config: Config,
    env: Env,
    clock: Clock,
    /// Row id of the open span.
    open: Option<i64>,
    /// The clock is paused because an excluded app is in front.
    excluded: bool,
    /// The app whose spans are open or would be (not Flick, not excluded).
    front: Option<i32>,
    /// The pid whose window titles are followed.
    watched: Option<i32>,
    /// The menu bar indicator is shown.
    indicated: bool,
    away: Away,
    /// The running task, from the last `Event::TaskChanged`.
    task: Option<i64>,
}

/// Why the machine is not in use; recording pauses while either holds.
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

impl Activity {
    /// Persist the clock's ops.
    fn apply(&mut self, input: Input<Subject>, store: &Store, now: i64) {
        for op in self.clock.observe(input, now) {
            match op {
                Op::Open { subject, at } => {
                    // A flicker's replacement starts in the past: its end is now.
                    self.open = store.span_open(&subject, at);
                    store.set_open_span(self.open);
                    self.touch(store, now);
                }
                Op::Close { at } => {
                    if let Some(id) = self.open.take() {
                        store.span_end(id, at);
                        store.set_open_span(None);
                    }
                }
                Op::Extend { at } => {
                    if let Some(id) = self.open {
                        store.span_end(id, at);
                    }
                }
                Op::Discard => {
                    if let Some(id) = self.open.take() {
                        store.span_delete(id);
                        store.set_open_span(None);
                    }
                }
            }
        }
    }

    /// The clock input for app `pid` coming to the front. `None`: ignore it (Flick itself,
    /// an app that quit).
    fn focus(&mut self, pid: i32) -> Option<Input<Subject>> {
        if pid == self.env.own_pid {
            return None;
        }
        let (id, name) = (self.env.identity)(pid)?;
        let app = id.unwrap_or_else(|| name.clone());
        if self.config.excludes(&app, &name) {
            self.excluded = true;
            self.front = None;
            return Some(Input::Pause);
        }
        self.front = Some(pid);
        // With `titles = false` no title is ever read.
        let title = if self.config.titles { (self.env.title)(pid) } else { None };
        let subject = Subject::new(&app, &name, title.as_deref(), self.task);
        Some(if std::mem::take(&mut self.excluded) {
            Input::Resume(subject)
        } else {
            Input::Focus(subject)
        })
    }

    /// Forget the in-memory clock and leave a span stored as open closed at its last `end`
    /// (after a restart, sleep, or a `forget`).
    fn settle(&mut self, store: &Store) {
        store.set_open_span(None);
        self.open = None;
        self.excluded = false;
        self.front = None;
        self.clock = Clock::new(self.config.merge_secs);
    }

    /// Start counting the frontmost app.
    fn begin(&mut self, store: &Store, now: i64) {
        if let Some(input) = (self.env.frontmost)().and_then(|pid| self.focus(pid)) {
            self.apply(input, store, now);
        }
    }

    /// Match the system to the recording state: the indicator shows while recording, and
    /// the front app's titles are followed only while recording with titles on and the
    /// machine in use.
    fn sync(&mut self, store: &Store) {
        let recording = store.recording();
        if recording != self.indicated {
            (self.env.indicator)(recording);
            self.indicated = recording;
            if recording {
                (self.env.on_quit)(store);
            }
        }
        let away = self.away.any();
        let want = self.front.filter(|_| recording && self.config.titles && !away);
        if want != self.watched {
            match want {
                Some(pid) => (self.env.follow)(pid),
                None => (self.env.unfollow)(),
            }
            self.watched = want;
        }
    }

    /// Handle `event` while recording.
    fn record(&mut self, event: Event, store: &Store) {
        let now = (self.env.now)();
        let away = self.away.any();
        match event {
            Event::Started => {
                // The time down is not counted: the span ended at its last event.
                self.settle(store);
                self.begin(store, now);
            }
            Event::Sleep | Event::Locked => {
                self.away.asleep |= event == Event::Sleep;
                self.away.locked |= event == Event::Locked;
                self.apply(Input::Pause, store, now);
            }
            // The time asleep is not counted; a locked screen waits for `Unlocked`.
            Event::Wake => {
                self.away.asleep = false;
                if !self.away.locked {
                    self.settle(store);
                    self.begin(store, now);
                }
            }
            Event::Unlocked if self.away.locked => {
                self.away.locked = false;
                if !self.away.asleep {
                    self.settle(store);
                    self.begin(store, now);
                }
            }
            Event::ModuleChanged { module: "activity" } if wire::take_stop() => {
                self.set_recording(false, store);
            }
            Event::AppActivated { pid } if !away => match self.focus(pid) {
                Some(input) => self.apply(input, store, now),
                None => self.touch(store, now),
            },
            Event::WindowChanged { pid } if !away && self.config.titles && self.front == Some(pid) => {
                match self.focus(pid) {
                    Some(input) => self.apply(input, store, now),
                    None => self.touch(store, now),
                }
            }
            // The open span ends; the front app's next span carries the new task.
            Event::TaskChanged { .. } if !away => match self.front.and_then(|pid| self.focus(pid)) {
                Some(input) => self.apply(input, store, now),
                None => self.touch(store, now),
            },
            Event::Idle { secs } => self.apply(Input::Idle { secs }, store, now),
            Event::Active => self.apply(Input::Active, store, now),
            _ => self.touch(store, now),
        }
    }

    /// Advance the open span to `now`.
    fn touch(&self, store: &Store, now: i64) {
        if let Some(id) = self.open {
            store.span_end(id, now);
        }
    }

    /// Turn recording on or off; the status line.
    fn set_recording(&mut self, on: bool, store: &Store) -> String {
        let now = (self.env.now)();
        if store.recording() != on {
            if on {
                store.set_recording(true);
                self.settle(store);
                self.begin(store, now);
            } else {
                self.apply(Input::Pause, store, now);
                store.set_recording(false);
                self.settle(store);
            }
        }
        self.sync(store);
        format!("Activity recording {}", if on { "on" } else { "off" })
    }

    fn report(&self, range: &'static str, store: &Store) -> Report {
        let (from, now, offset, spans) = self.clipped(range, store);
        Report::new(range, (from, now, offset), &spans, &self.config, store.recording())
    }

    /// Spans of `range` ("today" or "week") cut to it, with its start, now and the UTC offset.
    fn clipped(&self, range: &str, store: &Store) -> (i64, i64, i32, Vec<Span<Subject>>) {
        let now = (self.env.now)();
        let offset = (self.env.utc_offset)(now);
        let from = parse_since(range, now, offset).unwrap_or(now);
        self.touch(store, now);
        (from, now, offset, clip(store.spans(from, now), from, now))
    }

    /// `today|week [--by task]`.
    fn totals(&self, range: &'static str, args: &[String], cx: &Cx) -> Result<String, String> {
        let json = |v: Result<String, serde_json::Error>| v.map_err(|e| format!("activity: {e}"));
        match args {
            [] => {
                let mut r = self.report(range, cx.store);
                if self.hide_titles(cx) {
                    r.top_titles.clear();
                }
                if cx.json { json(serde_json::to_string(&r)) } else { Ok(r.text()) }
            }
            [by, what] if by == "--by" && what == "task" => {
                let (from, now, _, spans) = self.clipped(range, cx.store);
                let r = TaskReport::new(range, (from, now), &spans, cx.store.recording());
                if cx.json { json(serde_json::to_string(&r)) } else { Ok(r.text()) }
            }
            _ => Err(format!("activity: usage: activity {range} [--by task]")),
        }
    }

    fn status(&self, store: &Store) -> String {
        let on = if store.recording() { "on" } else { "off" };
        let titles = match (self.config.titles, (self.env.trusted)()) {
            (false, _) => "off",
            (true, true) => "on",
            (true, false) => "no Accessibility permission",
        };
        let open = match self.clock.open() {
            Some((s, start)) => {
                let since = report::local_time(start, (self.env.utc_offset)(start));
                format!("{} since {}", s.name, &since[11..])
            }
            None if self.away.any() => "screen locked or asleep".into(),
            None if self.excluded && self.clock.is_paused() => "excluded app in front".into(),
            None if self.clock.is_idle() => "idle".into(),
            None => "none".into(),
        };
        format!("recording: {on}\ntitles: {titles}\nopen span: {open}")
    }

    fn spans(&self, args: &[String], cx: &Cx) -> Result<String, String> {
        let now = (self.env.now)();
        let offset = (self.env.utc_offset)(now);
        let since = match args {
            [] => "today",
            [flag, since] if flag == "--since" => since,
            _ => return Err("activity: usage: activity spans [--since today|week|YYYY-MM-DD]".into()),
        };
        let from = parse_since(since, now, offset)
            .ok_or_else(|| format!("activity: bad date \"{since}\" (today, week or YYYY-MM-DD)"))?;
        self.touch(cx.store, now);
        let spans = cx.store.spans(from, now + 1);
        let mut list = span_list(&spans, &self.config);
        if self.hide_titles(cx) {
            for s in &mut list {
                s.title = None;
            }
        }
        if cx.json {
            return serde_json::to_string(&list).map_err(|e| format!("activity: {e}"));
        }
        Ok(span_text(&list, offset))
    }

    fn forget(&mut self, args: &[String], store: &Store) -> Result<String, String> {
        let (what, yes) = match args {
            [what @ .., yes] if yes == "--yes" => (what, true),
            what => (what, false),
        };
        let now = (self.env.now)();
        let n = match what {
            [w] if w == "today" && yes => {
                store.forget_since(parse_since("today", now, (self.env.utc_offset)(now)).unwrap_or(now))
            }
            [w] if w == "all" && yes => store.forget_all(),
            [w, app] if w == "app" && yes => store.forget_app(app),
            [w] | [w, _] if ["today", "all", "app"].contains(&w.as_str()) => {
                return Err("activity: forget deletes spans; add --yes".into());
            }
            _ => return Err("activity: usage: activity forget today|all|app <bundle id> --yes".into()),
        };
        // The open span may be gone: start over from the frontmost app.
        let recording = store.recording();
        self.settle(store);
        if recording {
            self.begin(store, now);
        }
        self.sync(store);
        Ok(format!("Deleted {n} spans"))
    }

    fn today_items(&self, store: &Store) -> Vec<Item> {
        let r = self.report("today", store);
        let only_uncategorized =
            r.by_category.iter().all(|t| t.name == report::UNCATEGORIZED);
        let categories = if only_uncategorized { &[][..] } else { &r.by_category[..] };
        [("category", categories), ("project", &r.by_project[..]), ("app", &r.by_app[..])]
            .into_iter()
            .flat_map(|(kind, totals)| {
                totals.iter().map(move |t| Item {
                    subtitle: format!("{}{}", kind[..1].to_uppercase(), &kind[1..]),
                    accessory: format!(
                        "{}  ·  {:.0}%",
                        report::duration(t.secs),
                        t.share * 100.0
                    ),
                    ..Item::new(
                        ItemId::new("activity", format!("row/{kind}/{}", t.name)),
                        t.name.clone(),
                        "Show",
                        Icon::Symbol("clock"),
                    )
                })
            })
            .collect()
    }
}

impl Module for Activity {
    fn id(&self) -> &'static str {
        "activity"
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn configure(&mut self, table: &crate::config::Section) -> Result<(), String> {
        self.config = Config::parse(table)?;
        Ok(())
    }

    fn items(&mut self, cx: &mut Cx) -> Vec<Item> {
        let record = if cx.store.recording() {
            ("Stop Activity Recording", "stop.circle")
        } else {
            ("Start Activity Recording", "record.circle")
        };
        [("record", record.0, record.1, "Run Command"), ("today", "Activity Today", "clock", "Open Command")]
            .into_iter()
            .map(|(key, title, symbol, verb)| Item {
                subtitle: "Activity".into(),
                accessory: "Command".into(),
                keywords: vec!["time tracking screen".into()],
                ..Item::new(ItemId::new("activity", key), title, verb, Icon::Symbol(symbol))
            })
            .chain([self.remote_item(cx.store)])
            .collect()
    }

    /// View `today`: time per category, project and app since local midnight.
    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        (view == "today").then(|| ListView {
            placeholder: "Search today's activity…".into(),
            footer: "Activity Today  ·  esc to go back".into(),
            ..ListView::new("activity", view)
        })
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        let items = self.today_items(cx.store);
        let count = items.len() as f64;
        let order: Vec<String> = items.iter().map(|i| i.id.to_string()).collect();
        view.items = cx.ranker.rank(cx.query, items, |i| {
            count - order.iter().position(|id| id == i.id.as_str()).unwrap_or(0) as f64
        });
        view.empty = if cx.store.recording() {
            "Nothing recorded today yet"
        } else {
            "Activity recording is off"
        }
        .into();
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        match id.key() {
            "record" => Outcome::Stay(Some(self.set_recording(!cx.store.recording(), cx.store))),
            "today" => Outcome::Push(ListView::new("activity", "today")),
            "remote" => Outcome::Stay(Some(self.toggle_remote(cx.store))),
            _ => Outcome::Stay(None),
        }
    }

    fn on_event(&mut self, event: Event, cx: &mut Cx) -> bool {
        let store = cx.store;
        if let Event::TaskChanged { task } = event {
            self.task = task;
        }
        if event == Event::Started {
            self.settle(store);
            self.away = Away::default();
        }
        let recording = store.recording();
        if recording {
            self.record(event, store);
        }
        self.sync(store);
        recording
    }

    fn hotkeys(&self) -> Vec<Binding> {
        self.config
            .hotkey
            .iter()
            .map(|spec| Binding { spec: spec.clone(), key: Ok("toggle".into()) })
            .collect()
    }

    fn hotkey(&mut self, key: &str, cx: &mut Cx) -> Option<ListView> {
        if key == "toggle" {
            self.set_recording(!cx.store.recording(), cx.store);
        }
        None
    }

    fn verbs(&self) -> &'static str {
        "activity on|off|status | activity today|week [--by task] | activity spans [--since <date>] | activity forget today|all|app <id> --yes | activity remote allow [<min>|always]|deny|status"
    }

    /// `--json` (`cx.json`) makes `today`, `week` and `spans` answer with JSON; spans carry
    /// their task id. Remote callers (`cx.remote`) pass `remote::gate` first.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        let read = if cx.remote { remote::gate(args, cx.store, (self.env.now)())? } else { false };
        let reply = match args {
            [v] if v == "on" => Ok(self.set_recording(true, cx.store)),
            [v] if v == "off" => Ok(self.set_recording(false, cx.store)),
            [v] if v == "status" => Ok(self.status(cx.store)),
            [v, rest @ ..] if v == "today" || v == "week" => {
                self.totals(if v == "today" { "today" } else { "week" }, rest, cx)
            }
            [v, rest @ ..] if v == "spans" => self.spans(rest, cx),
            [v, rest @ ..] if v == "forget" => self.forget(rest, cx.store),
            [v, rest @ ..] if v == "remote" => self.remote(rest, cx),
            _ => Err(unknown_verb("activity", args)),
        };
        if read && reply.is_ok() {
            cx.store.set_remote_last((self.env.now)());
        }
        reply
    }
}
