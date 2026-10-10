//! Controller: launcher state, the screen on the panel, and routing to modules. Knows no
//! feature: modules come from `crate::modules::lenient`. All calls happen on the main thread.

mod overlay;
mod screen;

use std::cell::RefCell;

use crate::config::{self, Config};
use crate::control;
use crate::core::control::Flags;
use crate::core::store::{self, Store};
use crate::core::{Cx, Event, Item, ListView, Outcome, Ranker, Registry};
use crate::hotkey::{self, Target};
use crate::modules;
use crate::platform::panel::Key;
use crate::platform::{events, status_item};
use crate::root;
use crate::ui::{self, VISIBLE_ROWS, View};
use screen::{Back, Screen};

const ROOT_PLACEHOLDER: &str = "Search for apps and commands…";

/// What modules may use, apart from the registry that holds them.
struct Env {
    config: Config,
    store: Store,
    ranker: Ranker,
}

impl Env {
    fn cx<'a>(&'a mut self, query: &'a str) -> Cx<'a> {
        Cx {
            query,
            store: &self.store,
            ranker: &mut self.ranker,
            hide: ui::hide,
            json: false,
            remote: false,
        }
    }
}

pub struct State {
    screen: Screen,
    registry: Registry,
    env: Env,
    results: Vec<Item>,
    selected: usize,
    scroll: usize,
    status: Option<String>,
    /// The module tables startup skipped, shown as root search's footer until a reload works.
    config_error: Option<String>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

/// The modules `build` sets up from `config` (`modules::lenient`: a bad module table is
/// skipped, the rest apply), and the skipped tables' errors, naming the file.
fn modules_for(
    config: &Config,
    build: impl Fn(&Config) -> (Registry, Vec<String>),
) -> (Registry, Option<String>) {
    let (registry, errors) = build(config);
    if errors.is_empty() {
        return (registry, None);
    }
    let error = format!("{}: {}", config.files(&config::config_path()), errors.join("; "));
    eprintln!("flick: {error}; skipped");
    (registry, Some(error))
}

/// Set up the controller. Returns the launcher hotkey in effect.
pub fn init(config: Config, store: Store) -> String {
    let (registry, config_error) = modules_for(&config, modules::lenient);
    let launcher = config.hotkey.clone();
    ui::set_opacity(config.launcher().opacity);
    let mut state = State {
        screen: Screen::Root,
        registry,
        env: Env { config, store, ranker: Ranker::new() },
        results: vec![],
        selected: 0,
        scroll: 0,
        status: None,
        config_error,
    };
    if let Err(e) = state.registry.migrate(&state.env.store) {
        eprintln!("flick: store migration failed: {e}");
    }
    state.registry.dispatch(Event::Started, &mut state.env.cx(""));
    STATE.with(|s| *s.borrow_mut() = Some(state));
    status_item::set_opener(open_from_menu);
    launcher
}

/// A status item's `Open` row: module `module`'s hotkey `key`, on the main queue's next turn,
/// because `AppKit` is mid-event and may already hold the state (mulch mx-fcbc43).
fn open_from_menu(module: &str, key: &str) {
    let (module, key) = (module.to_owned(), key.to_owned());
    events::on_main(move || hotkey(&module, &key));
}

/// Bind the launcher hotkey and every module's hotkeys from the current config.
pub fn bind_hotkeys() -> Result<(), String> {
    with_state(|s| s.bind()).unwrap_or(Ok(()))
}

/// The launcher hotkey: show root search, or hide it if it's showing.
pub fn toggle() {
    toggle_view(None);
}

/// Module `module`'s hotkey `key`; a view it returns toggles like the launcher hotkey.
pub fn hotkey(module: &str, key: &str) {
    let view = with_state(|s| s.registry.hotkey(module, key, &mut s.env.cx(""))).flatten();
    if let Some(view) = view {
        toggle_view(Some(view));
    }
}

/// Show `request` (root search for `None`), or hide the launcher if it already shows it.
fn toggle_view(request: Option<ListView>) {
    let visible = ui::is_visible();
    if visible && with_state(|s| s.screen.shows(request.as_ref())).unwrap_or(false) {
        ui::hide();
        return;
    }
    with_state(|s| {
        if request.is_none() {
            s.registry.dispatch(Event::LauncherOpened, &mut s.env.cx(""));
            control::publish(Event::LauncherOpened);
        }
        s.status = None;
        s.enter(request);
    });
    if !visible {
        ui::show();
    }
}

/// Show `query` in the root search (for `flick snapshot`).
pub fn set_root_query(query: &str) {
    with_state(|s| {
        s.enter(None);
        ui::set_query(query, ROOT_PLACEHOLDER);
        s.refresh();
    });
}

pub fn query_changed() {
    with_state(|s| {
        s.status = None;
        s.refresh();
    });
}

/// Form field `index` changed: keep the form's value in step with the panel.
pub fn field_changed(index: usize) {
    with_state(|s| {
        if let Screen::Form(form) = &mut s.screen {
            form.set_value(index, ui::field_value(index));
        }
    });
}

/// Keyboard commands from the panel. Returns true when handled.
pub fn command(key: Key) -> bool {
    with_state(|s| match s.screen {
        Screen::Root | Screen::List(_) => s.list_key(key),
        Screen::Form(_) => s.form_key(key),
        Screen::Actions { .. } => s.actions_key(key),
        Screen::Confirm { .. } => s.confirm_key(key),
    })
    .unwrap_or(false)
}

impl State {
    /// A key on root search or a module list.
    fn list_key(&mut self, key: Key) -> bool {
        let list = match &self.screen {
            Screen::List(view) => Some(view),
            _ => None,
        };
        if key == Key::Up {
            self.move_selection(-1);
        } else if key == Key::Down {
            self.move_selection(1);
        } else if key == Key::Enter {
            self.activate();
        } else if key == Key::Tab {
            self.tab();
        } else if key == Key::Escape {
            if list.is_none_or(|v| v.escape_hides) {
                ui::hide();
            } else {
                self.enter(None);
            }
        } else if key == Key::Backspace && list.is_some() && ui::query().is_empty() {
            self.enter(None);
        } else if key == Key::CmdK {
            return self.open_actions();
        } else {
            return false;
        }
        true
    }
}

/// A platform event: every module handles it, and a visible view they report stale
/// refreshes. `DisplaysChanged` re-places a visible panel. One that arrives while the controller is busy (a notification posted inside a
/// call) is posted back to the main queue instead of re-entering.
pub fn on_event(event: Event) {
    let busy = STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else { return true };
        if let Some(s) = s.as_mut() {
            let stale = s.registry.dispatch(event, &mut s.env.cx(""));
            control::publish(event);
            // Root search lists every module's items, so a module's own background change
            // refreshes it too.
            let changed = match &s.screen {
                Screen::List(view) => stale.contains(&view.module),
                Screen::Root => matches!(event, Event::ModuleChanged { .. }),
                _ => false,
            };
            if changed && ui::is_visible() {
                s.refresh_keeping_selection();
            }
            // A screen the open panel was on may have moved or gone.
            if event == Event::DisplaysChanged && ui::is_visible() {
                ui::place();
            }
        }
        false
    });
    if busy {
        events::post(event);
    }
}

