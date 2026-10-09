//! Module `switcher`: the window switcher view. Ids are `switcher:<index>` into the window
//! list captured when the view opened.

use crate::core::{Cx, Icon, Item, ItemId, ListView, Module, Outcome};
use crate::spaces;
use crate::windows::{self, AppWindow};

#[derive(Default)]
pub struct Switcher {
    windows: Vec<AppWindow>,
}

impl Module for Switcher {
    fn id(&self) -> &'static str {
        "switcher"
    }

    /// View `windows`. Captures the window list once per open.
    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        if view != "windows" {
            return None;
        }
        self.windows = windows::list_windows(&spaces::recent(), spaces::frontmost_pid());
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
            windows::focus(w);
        }
        Outcome::Hide
    }
}
