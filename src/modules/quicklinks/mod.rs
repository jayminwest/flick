//! Module `quicklink`: configured links in root search, and the view that takes a link's
//! argument. Ids are `quicklink:<name>`; the typed query rides in the id's `arg`.

mod link;

pub use link::Quicklink;
use serde::Deserialize;

use crate::config::{Config, Section};
use crate::core::{Cx, Icon, Item, ItemId, ListView, Module, Outcome};
use crate::platform::workspace;

/// Table `[quicklink]`: `[[quicklink.links]]` (legacy: top-level `[[quicklinks]]`).
#[derive(Default, Deserialize)]
#[serde(default)]
struct Settings {
    links: Vec<Quicklink>,
}

#[derive(Default)]
pub struct Quicklinks {
    links: Vec<Quicklink>,
}

impl Quicklinks {
    fn find(&self, name: &str) -> Option<&Quicklink> {
        self.links.iter().find(|q| q.name == name)
    }
}

/// The links `config` sets up; none when the module is disabled.
pub fn links(config: &Config) -> Result<Vec<Quicklink>, String> {
    let Some(section) = config.section("quicklink")? else { return Ok(vec![]) };
    Ok(section.get::<Settings>()?.links)
}

fn item(q: &Quicklink, arg: Option<String>) -> Item {
    let id = ItemId::new("quicklink", &q.name);
    let tab = arg.is_none();
    Item {
        accessory: "Quicklink".into(),
        tab,
        ..Item::new(
            match arg {
                Some(arg) => id.with_arg(arg),
                None => id,
            },
            q.name.clone(),
            if tab { "Enter Argument" } else { "Open Quicklink" },
            Icon::Symbol("link"),
        )
    }
}

/// The root item for "<keyword> <text>": runs link `q` with `text` directly.
fn keyword_item(q: &Quicklink, text: &str) -> Item {
    Item { subtitle: format!("“{text}”"), ..item(q, Some(text.to_string())) }
}

impl Module for Quicklinks {
    fn id(&self) -> &'static str {
        "quicklink"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.links = table.get::<Settings>()?.links;
        Ok(())
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        self.links
            .iter()
            .map(|q| Item {
                subtitle: q.keyword.clone().unwrap_or_default(),
                keywords: q.keyword.iter().cloned().collect(),
                // A link with no "{query}" opens as is; one with it asks for its argument.
                ..item(q, (!q.takes_query()).then(String::new))
            })
            .collect()
    }

    /// "<keyword> <text>" runs the first link with that keyword that takes a query.
    fn direct(&mut self, cx: &mut Cx) -> Vec<Item> {
        let Some((keyword, rest)) = cx.query.split_once(' ') else { return vec![] };
        let link = self.links.iter().find(|q| {
            q.takes_query() && q.keyword.as_deref() == Some(keyword) && !rest.trim().is_empty()
        });
        link.map(|q| keyword_item(q, rest.trim())).into_iter().collect()
    }

    /// View `<name>`: type the argument for that link.
    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        let q = self.find(view)?;
        Some(ListView {
            placeholder: format!("{} query…", q.name),
            footer: format!("{}  ·  esc to go back", q.name),
            record_use: true,
            ..ListView::new("quicklink", view)
        })
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        view.items = self
            .find(&view.name)
            .map(|q| Item {
                subtitle: if cx.query.is_empty() {
                    "Type a query".into()
                } else {
                    q.expand(cx.query)
                },
                ..item(q, Some(cx.query.to_string()))
            })
            .into_iter()
            .collect();
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        let Some(link) = self.find(id.key()) else { return Outcome::Stay(None) };
        let Some(query) = id.arg() else {
            return Outcome::Push(ListView::new("quicklink", id.key()));
        };
        if link.takes_query() && query.trim().is_empty() {
            return Outcome::Stay(None);
        }
        let url = link.expand(query.trim());
        cx.hide();
        workspace::open_url(&url);
        Outcome::Hide
    }
}
