//! The real `io::Hooks` and `item::Ui`: posting `ModuleChanged`, the clock, child
//! processes, this binary's path, the host name, sleeping, the menu bar item, notifications
//! and opening URLs. Tests use `testkit::HOOKS` and `testkit::UI`, so no test runs herdr,
//! curl, ssh or Flick, or touches the menu bar.

use std::sync::{Mutex, PoisonError};
use std::thread;

use super::io::Hooks;
use super::item::{MENU_OPENED, Ui};
use super::view::Entry;
use super::{ID, run, unix_now};
use crate::core::Event;
use crate::platform::status_item::{self, Entry as Row};
use crate::platform::{clock, events, notify, workspace};

pub const HOOKS: Hooks = Hooks {
    post: || events::post(Event::ModuleChanged { module: ID }),
    now: unix_now,
    run: run::run,
    feed: run::feed,
    exe: run::exe,
    host: run::host,
    sleep: thread::sleep,
    utc_offset: clock::utc_offset,
};

/// Menu picks and opens, from the menu's handlers until the next `ModuleChanged`.
static QUEUED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Menu handler (main thread, `AppKit` mid-event): queue the key, ask for the module's turn.
fn queue(key: &str) {
    QUEUED.lock().unwrap_or_else(PoisonError::into_inner).push(key.to_string());
    events::post(Event::ModuleChanged { module: ID });
}

fn show(title: &str, tooltip: &str, menu: &[Entry]) {
    let rows: Vec<Row> = menu
        .iter()
        .map(|e| match e {
            Entry::Info(text) => Row::Info(text),
            Entry::Separator => Row::Separator,
            Entry::Pick { title, key } => Row::Pick { title, key },
            Entry::Open { title, module, key } => Row::Open { title, module, key },
        })
        .collect();
    status_item::show(ID, title, tooltip, &rows, queue);
}

pub const UI: Ui = Ui {
    show,
    hide: || status_item::hide(ID),
    listen: || status_item::on_menu_open(ID, || queue(MENU_OPENED)),
    take: || std::mem::take(&mut *QUEUED.lock().unwrap_or_else(PoisonError::into_inner)),
    // Without Flick.app this does nothing.
    notify: |title, body| {
        let _ = notify::post("kota:down", title, body);
    },
    open_url: workspace::open_url,
};
