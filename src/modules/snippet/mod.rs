//! Module `snippet`: text Flick types into the frontmost app. Each `[[snippet.texts]]` entry
//! has a `name`, a `text` and an optional global `hotkey`. `{date}` in the text becomes
//! today's local date (`YYYY-MM-DD`) and `{time}` the local time (`HH:MM`), filled when the
//! snippet is typed. Each snippet is a root item: Enter hides the launcher and types it.
//! Its hotkey types it at once. `flick snippet type <name>` types it from this Mac (a
//! `[[keys.chord]]` can run that), and `flick snippet show <name>` prints it. Ids are
//! `snippet:<name>`.

mod wire;

use serde::Deserialize;

use crate::config::Section;
use crate::core::track::local_time;
use crate::core::{Binding, Cx, Icon, Item, ItemId, ListView, Module, Outcome, unknown_verb};

#[derive(Clone, Debug, Deserialize)]
struct Snippet {
    name: String,
    text: String,
    #[serde(default)]
    hotkey: Option<String>,
}

/// Table `[snippet]`: `[[snippet.texts]]`, unique names.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Settings {
    texts: Vec<Snippet>,
}

/// What the module asks of the system. Tests record it.
#[derive(Clone, Copy)]
struct Env {
    /// Unix time now and the local UTC offset at it, in seconds.
    now: fn() -> (i64, i32),
    /// Types the text into the frontmost app once the keyboard's modifiers are up.
    type_text: fn(String),
}

pub struct Snippets {
    texts: Vec<Snippet>,
    env: Env,
}

impl Default for Snippets {
    fn default() -> Self {
        Snippets { texts: vec![], env: Env { now: wire::now, type_text: wire::type_text } }
    }
}

/// `text` with each `{date}` as "YYYY-MM-DD" and each `{time}` as "HH:MM", local time at
/// Unix time `ts` with UTC offset `offset`.
fn render(text: &str, ts: i64, offset: i32) -> String {
    let stamp = local_time(ts, offset);
    let (date, time) = stamp.split_once(' ').unwrap_or((&stamp, ""));
    text.replace("{date}", date).replace("{time}", time)
}

/// The table's snippets. The name is the item id key and the hotkey key, so it must be
/// unique.
fn read(table: &Section) -> Result<Vec<Snippet>, String> {
    let texts = table.get::<Settings>()?.texts;
    for (i, s) in texts.iter().enumerate() {
        let name = &s.name;
        if name.trim().is_empty() {
            return Err("[snippet] texts: a snippet has no name".into());
        }
        let bad = |why: &str| Err(format!("[snippet] texts: \"{name}\": {why}"));
        if s.text.is_empty() {
            return bad("text is empty");
        }
        if texts[..i].iter().any(|p| p.name == *name) {
            return bad("duplicate name; keep one");
        }
    }
    Ok(texts)
}

impl Snippets {
    fn find(&self, name: &str) -> Option<&Snippet> {
        self.texts.iter().find(|s| s.name == name)
    }

    fn render(&self, s: &Snippet) -> String {
        let (ts, offset) = (self.env.now)();
        render(&s.text, ts, offset)
    }

    /// `snippet type|show <name...>`: the name words are joined with spaces.
    fn run_verb(&self, verb: &str, name: &[String]) -> Result<String, String> {
        let name = name.join(" ");
        let Some(s) = self.find(&name) else {
            let names: Vec<&str> = self.texts.iter().map(|s| s.name.as_str()).collect();
            let known = match names.as_slice() {
                [] => "none in [[snippet.texts]]".to_string(),
                names => format!("snippets: {}", names.join(", ")),
            };
            return Err(format!("snippet: no snippet \"{name}\" ({known})"));
        };
        let text = self.render(s);
        if verb == "show" {
            return Ok(text);
        }
        (self.env.type_text)(text.clone());
        Ok(format!("typed {name}: {text}"))
    }
}

impl Module for Snippets {
    fn id(&self) -> &'static str {
        "snippet"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.texts = read(table)?;
        Ok(())
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        self.texts
            .iter()
            .map(|s| Item {
                subtitle: self.render(s),
                accessory: "Snippet".into(),
                keywords: vec!["snippet".into()],
                ..Item::new(
                    ItemId::new("snippet", &s.name),
                    s.name.clone(),
                    "Type Snippet",
                    Icon::symbol("text.cursor"),
                )
            })
            .collect()
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        let Some(s) = self.find(id.key()) else { return Outcome::Stay(None) };
        let text = self.render(s);
        cx.hide();
        (self.env.type_text)(text);
        Outcome::Hide
    }

    fn hotkeys(&self) -> Vec<Binding> {
        let bound = self.texts.iter().filter_map(|s| {
            let spec = s.hotkey.as_ref().filter(|h| !h.trim().is_empty())?;
            Some(Binding { spec: spec.clone(), key: Ok(s.name.clone()) })
        });
        bound.collect()
    }

    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        if let Some(s) = self.find(key) {
            (self.env.type_text)(self.render(s));
        }
        None
    }

    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        match args.split_first() {
            Some((verb, [])) if verb == "type" || verb == "show" => {
                Err(format!("usage: snippet {verb} <name>"))
            }
            Some((verb, name)) if verb == "type" || verb == "show" => self.run_verb(verb, name),
            _ => Err(unknown_verb("snippet", args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "snippet type <name> | snippet show <name>"
    }
}

#[cfg(test)]
mod tests;
