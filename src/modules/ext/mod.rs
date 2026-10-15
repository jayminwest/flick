//! Module `ext`: external programs add root items, views, actions and forms to the launcher
//! (plan pl-cbf8). Each `[[ext.extensions]]` entry names a program (`settings`); a call
//! runs it once with a JSON request on stdin and reads one JSON reply from stdout
//! (`transport`, `proto`). Calls queue on the extension's own worker thread (`worker`), so
//! the main thread never spawns, reads or waits; answers land in an inbox and
//! `ModuleChanged { module: "ext" }` drains it.
//!
//! Stale while revalidate: root items and views are served at once from the last answer
//! (`items`), and asked for again on the worker, root items at `LauncherOpened` and
//! `Reloaded`, a view each time it opens (`Loading…` until its first answer). A call
//! already queued is not queued again. A failed call is logged, and shown in the footer of
//! the view it was for when that view is on the screen, else posted as a notification,
//! `<name>: <why>`; parse warnings are logged, at most `WARNINGS_LOGGED` per reply.
//! Actions (`act`) and forms (`forms`) build on these answers. `cli` holds the verbs.

mod act;
mod cli;
mod forms;
mod icons;
mod items;
pub mod proto;
mod settings;
mod transport;
mod wire;
mod worker;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::Section;
use crate::core::card::Do;
use crate::core::{Action, Binding, Cx, Event, Form, Item, ItemId, ListView, Module, Outcome};
use act::Asked;
use forms::Phase;
use proto::form::ExtForm;
use proto::item::ExtItem;
use proto::{Reply, Request, View};
use settings::Spec;
use worker::{Answer, Inbox, Job, Worker};

/// Warnings logged per reply; the rest are counted.
pub const WARNINGS_LOGGED: usize = 5;

/// Where answers and failures go, besides the inbox. Tests record them.
#[derive(Clone, Copy)]
struct Hooks {
    job: Job,
    /// One log line.
    log: fn(&str),
    /// A failed call of extension `.0`, `.1` the error.
    notify: fn(&str, &str),
    /// A line of extension `.0` for the user, `.1` (an action's or form's status or error).
    toast: fn(&str, &str),
    /// Run a card `open_url`, `open_app` or `copy`, on the main thread.
    card: fn(&Do) -> Result<(), String>,
    /// The launcher is on the screen.
    visible: fn() -> bool,
    /// Run module `.0`'s hotkey `.1` on a later main-queue turn, like a press of it.
    open: fn(&str, &str),
}

pub struct Ext {
    specs: Vec<Spec>,
    /// Started on an extension's first call.
    workers: Vec<Worker>,
    inbox: Inbox,
    hooks: Hooks,
    /// Root items by extension, from its last good `root` answer.
    root: HashMap<String, Vec<ExtItem>>,
    /// Views by `<name>/<view>`, from their last good `open` answer.
    views: HashMap<String, View>,
    /// Calls queued and not answered yet: `(extension, request)`.
    pending: Vec<(String, Request)>,
    /// The ext view the screen holds (`<name>/<view>`), from its `open` to its `closed`.
    showing: Option<String>,
    /// The showing view's last failed `open`, `(view, error)`, for its footer, until the view
    /// opens again or an answer lands.
    error: Option<(String, String)>,
    /// The showing view's last action line, `(view, line)`, for its footer: `<title>…` while
    /// it runs, then its status or error.
    status: Option<(String, String)>,
    /// The action waiting for the user to confirm it.
    asked: Option<Asked>,
    /// Forms by `<name>/<form>`, from their last good `form` answer.
    forms: HashMap<String, ExtForm>,
    /// The ext form on the screen, `(<name>/<form>, phase)`.
    form: Option<(String, Phase)>,
}

impl Ext {
    /// `socket` is the control socket path the extensions get as `FLICK_SOCKET`; `open`
    /// runs a module's hotkey later (`ext open`).
    pub fn new(socket: fn() -> PathBuf, open: fn(&str, &str)) -> Ext {
        let job = Job { run: transport::call, socket, post: wire::post, flick: wire::flick, raw: transport::raw };
        let (log, notify, toast, visible, card) = (wire::log, wire::notify, wire::toast, wire::visible, wire::card);
        Ext::with(Hooks { job, log, notify, toast, card, visible, open })
    }

