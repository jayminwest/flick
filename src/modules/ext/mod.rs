//! Module `ext`: external programs add root items, views, actions and forms to the launcher
//! (plan pl-cbf8). Each `[[ext.extensions]]` entry names a program (`settings`); a call
//! runs it once with a JSON request on stdin and reads one JSON reply from stdout
//! (`transport`, `proto`). Calls queue on the extension's own worker thread (`worker`), so
//! the main thread never spawns, reads or waits; answers land in an inbox and
//! `ModuleChanged { module: "ext" }` drains it. A failed call is logged and posted as a
//! notification, `<name>: <why>`; parse warnings are logged, at most `WARNINGS_LOGGED` per
//! reply. Root items, views, actions and forms build on these answers (flick-9313,
//! flick-b46d).

mod settings;
mod transport;
mod wire;
mod worker;

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "root items (flick-9313) and actions (flick-b46d) read the replies")
)]
pub mod proto;

use std::path::PathBuf;

use crate::config::Section;
use crate::core::{Cx, Event, Module};
use proto::{Parsed, Request};
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
}

pub struct Ext {
    specs: Vec<Spec>,
    /// Started on an extension's first call.
    workers: Vec<Worker>,
    inbox: Inbox,
    hooks: Hooks,
}

impl Ext {
    /// `socket` is the control socket path the extensions get as `FLICK_SOCKET`.
    pub fn new(socket: fn() -> PathBuf) -> Ext {
        let job = Job { run: transport::call, socket, post: wire::post };
        Ext::with(Hooks { job, log: wire::log, notify: wire::notify })
    }

    fn with(hooks: Hooks) -> Ext {
        Ext { specs: vec![], workers: vec![], inbox: Inbox::default(), hooks }
    }

    /// Queue `request` for extension `name`; returns at once. The answer arrives with
    /// `ModuleChanged`.
    #[cfg_attr(not(test), expect(dead_code, reason = "root items (flick-9313) call it first"))]
    pub fn call(&mut self, name: &str, request: Request) -> Result<(), String> {
        let Some(spec) = self.specs.iter().find(|s| s.name == name) else {
            return Err(format!("ext: no extension \"{name}\""));
        };
        if let Some(i) = self.workers.iter().position(|w| w.spec().name == name) {
            if self.workers[i].send(request.clone()) {
                return Ok(());
            }
            // Its thread died (a panic); start another.
            self.workers.swap_remove(i);
        }
        let worker = Worker::start(spec.clone(), self.hooks.job, self.inbox.clone())?;
        worker.send(request);
        self.workers.push(worker);
        Ok(())
    }

    /// The answers that arrived, without the failed calls, which are logged and notified.
    fn drain(&mut self) -> Vec<(String, Request, Parsed)> {
        let mut ok = vec![];
        for Answer { ext, request, result } in self.inbox.take() {
            match result {
                Ok(parsed) => {
                    self.log_warnings(&ext, &request, &parsed.warnings);
                    ok.push((ext, request, parsed));
                }
                Err(e) => {
                    (self.hooks.log)(&e);
                    (self.hooks.notify)(&ext, &e);
                }
            }
        }
        ok
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

    /// Workers of extensions that changed or went away stop after their current call.
    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let specs = settings::read(table.get()?, dirs::home_dir().as_deref())?;
        self.workers.retain(|w| specs.contains(w.spec()));
        self.specs = specs;
        Ok(())
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        // flick-9313 keeps the answers (root items, views); for now they are only checked.
        matches!(event, Event::ModuleChanged { module: "ext" }) && !self.drain().is_empty()
    }
}

#[cfg(test)]
mod tests;
