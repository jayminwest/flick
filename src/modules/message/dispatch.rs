//! Card presses and dismissals (plan flick-7da1, steps 9 and 10). The HUD's `on_press` and
//! `on_dismiss` handlers (`wire.rs`) only queue a `Note` and post `ModuleChanged`; the module
//! drains the queue here, on the main thread, along with finished KOTA sends (`run.rs`) and
//! local runs (`local.rs`). `message card press` calls the same `press`.
//!
//! - A press of a reply action (no `do`) runs `[message] action_command` on a worker; the
//!   card shows "Sent to KOTA…" with its actions off until KOTA re-posts it (a post of the
//!   same id clears the press state). Exit 2 or any failure puts an error line on the card
//!   and turns the actions back on. A card still pending after `pending_timeout_secs` turns
//!   to "No update from KOTA".
//! - Presses on a pending or running card are ignored (no double send or run), and so are
//!   presses on a card KOTA posted as `pending` or `done`: only `open` and `error` cards take
//!   presses (`actionable`, docs/cards.md "States and updates").
//! - A local action is checked again at press time (`Do::check` with the origin the card
//!   was posted from, the `remote` column); a refusal is an error line and nothing runs.
//!   `open_url`, `open_app` and `copy` run at once and leave a short line ("Copied"),
//!   `dismiss` closes the card; `script`, `flick` and `shell` run on a worker (`local.rs`),
//!   the card pending with "Running …", then the last output line or an error line.
//! - `shell` asks first, whatever the origin: its first press sets the confirm step (the
//!   card shows the exact command with Cancel and Run), Run (a second press of the same
//!   action) runs it, Cancel (`hud::CANCEL`) clears it.
//! - `"reply": true` beside a `do` runs the local part, then, if it worked, sends the press
//!   to KOTA as a reply action does (`dismiss` closes the card once the send succeeds). A
//!   local part that fails shows its error and sends nothing.
//! - A dismissal by the user or a timeout marks the card dismissed (plan risk 10) and drops
//!   its press state; a run or send that ends afterwards is ignored. A timed-out card still
//!   counts as waiting on the user (`pending.rs`), and any update of it shows (`card.rs`).
//!
//! Press state is in memory only; local output is never stored.

use std::collections::HashMap;
use std::path::PathBuf;

use super::local::{self, Ran};
use super::run::{Done, Exit, Job, NO_THREAD};
use super::store::Messages;
use super::{Inbox, card};
use crate::core::Cx;
use crate::core::card::{Action, Card, Do, Kind, Origin, State};
use crate::platform::hud::{CANCEL, CardUi};

/// The error line of a press with no `action_command`.
pub const NO_COMMAND: &str = "Set [message] action_command to send presses to KOTA";
/// The error line of a pending card KOTA did not update in time.
pub const NO_UPDATE: &str = "No update from KOTA";

/// What the HUD handlers queue for the module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// An action button: card id, action id, values JSON.
    Press { card: String, action: String, values: String },
    /// The user dismissed the card with this id.
    Dismissed(String),
    /// The card with this id timed out: gone from the corner, still waiting on the user
    /// (`pending.rs`).
    Expired(String),
}

/// Where a press is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// A local run (`local.rs`) for press number `press`.
    Running { press: u64 },
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
    /// A muted line: what a local action did, or what runs.
    pub note: Option<String>,
    /// The press (action id, values) to send to KOTA once the local run succeeds.
    pub then_send: Option<(String, String)>,
}

impl Ui {
    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }

    pub fn card_ui(&self) -> CardUi<'_> {
        CardUi {
            pending: self.busy(),
            error: self.error.as_deref(),
            confirm: self.confirm.as_deref(),
            note: self.note.as_deref(),
        }
    }
}

/// Press state by card id.
pub type Uis = HashMap<String, Ui>;