    fn with(hooks: Hooks) -> Ext {
        Ext {
            specs: vec![],
            workers: vec![],
            inbox: Inbox::default(),
            hooks,
            root: HashMap::new(),
            views: HashMap::new(),
            pending: vec![],
            showing: None,
            error: None,
            status: None,
            asked: None,
            forms: HashMap::new(),
            form: None,
        }
    }

    fn spec(&self, name: &str) -> Option<&Spec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// `call`, unless the same call is queued already on a live worker.
    fn fetch(&mut self, name: &str, request: Request) {
        let live = self.workers.iter().any(|w| w.spec().name == name && w.alive());
        if !live {
            // Nothing queued will be answered.
            self.pending.retain(|(n, _)| n != name);
        }
        if self.pending.iter().any(|(n, r)| n == name && *r == request) {
            return;
        }
        match self.call(name, request.clone()) {
            Ok(()) => self.pending.push((name.to_string(), request)),
            Err(e) => (self.hooks.log)(&e),
        }
    }

    /// Queue `request` for extension `name`; returns at once. The answer arrives with
    /// `ModuleChanged`.
    pub fn call(&mut self, name: &str, request: Request) -> Result<(), String> {
        self.worker(name)?.send(request);
        Ok(())
    }

    /// Extension `name`'s worker, started (again) when it has none or its thread died (a
    /// panic).
    fn worker(&mut self, name: &str) -> Result<&Worker, String> {
        let Some(spec) = self.specs.iter().find(|s| s.name == name) else {
            return Err(format!("ext: no extension \"{name}\""));
        };
        let i = match self.workers.iter().position(|w| w.spec().name == name) {
            Some(i) if self.workers[i].alive() => i,
            found => {
                let worker = Worker::start(spec.clone(), self.hooks.job, self.inbox.clone())?;
                if let Some(i) = found {
                    self.workers.swap_remove(i);
                }
                self.workers.push(worker);
                self.workers.len() - 1
            }
        };
        Ok(&self.workers[i])
    }

    /// Keep the answers that arrived; true when a root item or view changed. Answers of an
    /// extension no longer configured are dropped.
    fn drain(&mut self) -> bool {
        let mut stale = false;
        for Answer { ext, request, result } in self.inbox.take() {
            if let Some(i) = self.pending.iter().position(|(n, r)| *n == ext && *r == request) {
                self.pending.remove(i);
            }
            if self.spec(&ext).is_none() {
                continue;
            }
            if let Ok(parsed) = &result {
                self.log_warnings(&ext, &request, &parsed.warnings);
            }
            let result = result.map(|parsed| parsed.reply);
            stale |= match request {
                Request::Act { view, .. } => self.acted(&ext, view.as_deref(), result),
                Request::Form { form } => self.loaded(&ext, &form, result),
                Request::Submit { form, .. } => self.saved(&ext, &form, result),
                request => match result {
                    Ok(reply) => self.keep(ext, request, reply),
                    Err(e) => self.failed(&ext, &request, e),
                },
            };
        }
        stale
    }

    /// Cache `reply`, extension `ext`'s answer to `request`; true when it was a root or view
    /// answer.
    fn keep(&mut self, ext: String, request: Request, reply: Reply) -> bool {
        match (request, reply) {
            (Request::Root, Reply::Items(items)) => {
                self.root.insert(ext, items);
                true
            }
            (Request::Open { view }, Reply::View(answer)) => {
                let name = format!("{ext}/{view}");
                self.error.take_if(|(v, _)| *v == name);
                self.views.insert(name, answer);
                true
            }
            // Queries (flick-f4e6); events need no answer.
            _ => false,
        }
    }

    /// Log `error`; show it in the footer of the view it was for when that view is on the
    /// screen (true: the view is stale), else post it.
    fn failed(&mut self, ext: &str, request: &Request, error: String) -> bool {
        (self.hooks.log)(&error);
        let view = match request {
            Request::Open { view } => Some(format!("{ext}/{view}")),
            _ => None,
        };
        let shown = view.filter(|v| self.showing.as_ref() == Some(v) && (self.hooks.visible)());
        if let Some(view) = shown {
            self.error = Some((view, error));
            return true;
        }
        (self.hooks.notify)(ext, &error);
        false
    }

