//! Card presses and dismissals (plan flick-7da1, step 9). The HUD's `on_press` and
//! `on_dismiss` handlers (`wire.rs`) only queue a `Note` and post `ModuleChanged`; the module
//! drains the queue here, on the main thread, along with finished KOTA sends (`run.rs`).
//!
//! - A press of a reply action (no `do`, or `"reply": true`) runs `[message] action_command`
//!   on a worker; the card shows "Sent to KOTA…" with its actions off until KOTA re-posts it
//!   (a post of the same id clears the press state). Exit 2 or any failure puts an error line
//!   on the card and turns the actions back on. A card still pending after
//!   `pending_timeout_secs` turns to "No update from KOTA".
//! - Presses on a pending card are ignored (no double send).
//! - `dismiss` closes the card. A `shell` action asks first: its first press sets the
//!   confirm step, Cancel (`hud::CANCEL`) clears it. Running it, and the other local actions,
//!   come with flick-e244.
//! - A dismissal by the user or a timeout marks the card dismissed (plan risk 10) and drops
//!   its press state.
//!
//! Press state is in memory only.

use std::collections::HashMap;

use super::run::{Done, Exit, Job};
use super::store::Messages;
use super::{Inbox, card};
use crate::core::Cx;
use crate::core::card::{Card, Do, Kind, State};
use crate::platform::hud::{CANCEL, CardUi};

/// The error line of a local action this step does not run yet.
pub const LOCAL_LATER: &str = "Local actions arrive in flick-e244";
/// The error line of a press with no `action_command`.
pub const NO_COMMAND: &str = "Set [message] action_command to send presses to KOTA";
/// The error line of a pending card KOTA did not update in time.
pub const NO_UPDATE: &str = "No update from KOTA";

/// What the HUD handlers queue for the module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// An action button: card id, action id, values JSON.
    Press { card: String, action: String, values: String },
    /// The user or a timeout dismissed the card with this id.
    Dismissed(String),
}

/// Where a reply press is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// `action_command` runs for press number `press`.
    Sending { press: u64 },
    /// It exited 0 at `since` (seconds); KOTA has not re-posted the card yet.
    Waiting { since: i64 },
}

/// The module's state of one card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ui {
    pub phase: Phase,
    pub error: Option<String>,
    /// The shell action waiting for confirmation.
    pub confirm: Option<String>,
    /// Close the card once the send succeeds (a `dismiss` that also replies).
    pub close_on_sent: bool,
}

impl Ui {
    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }

    pub fn card_ui(&self) -> CardUi<'_> {
        CardUi { pending: self.busy(), error: self.error.as_deref(), confirm: self.confirm.as_deref() }
    }
}

/// Press state by card id.
pub type Uis = HashMap<String, Ui>;

impl Inbox {
    /// Handle everything queued since the last call, then the pending watchdog.
    pub(super) fn drain(&mut self, cx: &Cx) {
        for note in (self.env.take_notes)() {
            match note {
                Note::Press { card, action, values } => self.press(&card, &action, values, cx),
                Note::Dismissed(id) => self.forget(id),
            }
        }
        for done in self.worker.take() {
            self.finished(done, cx);
        }
        self.watchdog(cx);
    }

    /// Show `c` (a card stored in `m`) with its press state.
    pub(super) fn show_card(&self, c: &Card) {
        let ui = self.ui.get(&c.id).cloned().unwrap_or_default();
        let opts = self.card_options(c, &ui);
        (self.env.show_card)(c, &ui.card_ui(), &self.placement(), &opts);
    }

    /// Redraw card `id` if it shows, after its press state changed.
    fn redraw(&self, id: &str, cx: &Cx) {
        let Some(c) = stored(id, cx) else { return };
        let ui = self.ui.get(id).cloned().unwrap_or_default();
        (self.env.update_card)(&c, &ui.card_ui(), &self.card_options(&c, &ui));
    }

