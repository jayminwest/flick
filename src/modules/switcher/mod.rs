//! Module `switcher`: the window switcher view. Ids are `switcher:<index>` into the window
//! list captured when the view opened.

pub mod list;

use list::AppWindow;

use serde::Deserialize;

use crate::config::Section;
use crate::core::{Binding, Cx, Event, Icon, Item, ItemId, ListView, Module, Outcome, RecentPids};
use crate::platform::workspace;

/// Table `[switcher]`: `hotkey` (legacy: top-level `windows_hotkey`).
#[derive(Default, Deserialize)]
#[serde(default)]
struct Settings {
    hotkey: Option<String>,
}

/// `recent`: apps by activation, most recent first.
#[derive(Default)]
pub struct Switcher {
    hotkey: Option<String>,
    windows: Vec<AppWindow>,
    recent: RecentPids,
}

impl Module for Switcher {
    fn id(&self) -> &'static str {
        "switcher"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.hotkey = table.get::<Settings>()?.hotkey;
        Ok(())
    }

    /// View `windows`. Captures the window list once per open.
    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        if view != "windows" {
            return None;
        }
        self.windows = list::list_windows(self.recent.pids(), workspace::frontmost_pid());
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
                    w.bundle.clone().map_or(Icon::Symbol("macwindow"), Icon::File),
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
        if let Some(w) = id.key().parse::<usize>().ok().and_then(|i| self.windows.get(i)) {
            list::focus(w);
        }
        Outcome::Hide
    }

    /// Track app activations, starting from the frontmost app.
    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        self.recent.observe(event, workspace::frontmost_pid);
        false
    }

    /// `hotkey` binds key `windows`.
    fn hotkeys(&self) -> Vec<Binding> {
        self.hotkey
            .iter()
            .map(|spec| Binding { spec: spec.clone(), key: Ok("windows".into()) })
            .collect()
    }

    /// Toggles the launcher on view `windows`.
    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        (key == "windows").then(|| ListView::new("switcher", "windows"))
    }
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
    fn the_example_config_lists_every_key() {
        crate::config::example::assert_documents::<Settings>("switcher");
    }
}
