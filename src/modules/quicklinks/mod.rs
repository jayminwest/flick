//! Module `quicklink`: configured links in root search, and the view that takes a link's
//! argument. Ids are `quicklink:<name>`; the typed query rides in the id's `arg`. The
//! editor (`editor.rs`) adds, edits and removes links in config.toml.

mod editor;
mod link;
mod validate;

use std::path::PathBuf;

pub use link::Quicklink;
use serde::Deserialize;

use crate::config::{Config, Section};
use crate::core::{Action, Cx, Form, Icon, Item, ItemId, ListView, Module, Outcome, Tab};
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
    /// The file the editor writes; `None` is config.toml. Tests point it at a temp file.
    file: Option<PathBuf>,
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
        tab: if tab { Tab::Activate } else { Tab::None },
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
        // Resolve the app first: an unknown one is a status line while the launcher stays up.
        let app = match link.app.as_deref() {
            Some(app) => match workspace::find_app(app) {
                Some(path) => Some(path),
                None => return Outcome::Stay(Some(format!("No app \"{app}\" to open {}", link.name))),
            },
            None => None,
        };
        cx.hide();
        match app {
            Some(app) => workspace::open_url_with(&url, &app),
            None => workspace::open_url(&url),
        }
        Outcome::Hide
    }

    fn form(&mut self, name: &str, _cx: &mut Cx) -> Option<Form> {
        self.editor_form(name)
    }

    fn submit(&mut self, form: &Form, _cx: &mut Cx) -> Result<String, String> {
        self.submit_form(form)
    }

    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        self.link_actions(id)
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        match key {
            "open" => self.activate(id, cx),
            _ => self.link_act(id, key),
        }
    }

    fn confirmed(&mut self, token: &str, _cx: &mut Cx) -> Outcome {
        self.delete_confirmed(token)
    }

    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        self.run_verb(args)
    }

    fn verbs(&self) -> &'static str {
        "quicklink list | quicklink add <name> <url> [--keyword k] [--app a] | quicklink remove <name>"
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

    #[test]
    fn open_with_app_is_optional_and_an_unknown_app_is_a_status() {
        let text = "[[quicklink.links]]\nname = \"Docs\"\nurl = \"https://docs.rs\"\n\
                    app = \"No Such App 7f3e\"\n\
                    [[quicklink.links]]\nname = \"Plain\"\nurl = \"/\"\n";
        let mut m = configured(text).unwrap();
        assert_eq!(m.links[0].app.as_deref(), Some("No Such App 7f3e"));
        assert_eq!(m.links[1].app, None);
        let id = ItemId::new("quicklink", "Docs").with_arg(String::new());
        let out = crate::core::test_cx("", |cx| m.activate(&id, cx));
        let msg = "No app \"No Such App 7f3e\" to open Docs";
        assert!(matches!(out, Outcome::Stay(Some(s)) if s == msg));
    }

    #[test]
    fn the_example_config_lists_every_key() {
        crate::config::example::assert_documents::<Settings>("quicklink");
        crate::config::example::assert_documents::<Quicklink>("quicklink.links");
    }
}
