//! Module `builtin`: Flick's own commands in root search. Ids are `builtin:<title>`.

use crate::config;
use crate::core::{Cx, Icon, Item, ItemId, ListView, Module, Outcome};
use crate::platform::app;

pub struct Flick;

/// Title, SF Symbol, verb, keywords.
const BUILTINS: [(&str, &str, &str, &str); 5] = [
    ("Clipboard History", "doc.on.clipboard", "Open Command", "paste"),
    ("Switch Windows", "macwindow.on.rectangle", "Open Command", "focus alt tab"),
    ("Open Flick Config", "gearshape", "Run Command", "settings preferences"),
    ("Reload Flick Config", "arrow.clockwise", "Run Command", "refresh"),
    ("Quit Flick", "power", "Run Command", "exit"),
];

impl Module for Flick {
    fn id(&self) -> &'static str {
        "builtin"
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        BUILTINS
            .into_iter()
            .map(|(title, symbol, verb, keywords)| Item {
                subtitle: "Flick".into(),
                accessory: "Command".into(),
                keywords: vec![keywords.into()],
                ..Item::new(ItemId::new("builtin", title), title, verb, Icon::Symbol(symbol))
            })
            .collect()
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        match id.key() {
            "Clipboard History" => Outcome::Push(ListView::new("clip", "history")),
            "Switch Windows" => Outcome::Push(ListView::new("switcher", "windows")),
            "Open Flick Config" => {
                cx.hide();
                let _ =
                    std::process::Command::new("open").arg("-t").arg(config::config_path()).spawn();
                Outcome::Hide
            }
            "Reload Flick Config" => Outcome::ReloadConfig,
            "Quit Flick" => {
                app::quit();
                Outcome::Stay(None)
            }
            _ => Outcome::Stay(None),
        }
    }
}
