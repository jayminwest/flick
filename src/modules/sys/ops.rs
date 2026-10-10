//! The module's fleet actions (flick-4a4c): the cmd+K menus, Open Fleet Window (the root
//! item `Fleet`, flick-a2ed), Screen Sharing and Open Dash,
//! Tail Log (view `log`), Restart behind a destructive confirm, and the `sys tail` and
//! `sys restart` verbs. `act.rs` decides what runs; `jobs.rs` runs it on threads.

use super::act::{self, Target};
use super::jobs;
use super::menu::{self, Press};
use super::settings::Machine;
use super::views::{self, Key};
use super::{ID, Sys};
use crate::core::later;
use crate::core::{Action, Cx, Icon, ItemId, ListView, Outcome};
use crate::platform::hud;

/// `$HOME`, for a `~/` log on this Mac.
fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// `[service]` or `[machine, service]`.
fn names<'a>(args: &'a [String], verb: &str) -> Result<(Option<&'a str>, &'a str), String> {
    match args {
        [s] => Ok((None, s)),
        [m, s] => Ok((Some(m), s)),
        _ => Err(format!("sys: usage: sys {verb} [<machine>] <service>")),
    }
}

impl Sys {
    /// `service` of `machine` (none: this Mac) as this Mac's config defines it.
    fn resolve(&self, machine: Option<&str>, service: &str) -> Result<Target, String> {
        let st = self.shared.lock();
        act::resolve(&st.fleet, &st.services, machine, service)
    }

    /// The service a row names: a fleet `service/` row, or a `check/` row of the machine
    /// view. `None` for other rows and services this Mac's config does not define.
    fn target_of(&self, key: &str) -> Option<Target> {
        let st = self.shared.lock();
        let (machine, service) = match Key::parse(key, &st.fleet)? {
            Key::Service { machine, service } => (machine, service),
            Key::Check(service) => (self.detail.as_deref()?, service),
            _ => return None,
        };
        act::resolve(&st.fleet, &st.services, Some(machine), service).ok()
    }

    /// The machine a row names: a fleet `machine/` row, or the machine view's head.
    fn machine_of(&self, key: &str) -> Option<Machine> {
        let st = self.shared.lock();
        let name = match Key::parse(key, &st.fleet)? {
            Key::Machine(m) => m,
            Key::Head => self.detail.as_deref()?,
            _ => return None,
        };
        st.fleet.slots.iter().find(|s| s.machine.name == name).map(|s| s.machine.clone())
    }

    /// Cheap (no I/O): the launcher asks on every render.
    pub(super) fn menu(&self, id: &ItemId) -> Vec<Action> {
        if id.key() == "fleet" {
            return vec![Action::new("window", "Open Fleet Window", Icon::Symbol("macwindow"))];
        }
        if let Some(m) = self.machine_of(id.key()) {
            return act::machine_menu(&m);
        }
        self.target_of(id.key()).map(|t| act::service_menu(&t)).unwrap_or_default()
    }

    pub(super) fn run_action(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        let gone = || Outcome::Stay(Some("sys: that row has no such action now".into()));
        match key {
            "vnc" | "dash" => {
                let m = self.machine_of(id.key());
                let Some(url) = m.and_then(|m| if key == "vnc" { m.vnc } else { m.dash }) else { return gone() };
                cx.hide();
                (self.hooks.open)(&url);
                Outcome::Hide
            }
            "window" => {
                cx.hide();
                self.window_show();
                Outcome::Hide
            }
            "tail" => {
                let Some(target) = self.target_of(id.key()) else { return gone() };
                match self.tail(&target, false) {
                    Ok(()) => Outcome::Push(ListView::new(ID, "log")),
                    Err(e) => Outcome::Stay(Some(e)),
                }
            }
            "restart" => {
                let Some(target) = self.target_of(id.key()) else { return gone() };
                match target.restart((self.hooks.uid)()) {
                    Ok(run) => Outcome::Confirm(target.confirm(&run)),
                    Err(e) => Outcome::Stay(Some(e)),
                }
            }
            _ => Outcome::Stay(None),
        }
    }