/// A control request: `reload`, or `<module> <verb> [args...]` for that module.
/// `flags`: the request's trailing flag words (`Cx::json`, `Cx::remote`).
pub fn control(words: &[String], flags: Flags) -> Result<String, String> {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else { return Err("Flick is busy; try again".into()) };
        let s = s.as_mut().ok_or("Flick is still starting")?;
        match words {
            [verb] if verb == "reload" => s.reload(),
            _ => s
                .registry
                .command(words, &mut Cx { json: flags.json, remote: flags.remote, ..s.env.cx("") }),
        }
    })
}

impl State {
    /// Reload config.toml, reconfigure modules, migrate the store, and rebind hotkeys.
    /// `Err` is a load, configure, or binding error.
    fn reload(&mut self) -> Result<String, String> {
        let config = config::load()?;
        let started = modules::reload(&mut self.registry, &config)
            .map_err(|e| format!("{}: {e}", config.files(&config::config_path())))?;
        ui::set_opacity(config.launcher().opacity);
        self.env.config = config;
        self.config_error = None;
        if let Err(e) = self.registry.migrate(&self.env.store) {
            eprintln!("flick: store migration failed: {e}");
        }
        self.registry.dispatch_to(&started, Event::Started, &mut self.env.cx(""));
        self.registry.dispatch(Event::Reloaded, &mut self.env.cx(""));
        let bound = self.bind();
        self.enter(None);
        bound.map(|()| "Config reloaded".into())
    }

    /// Bind the launcher hotkey, then every module's.
    fn bind(&self) -> Result<(), String> {
        let mut wanted = vec![(self.env.config.hotkey.clone(), Ok(Target::Launcher))];
        for (module, b) in self.registry.hotkeys() {
            wanted.push((b.spec, b.key.map(|key| Target::Module(module, key))));
        }
        hotkey::register(wanted)
    }

    /// Show `view` (a request its module opens), or root search for `None`. False when the
    /// module has no such view; the screen does not change.
    fn enter(&mut self, view: Option<ListView>) -> bool {
        let screen = match view {
            None => Screen::Root,
            Some(request) => match self.registry.open(&request, &mut self.env.cx("")) {
                Some(view) => Screen::List(view),
                None => return false,
            },
        };
        self.show(screen, "");
        true
    }

    /// Put `screen` on the panel with `query` in the search field.
    fn show(&mut self, screen: Screen, query: &str) {
        self.screen = screen;
        let placeholder = match &self.screen {
            Screen::Root => ROOT_PLACEHOLDER,
            Screen::List(view) => view.placeholder.as_str(),
            Screen::Actions { .. } => "Search actions…",
            Screen::Form(_) | Screen::Confirm { .. } => "",
        };
        ui::set_query(query, placeholder);
        self.refresh();
    }

    fn refresh(&mut self) {
        self.fill();
        self.selected = 0;
        self.scroll = 0;
        self.render();
    }

