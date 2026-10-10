//! Module `kota`: KOTA's presence in the menu bar, its pending-card badge and quick ask
//! (plan flick-4354). KOTA is the always-on Claude Code session in a herdr pane on
//! mbp-server. Steps so far: the presence model and `kota status` (flick-3b9b). No
//! items, views, polling or menu bar item yet.
//!
//! Presence: a round's `herdr [--machine <machine>] agent list` and kota-dash `/ok` find
//! the KOTA pane (agent `claude` in `cwd`) and map to thinking, blocked, idle, degraded,
//! down, offline or unknown, debounced (`presence.rs`). `view.rs` renders it as plain
//! data.
//!
//! `flick kota status [--json]` reads the cached presence (no I/O), so it is allowed over
//! the network.
//!
//! Table `[kota]` (`settings.rs`): `machine`, `cwd`, `pane`, `herdr`, `dash`, `ssh`,
//! `kota_ask`, `poll_secs`, `fast_secs`, `status_item`, `notify_down`, `hotkey`.

#[cfg_attr(not(test), expect(dead_code, reason = "flick-4f39 polls"))]
mod presence;
#[cfg_attr(not(test), expect(dead_code, reason = "flick-4f39 runs the children and reads their exits"))]
mod run;
mod settings;
#[cfg(test)]
mod testkit;
mod view;

use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Section;
use crate::core::{Cx, Module, unknown_verb};
use crate::platform::clock;
use presence::Presence;
use settings::Settings;
use view::{Extra, Polling};

pub const ID: &str = "kota";

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[derive(Default)]
pub struct Kota {
    settings: Settings,
    /// The `[kota]` table sets a key: rounds will run on a timer (flick-4f39).
    active: bool,
    presence: Presence,
}

impl Kota {
    fn status(&self, json: bool) -> String {
        let polling = match (self.active, self.settings.poll_secs) {
            (false, _) => Polling::Off,
            (true, 0) => Polling::OnDemand,
            (true, secs) => Polling::Every(secs),
        };
        // flick-316c brings the count of cards waiting on the user.
        let extra = Extra { pending: 0, polling };
        if json {
            return view::status_json(&self.presence, extra).to_string();
        }
        let now = unix_now();
        let offset = clock::utc_offset(i64::try_from(now).unwrap_or(0));
        view::status_text(&self.presence, extra, now, offset)
    }
}

impl Module for Kota {
    fn id(&self) -> &'static str {
        ID
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.settings = table.get::<Settings>()?.check()?;
        self.active = !table.get::<toml::Table>()?.is_empty();
        Ok(())
    }

    /// `--json` (`cx.json`) makes `status` answer with JSON (`view::status_json`).
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [v] if v == "status" => Ok(self.status(cx.json)),
            _ => Err(unknown_verb(ID, args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "kota status"
    }
}

#[cfg(test)]
mod tests;
