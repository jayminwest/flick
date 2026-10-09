//! Module `activity`: records which app is in front as time spans, on this machine only,
//! and reports them. Event driven: no timer, no polling. Recording is off until turned on
//! (`flick activity on`, the root item or the hotkey); the flag lives in `activity_state`,
//! not in config. Window titles are never read unless `titles = true`. Excluded apps leave
//! a gap. The open span is a row whose `end` advances on every event while recording, so a
//! crash loses at most the time since the last event.
//! Ids are `activity:record`, `activity:today` and `activity:row/<kind>/<name>` (view rows).

mod report;
mod rules;
mod store;
#[cfg(test)]
mod tests;

use crate::core::track::{Clock, Input, Op, Subject};
use crate::core::{Binding, Cx, Event, Icon, Item, ItemId, ListView, Module, Outcome, unknown_verb};
use crate::platform::{clock, workspace};
use crate::store::Store;
use report::{Report, clip, parse_since, span_list, span_text};
use rules::Config;
use store::{MIGRATIONS, Spans};

/// What the module reads from the system; swapped out in tests.
struct Env {
    now: fn() -> i64,
    utc_offset: fn(i64) -> i32,
    /// Bundle id (if any) and display name of app `pid`.
    identity: fn(i32) -> Option<(Option<String>, String)>,
    frontmost: fn() -> Option<i32>,
    /// Flick's own pid: its activations never open spans.
    own_pid: i32,
}

impl Default for Env {
    fn default() -> Self {
        Env {
            now: crate::store::now,
            utc_offset: clock::utc_offset,
            identity: workspace::app_identity,
            frontmost: workspace::frontmost_pid,
            own_pid: std::process::id() as i32,
        }
    }
}

#[derive(Default)]
pub struct Activity {
    config: Config,
    env: Env,
    clock: Clock,
    /// Row id of the open span.
    open: Option<i64>,
    /// The clock is paused because an excluded app is in front.
    excluded: bool,
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
            return Some(Input::Pause);
        }
        // Titles arrive with Event::WindowChanged (flick-a30b); until then spans are app-level.
        let subject = Subject::new(&app, &name, None, None);
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
        self.clock = Clock::new(self.config.merge_secs);
    }

    /// Start counting the frontmost app.
    fn begin(&mut self, store: &Store, now: i64) {
        if let Some(input) = (self.env.frontmost)().and_then(|pid| self.focus(pid)) {
            self.apply(input, store, now);
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
        format!("Activity recording {}", if on { "on" } else { "off" })
    }

    fn report(&self, range: &'static str, store: &Store) -> Report {
        let now = (self.env.now)();
        let offset = (self.env.utc_offset)(now);
        let from = parse_since(range, now, offset).unwrap_or(now);
        self.touch(store, now);
        let spans = clip(store.spans(from, now), from, now);
        Report::new(range, (from, now, offset), &spans, &self.config, store.recording())
    }

    fn status(&self, store: &Store) -> String {
        let on = if store.recording() { "on" } else { "off" };
        let titles = if self.config.titles { "on" } else { "off" };
        let open = match self.clock.open() {
            Some((s, start)) => {
                let since = report::local_time(start, (self.env.utc_offset)(start));
                format!("{} since {}", s.name, &since[11..])
            }
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
        let list = span_list(&spans, &self.config);
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
            _ => Outcome::Stay(None),
        }
    }

    fn on_event(&mut self, event: Event, cx: &mut Cx) -> bool {
        let store = cx.store;
        if event == Event::Started {
            self.settle(store);
        }
        if !store.recording() {
            return false;
        }
        let now = (self.env.now)();
        match event {
            Event::Started | Event::Wake => {
                // The time asleep (or down) is not counted: the span ends at its last event.
                self.settle(store);
                self.begin(store, now);
            }
            Event::AppActivated { pid } => match self.focus(pid) {
                Some(input) => self.apply(input, store, now),
                None => self.touch(store, now),
            },
            Event::Idle { secs } => self.apply(Input::Idle { secs }, store, now),
            Event::Active => self.apply(Input::Active, store, now),
            _ => self.touch(store, now),
        }
        true
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
        "activity on|off|status | activity today|week | activity spans [--since <date>] | activity forget today|all|app <id> --yes"
    }

    /// `--json` (`cx.json`) makes `today`, `week` and `spans` answer with JSON.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v] if v == "on" => Ok(self.set_recording(true, cx.store)),
            [v] if v == "off" => Ok(self.set_recording(false, cx.store)),
            [v] if v == "status" => Ok(self.status(cx.store)),
            [v] if v == "today" || v == "week" => {
                let r = self.report(if v == "today" { "today" } else { "week" }, cx.store);
                if cx.json {
                    serde_json::to_string(&r).map_err(|e| format!("activity: {e}"))
                } else {
                    Ok(r.text())
                }
            }
            [v, rest @ ..] if v == "spans" => self.spans(rest, cx),
            [v, rest @ ..] if v == "forget" => self.forget(rest, cx.store),
            _ => Err(unknown_verb("activity", args)),
        }
    }
}