    /// A press on a fleet window action card (`menu.rs`, flick-1e00): open, tail, or
    /// restart on the second press (the card's Run), re-resolved from config. Why it did
    /// nothing goes to the window's notice line.
    pub(super) fn window_press(&mut self, card: &str, action: &str) {
        let Some(press) = menu::press(card, action, hud::CANCEL) else { return };
        let asked = Some((card.to_string(), action.to_string()));
        let confirmed = matches!(press, Press::Restart { .. }) && self.win.confirm == asked;
        self.win.confirm = None;
        let said = match press {
            Press::Cancel => Ok(()),
            Press::Open { machine, vnc } => {
                let m = self.shared.lock().fleet.slots.iter().find(|s| s.machine.name == machine).map(|s| s.machine.clone());
                match m.and_then(|m| if vnc { m.vnc } else { m.dash }) {
                    Some(url) => {
                        (self.hooks.open)(&url);
                        Ok(())
                    }
                    None => Err(format!("sys: {machine} has no such action now")),
                }
            }
            Press::Tail { machine, service } => {
                let started = self.resolve(Some(machine), service).and_then(|t| self.tail(&t, false));
                self.win.tail = started.is_ok();
                started
            }
            Press::Restart { .. } if !confirmed => {
                self.win.confirm = asked;
                Ok(())
            }
            Press::Restart { machine, service } => self.resolve(Some(machine), service).and_then(|t| {
                jobs::restart(&self.shared, &t, t.restart((self.hooks.uid)())?, self.hooks, None)
            }),
        };
        self.win.said = said.err();
    }

    /// The restart the user confirmed, resolved again from config.
    pub(super) fn confirm_restart(&mut self, token: &str) -> Outcome {
        let Some((machine, service)) = act::parse_token(token) else { return Outcome::Stay(None) };
        let started = self.resolve(Some(machine), service).and_then(|t| {
            jobs::restart(&self.shared, &t, t.restart((self.hooks.uid)())?, self.hooks, None)?;
            Ok(format!("Restarting {}…", t.what()))
        });
        Outcome::Stay(Some(started.unwrap_or_else(|e| e)))
    }

    /// Start a tail of `target`; it shows in view `log`. With `cli`, the request's answer
    /// is the lines (`core::later`), asked for only once the tail can run.
    fn tail(&self, target: &Target, cli: bool) -> Result<(), String> {
        let runs = (target.tail(&home())?, target.sudo_tail(&home()));
        jobs::tail(&self.shared, target, runs, self.hooks, cli.then(later::answer_later))
    }

    /// Enter on the log row: tail the same service again.
    pub(super) fn tail_again(&self) -> Outcome {
        let last = self.shared.lock().tail.as_ref().map(|t| (t.machine.clone(), t.service.clone()));
        let Some((machine, service)) = last else { return Outcome::Stay(None) };
        let machine = (machine != act::HERE).then_some(machine.as_str());
        let started = self.resolve(machine, &service).and_then(|t| self.tail(&t, false));
        Outcome::Stay(started.err())
    }

    /// View `log`: the last tail.
    pub(super) fn show_log() -> ListView {
        ListView { placeholder: "Log".into(), ..ListView::new(ID, "log") }
    }

    pub(super) fn fill_log(&self, view: &mut ListView) {
        let now = (self.hooks.now)();
        let st = self.shared.lock();
        (view.items, view.footer, view.text) = views::log_view(st.tail.as_ref(), now);
    }

    /// `sys tail [<machine>] <service>`: the log's last lines, answered later.
    pub(super) fn tail_command(&self, args: &[String]) -> Result<String, String> {
        let (machine, service) = names(args, "tail")?;
        let target = self.resolve(machine, service)?;
        self.tail(&target, true)?;
        Ok(format!("Tailing {}", target.what()))
    }

    /// `sys restart [<machine>] <service> [--yes]`: without `--yes` only the command it
    /// would run; with it the restart, answered later.
    pub(super) fn restart_command(&self, args: &[String]) -> Result<String, String> {
        let (yes, args) = match args {
            [rest @ .., y] if y == "--yes" => (true, rest),
            _ => (false, args),
        };
        let (machine, service) = names(args, "restart")?;
        let target = self.resolve(machine, service)?;
        let run = target.restart((self.hooks.uid)())?;
        if !yes {
            return Ok(format!("would run: {}\nadd --yes to restart {}", run.shown, target.what()));
        }
        jobs::restart(&self.shared, &target, run, self.hooks, Some(later::answer_later()))?;
        Ok(format!("Restarting {}", target.what()))
    }
}