    /// Log the first `WARNINGS_LOGGED` of `warnings`, then how many more there were.
    fn log_warnings(&self, ext: &str, request: &Request, warnings: &[String]) {
        let op = request.op().name();
        for w in warnings.iter().take(WARNINGS_LOGGED) {
            (self.hooks.log)(&format!("{ext}: {op}: {w}"));
        }
        if let Some(more) = warnings.len().checked_sub(WARNINGS_LOGGED).filter(|n| *n > 0) {
            (self.hooks.log)(&format!("{ext}: {op}: {more} more warnings"));
        }
    }
}

impl Module for Ext {
    fn id(&self) -> &'static str {
        "ext"
    }

    /// Workers of extensions that changed or went away stop after their current call; the
    /// cache of those that went away is dropped.
    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let specs = settings::read(table.get()?, dirs::home_dir().as_deref())?;
        self.workers.retain(|w| specs.contains(w.spec()));
        let kept = |name: &str| specs.iter().any(|s| s.name == name);
        self.root.retain(|name, _| kept(name));
        let view_kept = |view: &String| items::split(view).is_some_and(|(name, _)| kept(name));
        self.views.retain(|view, _| view_kept(view));
        self.error.take_if(|(view, _)| !view_kept(view));
        self.status.take_if(|(view, _)| !view_kept(view));
        self.forms.retain(|form, _| view_kept(form));
        self.form.take_if(|(form, _)| !view_kept(form));
        self.asked.take_if(|a| !kept(&a.run.ext));
        self.specs = specs;
        Ok(())
    }

    /// The cached root items, in config order; Flick ranks them.
    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        let cached = self.specs.iter().filter_map(|s| Some((&s.name, self.root.get(&s.name)?)));
        cached.flat_map(|(name, items)| items.iter().map(|e| items::item(name, e))).collect()
    }

    /// View `<name>/<view>`: its cached items (or `Loading…`) at once; it is asked again.
    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        let (name, inner) = items::split(view)?;
        self.spec(name)?;
        (self.error, self.status) = (None, None);
        self.showing = Some(view.to_string());
        self.fetch(name, Request::Open { view: inner.to_string() });
        Some(items::list(view, name, self.views.get(view)))
    }

    fn closed(&mut self, view: &str, _cx: &mut Cx) {
        if self.showing.as_deref() == Some(view) {
            (self.showing, self.error, self.status) = (None, None, None);
        }
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        let name = items::split(&view.name).map_or("", |(name, _)| name).to_string();
        let footer = [&self.error, &self.status].into_iter().flatten().find(|(v, _)| *v == view.name);
        let footer = footer.map(|(_, line)| line.as_str());
        items::fill(view, &name, self.views.get(&view.name), footer, cx);
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        self.act_on(id, None, cx)
    }

    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        self.actions_of(id)
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        self.act_on(id, Some(key), cx)
    }

    fn confirmed(&mut self, token: &str, cx: &mut Cx) -> Outcome {
        self.confirm_asked(token, cx)
    }

    fn form(&mut self, name: &str, _cx: &mut Cx) -> Option<Form> {
        self.open_form(name)
    }

    fn submit(&mut self, form: &Form, _cx: &mut Cx) -> Result<String, String> {
        self.submit_form(form)
    }

    fn refresh_form(&mut self, form: &mut Form, _cx: &mut Cx) -> Option<String> {
        self.fill_form(form)
    }

    fn form_closed(&mut self, form: &Form, _cx: &mut Cx) {
        self.close_form(form);
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        match event {
            Event::ModuleChanged { module: "ext" } => self.drain(),
            Event::LauncherOpened | Event::Reloaded => {
                let names: Vec<String> = self.specs.iter().map(|s| s.name.clone()).collect();
                for name in names {
                    self.fetch(&name, Request::Root);
                }
                false
            }
            _ => false,
        }
    }

    /// Each extension's `hotkeys`, keyed `<name>/<view>`.
    fn hotkeys(&self) -> Vec<Binding> {
        let each = self.specs.iter().flat_map(|s| s.hotkeys.iter().map(move |h| (&s.name, h)));
        each.map(|(name, (view, spec))| Binding { spec: spec.clone(), key: Ok(format!("{name}/{view}")) })
            .collect()
    }

    /// Opens the view the hotkey names.
    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        let (name, _) = items::split(key)?;
        self.spec(name)?;
        Some(ListView::new("ext", key))
    }

    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        self.run_verb(args, cx.json)
    }

    fn verbs(&self) -> &'static str {
        cli::VERBS
    }
}

#[cfg(test)]
mod tests;
