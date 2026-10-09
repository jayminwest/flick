//! Controller: launcher state, the view stack, and routing to modules. Knows no feature:
//! modules come from `crate::modules::registry`. All calls happen on the main thread.

use std::cell::RefCell;

use crate::config::{self, Config};
use crate::control;
use crate::core::{Cx, Event, Item, ListView, Outcome, Ranker, Registry};
use crate::hotkey::{self, Target};
use crate::modules;
use crate::platform::events;
use crate::platform::panel::Key;
use crate::root;
use crate::store::{self, Store};
use crate::ui::{self, VISIBLE_ROWS, View};

const ROOT_PLACEHOLDER: &str = "Search for apps and commands…";

/// What modules may use, apart from the registry that holds them.
struct Env {
    config: Config,
    store: Store,
    ranker: Ranker,
}

impl Env {
    fn cx<'a>(&'a mut self, query: &'a str) -> Cx<'a> {
        Cx { query, store: &self.store, ranker: &mut self.ranker, hide: ui::hide }
    }
}

pub struct State {
    /// The module view on screen; `None` is root search.
    view: Option<ListView>,
    registry: Registry,
    env: Env,
    results: Vec<Item>,
    selected: usize,
    scroll: usize,
    status: Option<String>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

/// The modules `config` sets up; an error names the file and the bad table.
fn registry(config: &Config) -> Result<Registry, String> {
    modules::registry(config).map_err(|e| format!("{}: {e}", config::config_path().display()))
}

/// The modules `config` sets up. A bad module table means defaults, like a bad file.
fn modules_for(config: Config) -> (Config, Registry) {
    match registry(&config) {
        Ok(registry) => (config, registry),
        Err(e) => {
            eprintln!("flick: {e}; using defaults");
            let config = Config::default();
            let registry = modules::registry(&config).unwrap_or_else(|_| Registry::new(vec![]));
            (config, registry)
        }
    }
}

pub fn init(config: Config, store: Store) {
    let (config, registry) = modules_for(config);
    let mut state = State {
        view: None,
        registry,
        env: Env { config, store, ranker: Ranker::new() },
        results: vec![],
        selected: 0,
        scroll: 0,
        status: None,
    };
    if let Err(e) = state.registry.migrate(&state.env.store) {
        eprintln!("flick: store migration failed: {e}");
    }
    state.registry.dispatch(Event::Started, &mut state.env.cx(""));
    STATE.with(|s| *s.borrow_mut() = Some(state));
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
    let showing = |s: &mut State| match (&s.view, &request) {
        (None, None) => true,
        (Some(view), Some(r)) => view.is(r.module, &r.name),
        _ => false,
    };
    if visible && with_state(showing).unwrap_or(false) {
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

/// Keyboard commands from the search field. Returns true when handled.
pub fn command(key: Key) -> bool {
    with_state(|s| {
        if key == Key::Up {
            s.move_selection(-1);
        } else if key == Key::Down {
            s.move_selection(1);
        } else if key == Key::Enter {
            s.activate();
        } else if key == Key::Tab {
            // Tab fills in a quicklink's argument, like Raycast.
            if s.results.get(s.selected).is_some_and(|i| i.tab) {
                s.activate();
            }
        } else if key == Key::Escape {
            if s.view.as_ref().is_none_or(|v| v.escape_hides) {
                ui::hide();
            } else {
                s.enter(None);
            }
        } else if key == Key::Backspace && s.view.is_some() && ui::query().is_empty() {
            s.enter(None);
        } else {
            return false;
        }
        true
    })
    .unwrap_or(false)
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
            if s.view.as_ref().is_some_and(|v| stale.contains(&v.module)) && ui::is_visible() {
                s.refresh();
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
pub fn control(words: &[String]) -> Result<String, String> {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else { return Err("Flick is busy; try again".into()) };
        let s = s.as_mut().ok_or("Flick is still starting")?;
        match words {
            [verb] if verb == "reload" => s.reload(),
            _ => s.registry.command(words, &mut s.env.cx("")),
        }
    })
}

impl State {
    /// Reload config.toml, reconfigure modules, migrate the store, and rebind hotkeys.
    /// `Err` is a load, configure, or binding error.
    fn reload(&mut self) -> Result<String, String> {
        let config = config::load()?;
        let started = modules::reload(&mut self.registry, &config)
            .map_err(|e| format!("{}: {e}", config::config_path().display()))?;
        self.env.config = config;
        if let Err(e) = self.registry.migrate(&self.env.store) {
            eprintln!("flick: store migration failed: {e}");
        }
        self.registry.dispatch_to(&started, Event::Started, &mut self.env.cx(""));
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

    /// Show `view` (a request its module opens), or root search for `None`.
    fn enter(&mut self, view: Option<ListView>) {
        self.view = match view {
            None => None,
            Some(request) => match self.registry.open(&request, &mut self.env.cx("")) {
                Some(view) => Some(view),
                None => return,
            },
        };
        let placeholder = self.view.as_ref().map_or(ROOT_PLACEHOLDER, |v| v.placeholder.as_str());
        ui::set_query("", placeholder);
        self.refresh();
    }

    fn refresh(&mut self) {
        let query = ui::query();
        self.results = match &mut self.view {
            None => {
                let usage = self.env.store.usage();
                let items = self.registry.items(&mut self.env.cx(&query));
                let direct = self.registry.direct(&mut self.env.cx(&query));
                root::rank(&mut self.env.ranker, &query, items, direct, &usage, store::now())
            }
            Some(view) => {
                self.registry.refresh(view, &mut self.env.cx(&query));
                std::mem::take(&mut view.items)
            }
        };
        self.selected = 0;
        self.scroll = 0;
        self.render();
    }

    fn render(&self) {
        let footer = match (&self.status, &self.view) {
            (Some(status), _) => status.as_str(),
            (None, None) => "Flick",
            (None, Some(view)) => view.footer.as_str(),
        };
        ui::render(&View {
            items: &self.results,
            selected: self.selected,
            scroll: self.scroll,
            footer,
            empty: self.view.as_ref().map_or("No Results", |v| v.empty.as_str()),
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
        if self.view.as_ref().is_none_or(|v| v.record_use) {
            self.env.store.record_use(item.id.as_str());
        }
        let query = ui::query();
        match self.registry.activate(&item.id, &mut self.env.cx(&query)) {
            Outcome::Hide => ui::hide(),
            Outcome::Stay(status) => {
                if let Some(status) = status {
                    self.set_status(status);
                }
            }
            Outcome::Push(view) => self.enter(Some(view)),
            Outcome::ReloadConfig => {
                let status = self.reload().unwrap_or_else(|e| e);
                self.set_status(status);
            }
        }
    }
}
