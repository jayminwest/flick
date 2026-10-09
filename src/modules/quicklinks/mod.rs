//! Module `quicklink`: configured links in root search, and the view that takes a link's
//! argument. Ids are `quicklink:<name>`; the typed query rides in the id's `arg`.

mod link;

pub use link::Quicklink;
use serde::Deserialize;

use crate::config::{Config, Section};
use crate::core::{Cx, Icon, Item, ItemId, ListView, Module, Outcome};
use crate::platform::workspace;

/// Table `[quicklink]`: `[[quicklink.links]]` (legacy: top-level `[[quicklinks]]`), unique names.
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
    read(&section)
}

/// The table's links. Names must be unique: the name is the item id key, so a second link
/// with the same name could never be opened.
fn read(table: &Section) -> Result<Vec<Quicklink>, String> {
    let links = table.get::<Settings>()?.links;
    for (i, q) in links.iter().enumerate() {
        if links[..i].iter().any(|p| p.name == q.name) {
            let name = &q.name;
            return Err(format!("[quicklink] links: duplicate name \"{name}\"; keep one"));
        }
    }
    Ok(links)
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
        self.links = read(table)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    fn configured(text: &str) -> Result<Quicklinks, String> {
        let mut m = Quicklinks::default();
        m.configure(&parse(text)?.section("quicklink")?.ok_or("disabled")?)?;
        Ok(m)
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let link = |name: &str, url: &str| {
            format!("[[quicklink.links]]\nname = \"{name}\"\nurl = \"{url}\"\n")
        };
        let text = link("Docs", "https://a.com") + &link("Docs", "https://b.com");
        let err = configured(&text).err().unwrap();
        assert_eq!(err, "[quicklink] links: duplicate name \"Docs\"; keep one");
        assert_eq!(links(&parse(&text).unwrap()).unwrap_err(), err);
        // A legacy [[quicklinks]] entry counts too.
        let legacy = "[[quicklinks]]\nname = \"Docs\"\nurl = \"/\"\n".to_string();
        let legacy = legacy + &link("Docs", "/x");
        assert_eq!(configured(&legacy).err().unwrap(), err);
        // Names compare exactly, as item ids do.
        let mut m = configured(&(link("Docs", "/a") + &link("docs", "/b"))).unwrap();
        let items = crate::core::test_cx("", |cx| m.items(cx));
        let ids: Vec<_> = items.iter().map(|i| i.id.to_string()).collect();
        assert_eq!(ids, ["quicklink:Docs", "quicklink:docs"]);
    }
}
