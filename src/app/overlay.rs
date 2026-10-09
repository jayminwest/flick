//! The screens over root search and module lists: the action menu (⌘K), confirmations and
//! forms. Their keys, and how they open and close.

use super::screen::{self, Back, ConfirmStep, Screen};
use super::{State, VISIBLE_ROWS};
use crate::core::Tab;
use crate::platform::panel::Key;
use crate::ui;

impl State {
    /// Take the screen on the panel, as the user left it, to come back to later.
    pub(super) fn back(&mut self) -> Back {
        Back {
            screen: Box::new(std::mem::replace(&mut self.screen, Screen::Root)),
            query: ui::query(),
            selected: self.selected,
            scroll: self.scroll,
        }
    }

    /// Return to `back` with its search text, refreshed, and its selection where it was.
    pub(super) fn restore(&mut self, back: Back) {
        self.status = None;
        self.show(*back.screen, &back.query);
        if back.selected < self.results.len() {
            self.selected = back.selected;
            self.scroll = back.scroll.min(back.selected);
            self.render();
        }
    }

    /// ⌘K on a list: the selected item's action menu. False when it has none.
    pub(super) fn open_actions(&mut self) -> bool {
        let Some(target) = self.results.get(self.selected).cloned() else { return false };
        let query = ui::query();
        let actions = self.registry.actions(&target.id, &mut self.env.cx(&query));
        if actions.is_empty() {
            return false;
        }
        let back = self.back();
        self.status = None;
        self.show(Screen::Actions { target, actions, back }, "");
        true
    }

    /// Tab on a list: what the selected item's `tab` asks for. `Activate` is Enter (a
    /// quicklink's argument, like Raycast); `Act(key)` runs that action as the ⌘K menu would.
    pub(super) fn tab(&mut self) {
        let Some(item) = self.results.get(self.selected).cloned() else { return };
        match item.tab {
            Tab::None => {}
            Tab::Activate => self.activate(),
            Tab::Act(key) => {
                let query = ui::query();
                let outcome = self.registry.act(&item.id, key, &mut self.env.cx(&query));
                self.apply(outcome, None);
            }
        }
    }

    /// A key in the action menu. Enter runs the selected action; Escape, ⌘K and Backspace
    /// in an empty field go back.
    pub(super) fn actions_key(&mut self, key: Key) -> bool {
        match key {
            Key::Up => self.move_selection(-1),
            Key::Down => self.move_selection(1),
            Key::Enter => self.run_action(),
            Key::Escape | Key::CmdK => self.close_actions(),
            Key::Backspace if ui::query().is_empty() => self.close_actions(),
            Key::Tab | Key::BackTab => {}
            _ => return false,
        }
        true
    }

    fn close_actions(&mut self) {
        if let Screen::Actions { back, .. } = std::mem::replace(&mut self.screen, Screen::Root) {
            self.restore(back);
        }
    }

    /// Run the selected action on the menu's item, through the item's module. The item's
    /// module sees the search text of the screen the menu came from.
    fn run_action(&mut self) {
        let Some(key) = self.results.get(self.selected).map(|i| i.id.key().to_owned()) else {
            return;
        };
        let Screen::Actions { target, back, .. } =
            std::mem::replace(&mut self.screen, Screen::Root)
        else {
            return;
        };
        let outcome = self.registry.act(&target.id, &key, &mut self.env.cx(&back.query));
        self.apply(outcome, Some(back));
    }

    /// A key on a confirmation. It takes every key: its rows are read-only.
    pub(super) fn confirm_key(&mut self, key: Key) -> bool {
        let Screen::Confirm { confirm, .. } = &self.screen else { return false };
        match screen::confirm_step(confirm.destructive, key) {
            ConfirmStep::Confirm => {
                let Screen::Confirm { confirm, back } =
                    std::mem::replace(&mut self.screen, Screen::Root)
                else {
                    return false;
                };
                let outcome = self.registry.confirmed(&confirm, &mut self.env.cx(&back.query));
                self.apply(outcome, Some(back));
            }
            ConfirmStep::Hint => {
                let hint = format!("Press ⌘↵ to {}", confirm.label);
                self.set_status(hint);
            }
            ConfirmStep::Cancel => {
                if let Screen::Confirm { back, .. } =
                    std::mem::replace(&mut self.screen, Screen::Root)
                {
                    self.restore(back);
                }
            }
            ConfirmStep::Scroll(delta) => {
                self.scroll =
                    screen::scrolled(self.scroll, delta, self.results.len(), VISIBLE_ROWS);
                self.render();
            }
            ConfirmStep::Ignore => {}
        }
        true
    }

    /// Show `module`'s form `name`. When it has none, return to `back` (if any) and say so.
    pub(super) fn open_form(&mut self, module: &'static str, name: &str, back: Option<Back>) {
        if let Some(form) = self.registry.form(module, name, &mut self.env.cx("")) {
            self.status = None;
            self.show(Screen::Form(form), "");
            return;
        }
        if let Some(back) = back {
            self.restore(back);
        }
        self.set_status(format!("Can't open {module} form \"{name}\""));
    }

    /// A key in a form. Tab and Shift-Tab move focus, Enter and ⌘↵ submit, Escape returns
    /// to root search. Other keys edit the focused field.
    pub(super) fn form_key(&mut self, key: Key) -> bool {
        let Screen::Form(form) = &mut self.screen else { return false };
        // A click may have moved focus behind the controller's back.
        if let Some(index) = ui::focused_field() {
            form.focused = index;
        }
        match key {
            Key::Tab => form.focus_next(),
            Key::BackTab => form.focus_prev(),
            Key::Enter | Key::CmdEnter => {
                self.submit();
                return true;
            }
            Key::Escape => {
                self.enter(None);
                return true;
            }
            _ => return false,
        }
        self.render();
        true
    }

    /// Submit the form: a blank required field is an inline error, without asking the
    /// module. On `Ok` back to root search with the module's status; on `Err` the form
    /// stays with the error.
    fn submit(&mut self) {
        let Screen::Form(form) = &self.screen else { return };
        let result = match screen::missing(form) {
            Some(error) => Err(error),
            None => self.registry.submit(form, &mut self.env.cx("")),
        };
        match result {
            Ok(status) => {
                self.enter(None);
                self.set_status(status);
            }
            Err(error) => {
                if let Screen::Form(form) = &mut self.screen {
                    form.error = Some(error);
                }
                self.render();
            }
        }
    }
}
