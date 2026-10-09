//! What `herdr` reads from and does on the system: posting `ModuleChanged`, the clock, the
//! launcher panel, the terminal app and notifications (`platform::notify`). `HOOKS` is the
//! real thing; tests use `testkit::HOOKS`, so no test posts a real notification.

use std::sync::Mutex;

use super::io::{Hooks, lock};
use super::unix_now;
use super::views::ID;
use crate::core::Event;
use crate::platform::{events, notify, panel, workspace};

/// Clicked notification ids, from `notify::on_click` until the next `ModuleChanged`.
static CLICKED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `notify::on_click` handler (main thread): queue the id for `on_event`.
fn on_click(id: &str) {
    lock(&CLICKED).push(id.to_string());
    events::post(Event::ModuleChanged { module: ID });
}

pub const HOOKS: Hooks = Hooks {
    post: || events::post(Event::ModuleChanged { module: ID }),
    now: unix_now,
    visible: panel::is_visible,
    front: |name| {
        events::on_main(move || match workspace::find_app(&name) {
            Some(app) => workspace::open_file(&app),
            None => eprintln!("flick: herdr: no app \"{name}\" to bring to the front"),
        });
    },
    is_front: |name| {
        let front = workspace::frontmost_pid().and_then(workspace::app_identity);
        front.is_some_and(|(_, app)| app.eq_ignore_ascii_case(name))
    },
    // Without Flick.app this does nothing; `flick herdr status` says so.
    notify: |id, title, body| {
        let _ = notify::post(id, title, body);
    },
    listen: |ask| {
        let _ = notify::on_click(on_click);
        if ask {
            let _ = notify::request_permission();
        }
    },
    clicks: || std::mem::take(&mut *lock(&CLICKED)),
    notifications: || {
        if !notify::available() {
            return notify::NO_BUNDLE.into();
        }
        match notify::permitted() {
            Some(true) => "on".into(),
            Some(false) => "not permitted (System Settings > Notifications > Flick)".into(),
            None => "permission not answered yet".into(),
        }
    },
};