/// What a press did, for `message card press` (the HUD needs no answer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pressed {
    /// Sent to KOTA; the card waits for its update.
    Sent,
    /// A shell action's first press: the card shows this exact command with Cancel and Run.
    Confirm(String),
    /// The confirm step was cancelled.
    Cancelled,
    /// Runs on a worker (this label); the result shows on the card.
    Running(String),
    /// Done at once; the card shows this line.
    Did(String),
    /// The card closed.
    Closed,
}

/// Whether a card in `state` takes presses: `open` and `error` (retry) do; `pending` (KOTA
/// works on it) and `done` show their actions off and ignore them.
pub fn actionable(state: State) -> bool {
    matches!(state, State::Open | State::Error)
}

/// What pressing `a` on a card from `origin` runs: `None` replies to KOTA, else the local
/// action and whether it also replies. `Err` is why it may not run: a disabled action, or
/// a `do` that `Do::check` refuses now (checked again at press time, so a stored card can
/// never run what its origin may not).
pub fn runnable(a: &Action, origin: Origin) -> Result<Option<(&Do, bool)>, String> {
    match &a.kind {
        Kind::Reply => Ok(None),
        Kind::Local { run, reply } => run.check(origin).map(|()| Some((run, *reply))),
        Kind::Disabled { reason, .. } => Err(reason.clone()),
    }
}

