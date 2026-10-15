//! Module `switcher`: the window switcher view. Ids are `switcher:<index>` into the window
//! list captured when the view opened. `cycle_hotkey` (e.g. ⌘Tab) steps through the same view
//! while held, ⇧ added steps back, and releasing it switches. ⌘Q (action `quit`) quits the
//! selected window's app; its windows leave the list once it has quit.

pub mod list;

use list::AppWindow;

use serde::Deserialize;

use crate::config::Section;
use crate::core::{Action, Binding, Cx, Cycle, Event, Icon, Item, ItemId, ListView, Module, Outcome, RecentPids};
use crate::platform::workspace;

/// Table `[switcher]`: `hotkey` (legacy: top-level `windows_hotkey`) and `cycle_hotkey`.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Settings {
    hotkey: Option<String>,
    cycle_hotkey: Option<String>,
}

/// `recent`: apps by activation, most recent first. `quit`: apps that quit since the view
/// opened; their windows stay in `windows` so ids keep their index, but are not listed.
#[derive(Default)]
pub struct Switcher {
    hotkey: Option<String>,
    cycle_hotkey: Option<String>,
    windows: Vec<AppWindow>,
    quit: Vec<i32>,
    recent: RecentPids,
}

impl Module for Switcher {
    fn id(&self) -> &'static str {
        "switcher"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let settings = table.get::<Settings>()?;
        self.hotkey = settings.hotkey;
        self.cycle_hotkey = settings.cycle_hotkey;
        Ok(())
    }

    /// View `windows`. Captures the window list once per open.
    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        if view != "windows" {
            return None;
        }
        self.windows = list::list_windows(self.recent.pids(), workspace::frontmost_pid());
        self.quit.clear();
        Some(ListView {
            placeholder: "Search windows…".into(),
            footer: format!("{} windows", self.windows.len()),
            empty: if self.windows.is_empty() {
                "No windows (Flick needs Accessibility permission)".into()
            } else {
                "No Results".into()
            },
            escape_hides: true,
            ..ListView::new("switcher", view)
        })
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        let items = self
            .windows
            .iter()
            .enumerate()
            .filter(|(_, w)| !self.quit.contains(&w.pid))
            .map(|(i, w)| Item {
                subtitle: if w.title == w.app { String::new() } else { w.app.clone() },
                accessory: if w.on_other_desktop() {
                    "Other Desktop".into()
                } else if w.minimized {
                    "Minimized".into()
                } else {
                    String::new()
                },
                keywords: vec![w.app.clone()],
                ..Item::new(
                    ItemId::new("switcher", i),
                    w.title.clone(),
                    "Switch to Window",
                    w.bundle.clone().map_or(Icon::symbol("macwindow"), Icon::File),
                )
            })
            .collect();
        // Bonus keeps most-recent-first order for equal scores.
        view.items = cx.ranker.rank(cx.query, items, |item| {
            item.id.key().parse::<usize>().map_or(0.0, |i| -(i as f64) * 1e-3)
        });
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        cx.hide();
        if let Some(w) = self.window(id) {
            list::focus(w);
        }
        Outcome::Hide
    }

    /// `quit` (⌘Q): quit the window's app, like ⌘Q in it.
    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        self.window(id)
            .map(|w| Action {
                shortcut_hint: "⌘Q",
                ..Action::new(QUIT, format!("Quit {}", w.app), Icon::symbol("xmark.circle"))
            })
            .into_iter()
            .collect()
    }

    /// The app may ask to save or refuse; its windows leave the list when it has quit.
    fn act(&mut self, id: &ItemId, key: &str, _cx: &mut Cx) -> Outcome {
        let Some(w) = self.window(id).filter(|_| key == QUIT) else { return Outcome::Stay(None) };
        // Its windows leaving the list say it quit; a status would outlive them.
        let quitting = workspace::terminate(w.pid);
        Outcome::Stay((!quitting).then(|| format!("Can't quit {}", w.app)))
    }

    /// Track app activations, starting from the frontmost app. A listed app that quits
    /// makes the view stale.
    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        self.recent.observe(event, workspace::frontmost_pid);
        match event {
            Event::AppTerminated { pid } if self.windows.iter().any(|w| w.pid == pid) => {
                self.quit.push(pid);
                true
            }
            _ => false,
        }
    }

    /// `hotkey` binds key `windows`; `cycle_hotkey` binds `next`, and with ⇧ added `previous`.
    fn hotkeys(&self) -> Vec<Binding> {
        let bind = |spec: &String, key: &str| Binding { spec: spec.clone(), key: Ok(key.into()) };
        let cycle = self.cycle_hotkey.iter();
        let back = cycle.clone().filter_map(|spec| with_shift(spec));
        self.hotkey
            .iter()
            .map(|spec| bind(spec, "windows"))
            .chain(cycle.map(|spec| bind(spec, "next")))
            .chain(back.map(|spec| bind(&spec, "previous")))
            .collect()
    }

    /// Toggles the launcher on view `windows`; `next` and `previous` cycle it.
    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        let cycle = match key {
            "windows" => None,
            "next" => Some(Cycle::Next),
            "previous" => Some(Cycle::Previous),
            _ => return None,
        };
        Some(ListView { cycle, ..ListView::new("switcher", "windows") })
    }
}

