//! Controller: launcher state, the view stack, and routing to modules. All calls happen
//! on the main thread.

use std::cell::RefCell;

use crate::apps;
use crate::config::Config;
use crate::core::{Cx, Event, Item, ListView, Outcome, Ranker, Registry};
use crate::platform::panel::Key;
use crate::platform::pasteboard;
use crate::root::{rank_root, registry};
use crate::store::{self, Store};
use crate::ui::{self, VISIBLE_ROWS, View};
use crate::windows::{self, WindowAction};

const ROOT_PLACEHOLDER: &str = "Search for apps and commands…";

pub struct State {
    /// The module view on screen; `None` is root search.
    view: Option<ListView>,
    config: Config,
    registry: Registry,
    store: Store,
    ranker: Ranker,
    results: Vec<Item>,
    selected: usize,
    scroll: usize,
    status: Option<String>,
    pasteboard_count: isize,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

/// A module context over `state`'s fields. A macro, not a method, so the borrow covers
/// only those fields and `state.registry` stays free to call.
macro_rules! cx {
    ($state:expr, $query:expr) => {
        Cx {
            query: $query,
            config: &mut $state.config,
            store: &$state.store,
            ranker: &mut $state.ranker,
            hide: ui::hide,
        }
    };
}

pub fn init(config: Config, store: Store) {
    let pasteboard_count = pasteboard::change_count();
    let state = State {
        view: None,
        config,
        registry: registry(apps::scan()),
        store,
        ranker: Ranker::new(),
        results: vec![],
        selected: 0,
        scroll: 0,
        status: None,
        pasteboard_count,
    };
    STATE.with(|s| *s.borrow_mut() = Some(state));
}

/// The launcher hotkey: show root search, or hide it if it's showing.
pub fn toggle() {
    let visible = ui::is_visible();
    if visible && with_state(|s| s.view.is_none()).unwrap_or(false) {
        ui::hide();
        return;
    }
    with_state(|s| {
        s.registry.dispatch(Event::LauncherOpened, &mut cx!(s, ""));
        s.status = None;
        s.enter(None);
    });
    if !visible {
        ui::show();
    }
}

/// The window switcher hotkey: show the switcher, or hide it if it's showing.
pub fn toggle_windows() {
    let visible = ui::is_visible();
    let showing = |s: &mut State| s.view.as_ref().is_some_and(|v| v.is("switcher", "windows"));
    if visible && with_state(showing).unwrap_or(false) {
        ui::hide();
        return;
    }
    with_state(|s| {
        s.status = None;
        s.enter(Some(ListView::new("switcher", "windows")));
    });
    if !visible {
        ui::show();
    }
}

/// A window action from a global hotkey, without the panel.
pub fn window_action(action: WindowAction) {
    windows::run(action);
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

/// Record new clipboard text. Skips content that password managers mark as concealed or transient.
pub fn poll_clipboard() {
    with_state(|s| {
        let count = pasteboard::change_count();
        if count == s.pasteboard_count {
            return;
        }
        s.pasteboard_count = count;
        if let Some(text) = pasteboard::copied_text() {
            s.store.add_clip(&text);
            if s.view.as_ref().is_some_and(|v| v.is("clip", "history")) && ui::is_visible() {
                s.refresh();
            }
        }
    });
}

impl State {
    /// Show `view` (a request its module opens), or root search for `None`.
    fn enter(&mut self, view: Option<ListView>) {
        self.view = match view {
            None => None,
            Some(request) => match self.registry.open(&request, &mut cx!(self, "")) {
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
                let usage = self.store.usage();
                let items = self.registry.items(&mut cx!(self, &query));
                let links = &self.config.quicklinks;
                rank_root(&mut self.ranker, &query, items, links, &usage, store::now())
            }
            Some(view) => {
                self.registry.refresh(view, &mut cx!(self, &query));
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
            self.store.record_use(item.id.as_str());
        }
        let query = ui::query();
        match self.registry.activate(&item.id, &mut cx!(self, &query)) {
            Outcome::Hide => ui::hide(),
            Outcome::Stay(status) => {
                if let Some(status) = status {
                    self.set_status(status);
                }
            }
            Outcome::Push(view) => self.enter(Some(view)),
            Outcome::Pop(status) => {
                self.enter(None);
                if let Some(status) = status {
                    self.set_status(status);
                }
            }
        }
    }
}