impl Inbox {
    /// Handle everything queued since the last call, then the pending watchdog.
    pub(super) fn drain(&mut self, cx: &Cx) {
        for note in (self.env.take_notes)() {
            match note {
                Note::Press { card, action, values } => {
                    // The HUD has no use for the answer; errors already show on the card.
                    let _ = self.press(&card, &action, values, cx);
                }
                Note::Dismissed(id) => self.forget(id),
                Note::Expired(id) => self.expire(id),
            }
        }
        for done in self.worker.take() {
            self.finished(done, cx);
        }
        for done in self.local.take() {
            self.ran(done, cx);
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
        let Some((c, _)) = stored(id, cx) else { return };
        let ui = self.ui.get(id).cloned().unwrap_or_default();
        (self.env.update_card)(&c, &ui.card_ui(), &self.card_options(&c, &ui));
    }

    fn set_error(&mut self, id: &str, error: &str, cx: &Cx) {
        let ui = self.ui.entry(id.into()).or_default();
        *ui = Ui { error: Some(error.into()), ..Ui::default() };
        self.redraw(id, cx);
    }

    /// Show `error` on card `id` and answer it.
    fn fail(&mut self, id: &str, error: String, cx: &Cx) -> Result<Pressed, String> {
        self.set_error(id, &error, cx);
        Err(error)
    }

    /// Press `action` on card `id` with `values` (JSON): from the HUD or `card press`.
    pub(super) fn press(&mut self, id: &str, action: &str, values: String, cx: &Cx) -> Result<Pressed, String> {
        let (c, origin) = stored(id, cx).ok_or(format!("No card {id}"))?;
        if !actionable(c.state) {
            return Err(format!("Card {id} is {}: its actions are off", card::state_name(c.state)));
        }
        let ui = self.ui.entry(id.into()).or_default();
        if ui.busy() {
            return Err(format!("Card {id} is busy: waiting on KOTA or a run"));
        }
        if action == CANCEL {
            ui.confirm = None;
            self.redraw(id, cx);
            return Ok(Pressed::Cancelled);
        }
        let a = c.actions.iter().find(|a| a.id == action).ok_or(format!("No action {action} on card {id}"))?;
        let (run, reply) = match runnable(a, origin) {
            Ok(Some(local)) => local,
            Ok(None) => return self.send(id, action, values, false, cx),
            Err(why) => return self.fail(id, why, cx),
        };
        if let Do::Shell(cmd) = run
            && ui.confirm.as_deref() != Some(action)
        {
            *ui = Ui { confirm: Some(action.into()), ..Ui::default() };
            self.redraw(id, cx);
            return Ok(Pressed::Confirm(cmd.clone()));
        }
        let did = match run {
            Do::Dismiss if reply => return self.send(id, action, values, true, cx),
            Do::Dismiss => {
                (self.env.dismiss)(id);
                self.forget(id.into());
                return Ok(Pressed::Closed);
            }
            Do::OpenUrl(url) => {
                (self.env.open_url)(url);
                "Opened the link".to_string()
            }
            Do::OpenApp(app) => match (self.env.open_app)(app) {
                Ok(()) => format!("Opened {app}"),
                Err(e) => return self.fail(id, e, cx),
            },
            Do::Copy(text) => {
                (self.env.copy)(text);
                "Copied".to_string()
            }
            Do::Script { .. } | Do::Flick(_) | Do::Shell(_) => {
                let then = reply.then(|| (action.to_string(), values));
                return self.spawn(id, run, origin, then, cx);
            }
        };
        if reply {
            return self.send(id, action, values, false, cx);
        }
        self.ui.insert(id.into(), Ui { note: Some(did.clone()), ..Ui::default() });
        self.redraw(id, cx);
        Ok(Pressed::Did(did))
    }

    /// Run a `script`, `flick` or `shell` action of card `id` on the local worker; the card
    /// goes pending with "Running …". `then` is the press to send to KOTA if it works.
    fn spawn(&mut self, id: &str, run: &Do, origin: Origin, then: Option<(String, String)>, cx: &Cx) -> Result<Pressed, String> {
        // A shell command runs `/bin/sh`, not this binary.
        let exe = if matches!(run, Do::Shell(_)) { Ok(PathBuf::new()) } else { (self.env.self_exe)() };
        self.presses += 1;
        let press = self.presses;
        let job = exe.and_then(|exe| local::Job::new(run, &exe, origin, id, press).ok_or(String::from("not a process")));
        let job = match job {
            Ok(job) => job,
            Err(e) => return self.fail(id, e, cx),
        };
        let label = job.label.clone();
        let note = Some(format!("Running {label}…"));
        self.ui.insert(id.into(), Ui { phase: Phase::Running { press }, note, then_send: then, ..Ui::default() });
        self.redraw(id, cx);
        let failed = local::Done { card: id.into(), press, ran: Ran::Failed(NO_THREAD.into()) };
        let exec = self.env.run_local;
        let work = move || local::Done { ran: exec(&job), card: job.card, press: job.press };
        self.local.spawn("flick-card-local", work, failed, self.env.changed);
        Ok(Pressed::Running(label))
    }

    /// Send a press to KOTA on the worker; the card goes pending.
    fn send(&mut self, id: &str, action: &str, values: String, close: bool, cx: &Cx) -> Result<Pressed, String> {
        if self.settings.action_command.is_empty() {
            return self.fail(id, NO_COMMAND.into(), cx);
        }
        self.presses += 1;
        let press = self.presses;
        self.ui.insert(id.into(), Ui { phase: Phase::Sending { press }, close_on_sent: close, ..Ui::default() });
        self.redraw(id, cx);
        let job = Job::new(&self.settings.action_command, id, action, press, values);
        self.worker.start(job, self.env.exec, self.env.changed);
        Ok(Pressed::Sent)
    }

    /// A local run ended: its line, or its error, on the card; then the reply, if any.
    /// Ignored when the card was re-posted or dismissed since.
    fn ran(&mut self, done: local::Done, cx: &Cx) {
        let Some(ui) = self.ui.get_mut(&done.card) else { return };
        if ui.phase != (Phase::Running { press: done.press }) {
            return;
        }
        match (done.ran, ui.then_send.take()) {
            (Ran::Ok(_), Some((action, values))) => {
                // An error (no action_command) shows on the card.
                let _ = self.send(&done.card, &action, values, false, cx);
            }
            (Ran::Ok(line), None) => {
                *ui = Ui { note: Some(line), ..Ui::default() };
                self.redraw(&done.card, cx);
            }
            (Ran::Failed(why), _) => self.set_error(&done.card, &why, cx),
        }
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

/// The card stored under `id` and the origin it was posted from.
fn stored(id: &str, cx: &Cx) -> Option<(Card, Origin)> {
    let m = cx.store.message(id)?;
    card::stored(&m).map(|c| (c, card::origin(m.remote)))
}
