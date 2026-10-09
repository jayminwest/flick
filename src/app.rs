//! Controller: launcher state, modes, and actions. All calls happen on the main thread.

use std::cell::RefCell;

use crate::apps::{self, App};
use crate::config::{self, Config};
use crate::platform::panel::Key;
use crate::platform::{app as platform_app, pasteboard, timer, workspace};
use crate::root::{rank_root, root_items};
use crate::search::{Action, Icon, Item, Ranker};
use crate::store::{self, Store};
use crate::ui::{self, VISIBLE_ROWS, View};
use crate::windows::{self, WindowAction};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Root,
    Clipboard,
    Windows,
    /// Typing the argument for quicklink `index`.
    Argument(usize),
}

pub struct State {
    mode: Mode,
    config: Config,
    apps: Vec<App>,
    store: Store,
    ranker: Ranker,
    results: Vec<Item>,
    windows: Vec<windows::AppWindow>,
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

pub fn init(config: Config, store: Store) {
    let pasteboard_count = pasteboard::change_count();
    let state = State {
        mode: Mode::Root,
        config,
        apps: apps::scan(),
        store,
        ranker: Ranker::new(),
        results: vec![],
        windows: vec![],
        selected: 0,
        scroll: 0,
        status: None,
        pasteboard_count,
    };
    STATE.with(|s| *s.borrow_mut() = Some(state));
}

pub fn toggle() {
    open_in(Mode::Root);
}

/// The window switcher hotkey.
pub fn toggle_windows() {
    open_in(Mode::Windows);
}

/// Show the panel in `mode`; hide it if it's already showing that mode.
fn open_in(mode: Mode) {
    let visible = ui::is_visible();
    if visible && with_state(|s| s.mode == mode).unwrap_or(false) {
        ui::hide();
        return;
    }
    with_state(|s| {
        if mode == Mode::Root {
            s.apps = apps::scan();
        }
        s.status = None;
        s.enter(mode);
    });
    if !visible {
        ui::show();
    }
}

/// A window action from a global hotkey, without the panel.
pub fn window_action(action: WindowAction) {
    if let Err(e) = windows::apply(action) {
        eprintln!("flick: {}: {e}", action.title());
    }
}

/// Show `query` in the root search (for `flick snapshot`).
pub fn set_root_query(query: &str) {
    with_state(|s| {
        s.enter(Mode::Root);
        ui::set_query(query, "Search for apps and commands…");
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
            if matches!(
                s.results.get(s.selected).map(|i| &i.action),
                Some(Action::Quicklink { query: None, .. })
            ) {
                s.activate();
            }
        } else if key == Key::Escape {
            if matches!(s.mode, Mode::Root | Mode::Windows) {
                ui::hide();
            } else {
                s.enter(Mode::Root);
            }
        } else if key == Key::Backspace && s.mode != Mode::Root && ui::query().is_empty() {
            s.enter(Mode::Root);
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
            if s.mode == Mode::Clipboard && ui::is_visible() {
                s.refresh();
            }
        }
    });
}