    /// `refresh` for a change the user did not make: the selected item stays selected
    /// while it is still listed.
    fn refresh_keeping_selection(&mut self) {
        let selected = self.results.get(self.selected).map(|i| i.id.clone());
        self.refresh();
        let at = selected.and_then(|id| self.results.iter().position(|i| i.id == id));
        if let Some(at) = at.filter(|&at| at > 0) {
            self.move_selection(at as isize);
        }
    }

    /// Recompute `results` for the screen and the search field.
    fn fill(&mut self) {
        let query = ui::query();
        self.results = match &mut self.screen {
            Screen::Root => {
                let usage = self.env.store.usage();
                let items = self.registry.items(&mut self.env.cx(&query));
                let direct = self.registry.direct(&mut self.env.cx(&query));
                root::rank(&mut self.env.ranker, &query, items, direct, &usage, store::now())
            }
            Screen::List(view) => {
                self.registry.refresh(view, &mut self.env.cx(&query));
                std::mem::take(&mut view.items)
            }
            Screen::Actions { actions, .. } => {
                screen::action_items(&mut self.env.ranker, &query, actions)
            }
            Screen::Confirm { confirm, .. } => screen::confirm_items(confirm),
            Screen::Form(_) => vec![],
        };
    }

    fn render(&mut self) {
        let has_actions = self.screen.is_list()
            && self.results.get(self.selected).is_some_and(|item| {
                let query = ui::query();
                !self.registry.actions(&item.id, &mut self.env.cx(&query)).is_empty()
            });
        let item = self.results.get(self.selected);
        let (title, footer, empty, action) = match &self.screen {
            Screen::Form(form) => {
                ui::render_form(form, self.status.as_deref().unwrap_or("⇥  Next Field"));
                return;
            }
            Screen::Root => {
                let footer = self.config_error.as_deref().unwrap_or("Flick");
                (None, footer, "No Results", screen::list_hint(item, has_actions))
            }
            Screen::List(view) => (
                None,
                view.footer.as_str(),
                view.empty.as_str(),
                screen::list_hint(item, has_actions),
            ),
            Screen::Actions { target, .. } => {
                (None, target.title.as_str(), "No Actions", screen::list_hint(item, false))
            }
            Screen::Confirm { confirm, .. } => {
                (Some(confirm.title.as_str()), "", "", screen::confirm_hint(confirm))
            }
        };
        let rows = !matches!(self.screen, Screen::Confirm { .. });
        let text = if let Screen::List(view) = &self.screen { view.text.as_str() } else { "" };
        ui::render(&View {
            title,
            items: &self.results,
            selected: rows.then_some(self.selected),
            scroll: self.scroll,
            footer: self.status.as_deref().unwrap_or(footer),
            empty,
            action: &action,
            text,
        });
    }

    fn move_selection(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        let last = self.results.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + VISIBLE_ROWS {
            self.scroll = self.selected + 1 - VISIBLE_ROWS;
        }
        self.render();
    }

    fn set_status(&mut self, status: String) {
        self.status = Some(status);
        self.render();
    }

    /// Route the selected item to its module and apply the outcome.
    fn activate(&mut self) {
        let Some(item) = self.results.get(self.selected).cloned() else { return };
        let record = match &self.screen {
            Screen::Root => true,
            Screen::List(view) => view.record_use,
            _ => false,
        };
        if record {
            self.env.store.record_use(item.id.as_str());
        }
        let query = ui::query();
        let outcome = self.registry.activate(&item.id, &mut self.env.cx(&query));
        self.apply(outcome, None);
    }

    /// Apply a module's `outcome`. `back` is the screen an action menu or a confirmation
    /// returns to; `None` when the outcome came from the screen on the panel.
    fn apply(&mut self, outcome: Outcome, back: Option<Back>) {
        match outcome {
            Outcome::Hide => ui::hide(),
            Outcome::Stay(status) => {
                if let Some(back) = back {
                    self.restore(back);
                }
                if let Some(status) = status {
                    self.set_status(status);
                }
            }
            Outcome::Push(view) => {
                if !self.enter(Some(view))
                    && let Some(back) = back
                {
                    self.restore(back);
                }
            }
            Outcome::Form { module, name } => self.open_form(module, &name, back),
            Outcome::Confirm(confirm) => {
                let back = back.unwrap_or_else(|| self.back());
                self.status = None;
                self.show(Screen::Confirm { confirm, back }, "");
            }
            Outcome::ReloadConfig => {
                let status = self.reload().unwrap_or_else(|e| e);
                self.set_status(status);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_module_table_is_skipped_and_reported() {
        let build = |c: &Config| modules::lenient_with_apps(c, vec![]);
        let (_, error) = modules_for(&config::parse("hotkey = \"cmd+K\"").unwrap(), build);
        assert_eq!(error, None);
        let bad = config::parse("hotkey = \"cmd+K\"\n[window]\nenabled = 1").unwrap();
        let (registry, error) = modules_for(&bad, build);
        let error = error.unwrap();
        assert!(error.ends_with(": [window] enabled: expected true or false"), "{error}");
        assert!(registry.into_modules().iter().any(|m| m.id() == "window"));
    }
}
