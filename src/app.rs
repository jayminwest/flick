//! Controller: launcher state, modes, and actions. All calls happen on the main thread.

use std::cell::RefCell;

use objc2::runtime::Sel;
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSPasteboard, NSPasteboardTypeString, NSWorkspace};
use objc2_foundation::{NSString, NSURL};

use crate::apps::{self, App};
use crate::config::{self, Config};
use crate::search::{frecency, Action, Icon, Item, Ranker};
use crate::store::{self, Store};
use crate::ui::{self, View, VISIBLE_ROWS};
use crate::windows::{self, WindowAction};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Root,
    Clipboard,
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

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("Flick UI runs on the main thread")
}

pub fn init(config: Config, store: Store) {
    let pasteboard_count = NSPasteboard::generalPasteboard().changeCount();
    let state = State {
        mode: Mode::Root,
        config,
        apps: apps::scan(),
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

pub fn toggle() {
    if ui::is_visible() {
        ui::hide();
        return;
    }
    with_state(|s| {
        s.apps = apps::scan();
        s.status = None;
        s.enter(Mode::Root);
    });
    ui::show(mtm());
}

pub fn query_changed() {
    with_state(|s| {
        s.status = None;
        s.refresh();
    });
}

/// Keyboard commands from the search field. Returns true when handled.
pub fn command(sel: Sel) -> bool {
    with_state(|s| {
        if sel == sel!(moveUp:) {
            s.move_selection(-1);
        } else if sel == sel!(moveDown:) {
            s.move_selection(1);
        } else if sel == sel!(insertNewline:) {
            s.activate();
        } else if sel == sel!(insertTab:) {
            // Tab fills in a quicklink's argument, like Raycast.
            if matches!(s.results.get(s.selected).map(|i| &i.action), Some(Action::Quicklink { query: None, .. })) {
                s.activate();
            }
        } else if sel == sel!(cancelOperation:) {
            if s.mode == Mode::Root {
                ui::hide();
            } else {
                s.enter(Mode::Root);
            }
        } else if sel == sel!(deleteBackward:) && s.mode != Mode::Root && ui::query().is_empty() {
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
        let pb = NSPasteboard::generalPasteboard();
        let count = pb.changeCount();
        if count == s.pasteboard_count {
            return;
        }
        s.pasteboard_count = count;
        let skip = pb.types().is_some_and(|types| {
            types.iter().any(|t| {
                let t = t.to_string();
                t == "org.nspasteboard.ConcealedType" || t == "org.nspasteboard.TransientType"
            })
        });
        if skip {
            return;
        }
        if let Some(text) = pb.stringForType(unsafe { NSPasteboardTypeString }) {
            s.store.add_clip(&text.to_string());
            if s.mode == Mode::Clipboard && ui::is_visible() {
                s.refresh();
            }
        }
    });
}

fn open_url(url: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

fn root_items(s: &State) -> Vec<Item> {
    let mut items: Vec<Item> = s
        .apps
        .iter()
        .map(|a| Item {
            id: format!("app:{}", a.path.display()),
            title: a.name.clone(),
            subtitle: String::new(),
            accessory: "Application".into(),
            icon: Icon::File(a.path.clone()),
            action: Action::LaunchApp(a.path.clone()),
            keywords: vec![],
        })
        .collect();

    items.extend(WindowAction::ALL.iter().map(|&w| Item {
        id: format!("window:{}", w.title()),
        title: w.title().into(),
        subtitle: "Window Management".into(),
        accessory: "Command".into(),
        icon: Icon::Symbol(w.symbol()),
        action: Action::Window(w),
        keywords: vec!["window".into()],
    }));

    items.extend(s.config.quicklinks.iter().enumerate().map(|(index, q)| Item {
        id: format!("quicklink:{}", q.name),
        title: q.name.clone(),
        subtitle: q.keyword.clone().unwrap_or_default(),
        accessory: "Quicklink".into(),
        icon: Icon::Symbol("link"),
        action: Action::Quicklink { index, query: (!q.takes_query()).then(String::new) },
        keywords: q.keyword.iter().cloned().collect(),
    }));

    let builtins = [
        ("Clipboard History", "doc.on.clipboard", Action::ClipboardHistory, "paste"),
        ("Open Flick Config", "gearshape", Action::OpenConfig, "settings preferences"),
        ("Reload Flick Config", "arrow.clockwise", Action::ReloadConfig, "refresh"),
        ("Quit Flick", "power", Action::Quit, "exit"),
    ];
    items.extend(builtins.into_iter().map(|(title, symbol, action, keywords)| Item {
        id: format!("builtin:{title}"),
        title: title.into(),
        subtitle: "Flick".into(),
        accessory: "Command".into(),
        icon: Icon::Symbol(symbol),
        action,
        keywords: vec![keywords.into()],
    }));
    items
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
            Mode::Argument(i) => format!("{} query…", self.config.quicklinks[i].name),
        };
        ui::set_query("", &placeholder);
        self.refresh();
    }

    fn refresh(&mut self) {
        let query = ui::query();
        self.results = match self.mode {
            Mode::Root => self.root_results(&query),
            Mode::Clipboard => self.clip_results(&query),
            Mode::Argument(index) => {
                let q = &self.config.quicklinks[index];
                let subtitle = if query.is_empty() { "Type a query".into() } else { q.expand(&query) };
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
        let now = store::now();
        let items = root_items(self);
        let mut results = self.ranker.rank(query, items, |i| frecency(&usage, &i.id, now));

        // "<keyword> <text>" runs a quicklink directly.
        if let Some((keyword, rest)) = query.split_once(' ') {
            let link = self.config.quicklinks.iter().enumerate().find(|(_, q)| {
                q.takes_query() && q.keyword.as_deref() == Some(keyword) && !rest.trim().is_empty()
            });
            if let Some((index, q)) = link {
                results.insert(0, Item {
                    id: format!("quicklink:{}", q.name),
                    title: q.name.clone(),
                    subtitle: format!("“{}”", rest.trim()),
                    accessory: "Quicklink".into(),
                    icon: Icon::Symbol("link"),
                    action: Action::Quicklink { index, query: Some(rest.trim().to_string()) },
                    keywords: vec![],
                });
            }
        }
        results
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
            (None, Mode::Argument(i)) => format!("{}  ·  esc to go back", self.config.quicklinks[i].name),
        };
        let empty = match self.mode {
            Mode::Clipboard if ui::query().is_empty() => "Clipboard history is empty",
            _ => "No Results",
        };
        ui::render(&View { items: &self.results, selected: self.selected, scroll: self.scroll, footer: &footer, empty });
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
        if !matches!(item.action, Action::PasteClip(_)) {
            self.store.record_use(&item.id);
        }
        match item.action {
            Action::LaunchApp(path) => {
                ui::hide();
                NSWorkspace::sharedWorkspace().openURL(&NSURL::fileURLWithPath(&NSString::from_str(
                    &path.display().to_string(),
                )));
            }
            Action::Window(action) => {
                ui::hide();
                if let Err(e) = windows::apply(action, mtm()) {
                    eprintln!("flick: {}: {e}", action.title());
                }
            }
            Action::ClipboardHistory => self.enter(Mode::Clipboard),
            Action::Quicklink { index, query: None } => self.enter(Mode::Argument(index)),
            Action::Quicklink { index, query: Some(query) } => {
                let link = &self.config.quicklinks[index];
                if link.takes_query() && query.trim().is_empty() {
                    return;
                }
                ui::hide();
                open_url(&link.expand(query.trim()));
            }
            Action::PasteClip(id) => {
                let Some(text) = self.store.clip_text(id) else { return };
                let pb = NSPasteboard::generalPasteboard();
                pb.clearContents();
                pb.setString_forType(&NSString::from_str(&text), unsafe { NSPasteboardTypeString });
                ui::hide();
                // Give focus a moment to return to the previous app, then paste there.
                if windows::is_trusted(true) {
                    ui::after(0.08, windows::send_paste);
                }
            }
            Action::OpenConfig => {
                ui::hide();
                let _ = std::process::Command::new("open").arg("-t").arg(config::config_path()).spawn();
            }
            Action::ReloadConfig => match config::load() {
                Ok(config) => {
                    let hotkey = crate::hotkey::register(&config.hotkey);
                    self.config = config;
                    self.enter(Mode::Root);
                    match hotkey {
                        Ok(()) => self.set_status("Config reloaded"),
                        Err(e) => self.set_status(e),
                    }
                }
                Err(e) => self.set_status(e),
            },
            Action::Quit => NSApplication::sharedApplication(mtm()).terminate(None),
        }
    }
}