fn relative_time(ts: i64) -> String {
    let secs = (store::now() - ts).max(0);
    match secs {
        0..60 => "Just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

impl State {
    fn enter(&mut self, mode: Mode) {
        self.mode = mode;
        let placeholder = match mode {
            Mode::Root => "Search for apps and commands…".to_string(),
            Mode::Clipboard => "Search clipboard history…".to_string(),
            Mode::Windows => "Search windows…".to_string(),
            Mode::Argument(i) => format!("{} query…", self.config.quicklinks[i].name),
        };
        ui::set_query("", &placeholder);
        if mode == Mode::Windows {
            self.windows =
                windows::list_windows(&crate::spaces::recent(), crate::spaces::frontmost_pid());
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        let query = ui::query();
        self.results = match self.mode {
            Mode::Root => self.root_results(&query),
            Mode::Clipboard => self.clip_results(&query),
            Mode::Windows => self.window_results(&query),
            Mode::Argument(index) => {
                let q = &self.config.quicklinks[index];
                let subtitle =
                    if query.is_empty() { "Type a query".into() } else { q.expand(&query) };
                vec![Item {
                    id: format!("quicklink:{}", q.name),
                    title: q.name.clone(),
                    subtitle,
                    accessory: "Quicklink".into(),
                    icon: Icon::Symbol("link"),
                    action: Action::Quicklink { index, query: Some(query.clone()) },
                    keywords: vec![],
                }]
            }
        };
        self.selected = 0;
        self.scroll = 0;
        self.render();
    }

    fn root_results(&mut self, query: &str) -> Vec<Item> {
        let usage = self.store.usage();
        let items = root_items(&self.apps, &self.config.quicklinks);
        rank_root(&mut self.ranker, query, items, &self.config.quicklinks, &usage, store::now())
    }

    fn window_results(&mut self, query: &str) -> Vec<Item> {
        let items = self
            .windows
            .iter()
            .enumerate()
            .map(|(i, w)| Item {
                id: format!("window-item:{i}"),
                title: w.title.clone(),
                subtitle: if w.title == w.app { String::new() } else { w.app.clone() },
                accessory: if w.on_other_desktop() {
                    "Other Desktop".into()
                } else if w.minimized {
                    "Minimized".into()
                } else {
                    String::new()
                },
                icon: w.bundle.clone().map_or(Icon::Symbol("macwindow"), Icon::File),
                action: Action::FocusWindow(i),
                keywords: vec![w.app.clone()],
            })
            .collect();
        // Bonus keeps most-recent-first order for equal scores.
        self.ranker.rank(query, items, |item| match item.action {
            Action::FocusWindow(i) => -(i as f64) * 1e-3,
            _ => 0.0,
        })
    }

    fn clip_results(&mut self, query: &str) -> Vec<Item> {
        let clips = self.store.clips();
        let items = clips
            .iter()
            .map(|c| {
                let mut lines = c.text.trim().lines();
                let first: String = lines.next().unwrap_or("").trim().chars().take(100).collect();
                let more = lines.count();
                Item {
                    id: format!("clip:{}", c.id),
                    title: first,
                    subtitle: if more > 0 { format!("+{more} lines") } else { String::new() },
                    accessory: relative_time(c.ts),
                    icon: Icon::Symbol("doc.text"),
                    action: Action::PasteClip(c.id),
                    keywords: vec![c.text.chars().take(2000).collect()],
                }
            })
            .enumerate()
            .collect::<Vec<_>>();
        // Bonus keeps newest-first order for equal scores.
        let order: std::collections::HashMap<String, usize> =
            items.iter().map(|(i, item)| (item.id.clone(), *i)).collect();
        let items = items.into_iter().map(|(_, i)| i).collect();
        self.ranker.rank(query, items, |i| -(order[&i.id] as f64) * 1e-3)
    }

    fn render(&self) {
        let footer = match (&self.status, self.mode) {
            (Some(status), _) => status.clone(),
            (None, Mode::Root) => "Flick".into(),
            (None, Mode::Clipboard) => "Clipboard History  ·  esc to go back".into(),
            (None, Mode::Windows) => format!("{} windows", self.windows.len()),
            (None, Mode::Argument(i)) => {
                format!("{}  ·  esc to go back", self.config.quicklinks[i].name)
            }
        };
        let empty = match self.mode {
            Mode::Clipboard if ui::query().is_empty() => "Clipboard history is empty",
            Mode::Windows if self.windows.is_empty() => {
                "No windows (Flick needs Accessibility permission)"
            }
            _ => "No Results",
        };
        ui::render(&View {
            items: &self.results,
            selected: self.selected,
            scroll: self.scroll,
            footer: &footer,
            empty,
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

    fn set_status(&mut self, status: impl Into<String>) {
        self.status = Some(status.into());
        self.render();
    }

    fn activate(&mut self) {
        let Some(item) = self.results.get(self.selected).cloned() else { return };
        if !matches!(item.action, Action::PasteClip(_) | Action::FocusWindow(_)) {
            self.store.record_use(&item.id);
        }
        match item.action {
            Action::LaunchApp(path) => {
                ui::hide();
                workspace::open_file(&path);
            }
            Action::Window(action) => {
                ui::hide();
                if let Err(e) = windows::apply(action) {
                    eprintln!("flick: {}: {e}", action.title());
                }
            }
            Action::ClipboardHistory => self.enter(Mode::Clipboard),
            Action::SwitchWindows => self.enter(Mode::Windows),
            Action::FocusWindow(i) => {
                ui::hide();
                if let Some(w) = self.windows.get(i) {
                    windows::focus(w);
                }
            }
            Action::Quicklink { index, query: None } => self.enter(Mode::Argument(index)),
            Action::Quicklink { index, query: Some(query) } => {
                let link = &self.config.quicklinks[index];
                if link.takes_query() && query.trim().is_empty() {
                    return;
                }
                ui::hide();
                workspace::open_url(&link.expand(query.trim()));
            }
            Action::PasteClip(id) => {
                let Some(text) = self.store.clip_text(id) else { return };
                pasteboard::set_text(&text);
                ui::hide();
                // Give focus a moment to return to the previous app, then paste there.
                if windows::ensure_trusted() {
                    timer::after(0.08, windows::send_paste);
                }
            }
            Action::OpenConfig => {
                ui::hide();
                let _ =
                    std::process::Command::new("open").arg("-t").arg(config::config_path()).spawn();
            }
            Action::ReloadConfig => match config::load() {
                Ok(config) => {
                    let hotkey = crate::hotkey::register(&config);
                    self.config = config;
                    self.enter(Mode::Root);
                    match hotkey {
                        Ok(()) => self.set_status("Config reloaded"),
                        Err(e) => self.set_status(e),
                    }
                }
                Err(e) => self.set_status(e),
            },
            Action::Quit => platform_app::quit(),
        }
    }
}