    fn set_error(&mut self, id: &str, error: &str, cx: &Cx) {
        let ui = self.ui.entry(id.into()).or_default();
        *ui = Ui { error: Some(error.into()), ..Ui::default() };
        self.redraw(id, cx);
    }

    fn press(&mut self, id: &str, action: &str, values: String, cx: &Cx) {
        let Some(c) = stored(id, cx) else { return };
        let ui = self.ui.entry(id.into()).or_default();
        if ui.busy() || c.state == State::Pending {
            return;
        }
        if action == CANCEL {
            ui.confirm = None;
            return self.redraw(id, cx);
        }
        let Some(a) = c.actions.iter().find(|a| a.id == action && a.enabled()) else { return };
        match &a.kind {
            Kind::Local { run: Do::Shell(_), .. } if ui.confirm.as_deref() == Some(action) => {
                // TODO(flick-e244): run the confirmed command (and reply when asked).
                self.set_error(id, LOCAL_LATER, cx);
            }
            Kind::Local { run: Do::Shell(_), .. } => {
                *ui = Ui { confirm: Some(action.into()), ..Ui::default() };
                self.redraw(id, cx);
            }
            Kind::Local { run: Do::Dismiss, reply: false } => {
                (self.env.dismiss)(id);
                self.forget(id.into());
            }
            Kind::Local { run: Do::Dismiss, reply: true } => self.send(id, action, values, true, cx),
            // TODO(flick-e244): run the local part of a replying local action too.
            Kind::Reply | Kind::Local { reply: true, .. } => self.send(id, action, values, false, cx),
            Kind::Local { .. } | Kind::Disabled { .. } => self.set_error(id, LOCAL_LATER, cx),
        }
    }

    /// Send a press to KOTA on the worker; the card goes pending.
    fn send(&mut self, id: &str, action: &str, values: String, close: bool, cx: &Cx) {
        if self.settings.action_command.is_empty() {
            return self.set_error(id, NO_COMMAND, cx);
        }
        self.presses += 1;
        let press = self.presses;
        self.ui.insert(id.into(), Ui { phase: Phase::Sending { press }, close_on_sent: close, ..Ui::default() });
        self.redraw(id, cx);
        let job = Job::new(&self.settings.action_command, id, action, press, values);
        self.worker.start(job, self.env.exec, self.env.changed);
    }

    /// A send ended. Ignored when the card was re-posted or dismissed since.
    fn finished(&mut self, done: Done, cx: &Cx) {
        let Some(ui) = self.ui.get_mut(&done.card) else { return };
        if ui.phase != (Phase::Sending { press: done.press }) {
            return;
        }
        match done.exit {
            Exit::Sent if ui.close_on_sent => {
                (self.env.dismiss)(&done.card);
                self.forget(done.card);
            }
            Exit::Sent => {
                ui.phase = Phase::Waiting { since: (self.env.now)() };
                let secs = self.settings.pending_timeout_secs;
                if secs > 0 {
                    // One second late, so the clock (whole seconds) has passed the deadline.
                    (self.env.wake_after)(secs + 1);
                }
            }
            Exit::Rejected(why) => self.set_error(&done.card, &format!("KOTA rejected: {why}"), cx),
            Exit::Failed(why) => self.set_error(&done.card, &why, cx),
        }
    }

    /// Turn cards KOTA left pending past `pending_timeout_secs` into errors.
    fn watchdog(&mut self, cx: &Cx) {
        let secs = self.settings.pending_timeout_secs;
        if secs == 0 {
            return;
        }
        let now = (self.env.now)();
        let late: Vec<String> = self
            .ui
            .iter()
            .filter(|(_, ui)| matches!(ui.phase, Phase::Waiting { since } if now - since >= secs as i64))
            .map(|(id, _)| id.clone())
            .collect();
        for id in late {
            self.set_error(&id, NO_UPDATE, cx);
        }
    }
}

/// The card stored under `id`.
fn stored(id: &str, cx: &Cx) -> Option<Card> {
    cx.store.message(id).as_ref().and_then(card::stored)
}
