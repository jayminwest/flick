//! Module `snippet`: text Flick types into the frontmost app. Each `[[snippet.texts]]` entry
//! has a `name`, a `text` and an optional global `hotkey`. `{date}` in the text becomes
//! today's local date (`YYYY-MM-DD`) and `{time}` the local time (`HH:MM`), filled when the
//! snippet is typed. Each snippet is a root item: Enter hides the launcher and types it.
//! Its hotkey types it at once. `flick snippet type <name>` types it from this Mac (a
//! `[[keys.chord]]` can run that), and `flick snippet show <name>` prints it. Ids are
//! `snippet:<name>`. View `all` browses them; the editor (`editor.rs`) adds, edits and
//! removes them in config.toml, or in the per-host overlay when the overlay sets the list.

mod editor;
mod wire;

use std::path::PathBuf;

use serde::Deserialize;

use crate::config::Section;
use crate::core::track::local_time;
use crate::core::{
    Action, Binding, Cx, Form, Icon, Item, ItemId, ListView, Module, Outcome, unknown_verb,
};

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
    /// The config file and overlay host the editor writes; `None` is config.toml and this
    /// Mac's overlay. Tests point it at temp files.
    file: Option<(PathBuf, Option<String>)>,
}

impl Default for Snippets {
    fn default() -> Self {
        let env = Env { now: wire::now, type_text: wire::type_text };
        Snippets { texts: vec![], env, file: None }
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

    fn item(&self, s: &Snippet) -> Item {
        Item {
            subtitle: self.render(s),
            accessory: "Snippet".into(),
            keywords: vec!["snippet".into()],
            ..Item::new(
                ItemId::new("snippet", &s.name),
                s.name.clone(),
                "Type Snippet",
                Icon::symbol("text.cursor"),
            )
        }
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
        self.texts.iter().map(|s| self.item(s)).collect()
    }

    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        (view == editor::ALL).then(Self::browse)
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        self.fill(view, cx);
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        if id.key() == editor::CREATE {
            return Outcome::Form { module: "snippet", name: "new".into() };
        }
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

    fn form(&mut self, name: &str, _cx: &mut Cx) -> Option<Form> {
        self.editor_form(name)
    }

    fn submit(&mut self, form: &Form, _cx: &mut Cx) -> Result<String, String> {
        self.submit_form(form)
    }

    fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
        self.snippet_actions(id)
    }

    fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        match key {
            "type" => self.activate(id, cx),
            _ => self.snippet_act(id, key),
        }
    }

    fn confirmed(&mut self, token: &str, _cx: &mut Cx) -> Outcome {
        self.delete_confirmed(token)
    }

    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        if let Some(result) = self.edit_verb(args) {
            return result;
        }
        match args.split_first() {
            Some((verb, [])) if verb == "type" || verb == "show" => {
                Err(format!("usage: snippet {verb} <name>"))
            }
            Some((verb, name)) if verb == "type" || verb == "show" => self.run_verb(verb, name),
            _ => Err(unknown_verb("snippet", args)),
        }
    }

    fn verbs(&self) -> &'static str {
        "snippet type <name> | snippet show <name> | snippet list | \
         snippet add <name> <text> [--hotkey h] | snippet remove <name>"
    }
}

#[cfg(test)]
mod tests;