const QUIT: &str = "quit";

impl Switcher {
    fn window(&self, id: &ItemId) -> Option<&AppWindow> {
        id.key().parse::<usize>().ok().and_then(|i| self.windows.get(i))
    }
}

/// `spec` with ⇧ added before its key; `None` when it already holds ⇧.
fn with_shift(spec: &str) -> Option<String> {
    let (mods, key) = spec.rsplit_once('+')?;
    let shifted = mods.split('+').any(|m| m.eq_ignore_ascii_case("shift"));
    (!shifted).then(|| format!("{mods}+shift+{key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_hotkey_toggles_the_switcher_view() {
        let switcher = |text: &str| {
            let config = crate::config::parse(text).unwrap();
            let mut switcher = Switcher::default();
            switcher.configure(&config.section("switcher").unwrap().unwrap()).unwrap();
            switcher
        };
        let want = Binding { spec: "cmd+Space".into(), key: Ok("windows".into()) };
        assert_eq!(
            switcher("[switcher]\nhotkey = \"cmd+Space\"").hotkeys(),
            std::slice::from_ref(&want)
        );
        assert_eq!(switcher("windows_hotkey = \"cmd+Space\"").hotkeys(), [want]);
        assert!(switcher("").hotkeys().is_empty());
        crate::core::test_cx("", |cx| {
            let mut s = switcher("");
            assert!(s.hotkey("windows", cx).is_some_and(|v| v.is("switcher", "windows")));
            assert!(s.hotkey("other", cx).is_none());
        });
    }

    #[test]
    fn cycle_hotkey_binds_next_and_shifted_previous() {
        let config = crate::config::parse("[switcher]\ncycle_hotkey = \"cmd+Tab\"").unwrap();
        let mut s = Switcher::default();
        s.configure(&config.section("switcher").unwrap().unwrap()).unwrap();
        let bind = |spec: &str, key: &str| Binding { spec: spec.into(), key: Ok(key.into()) };
        assert_eq!(s.hotkeys(), [bind("cmd+Tab", "next"), bind("cmd+shift+Tab", "previous")]);
        crate::core::test_cx("", |cx| {
            let mut cycle = |key| s.hotkey(key, cx).filter(|v| v.is("switcher", "windows")).map(|v| v.cycle);
            assert_eq!(cycle("next"), Some(Some(Cycle::Next)));
            assert_eq!(cycle("previous"), Some(Some(Cycle::Previous)));
            assert_eq!(cycle("windows"), Some(None));
        });
        assert_eq!(with_shift("alt+Tab").as_deref(), Some("alt+shift+Tab"));
        assert_eq!(with_shift("cmd+Shift+Tab"), None);
        assert_eq!(with_shift("Tab"), None);
    }

    #[test]
    fn cmd_q_quits_the_app_and_its_windows_leave_once_it_quit() {
        const MAIL: i32 = i32::MAX - 1;
        let app = |pid, name: &str| workspace::RunningApp {
            pid,
            name: name.into(),
            bundle: None,
            hidden: false,
        };
        let apps = vec![(app(MAIL, "Mail"), vec![]), (app(MAIL + 1, "Notes"), vec![])];
        let mut s = Switcher {
            windows: list::arrange(apps, &[MAIL, MAIL + 1].into(), &[], None),
            ..Switcher::default()
        };
        let id = |i| ItemId::new("switcher", i);
        crate::core::test_cx("", |cx| {
            let actions = s.actions(&id(0), cx);
            assert_eq!(actions.len(), 1);
            assert_eq!((actions[0].title.as_str(), actions[0].shortcut_hint), ("Quit Mail", "⌘Q"));
            assert!(s.actions(&id(9), cx).is_empty());
            assert!(matches!(s.act(&id(0), "other", cx), Outcome::Stay(None)));
            // No process has this pid, so terminate fails without touching a real app.
            assert!(
                matches!(s.act(&id(0), QUIT, cx), Outcome::Stay(Some(t)) if t == "Can't quit Mail")
            );
            assert!(!s.on_event(Event::AppTerminated { pid: 99 }, cx));
            assert!(s.on_event(Event::AppTerminated { pid: MAIL }, cx));
            let mut view = ListView::new("switcher", "windows");
            s.refresh(&mut view, cx);
            let titles: Vec<_> = view.items.iter().map(|i| (i.id.key(), i.title.as_str())).collect();
            assert_eq!(titles, [("1", "Notes")]);
        });
    }

    #[test]
    fn the_example_config_lists_every_key() {
        crate::config::example::assert_documents::<Settings>("switcher");
    }
}
