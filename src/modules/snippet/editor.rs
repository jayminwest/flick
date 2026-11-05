//! The snippet editor: view `all` (browse every snippet, and a Create Snippet row), forms
//! `new` and `edit/<name>`, the Type/Copy/Edit/Delete actions, and the `add`/`remove`/`list`
//! verbs. Saves go through the config writer, then update the snippets in memory; the
//! controller rebinds hotkeys when a save changes them, so no reload is needed.

use std::collections::HashMap;

use super::{Snippet, Snippets};
use crate::config::edit::{self, Edit, Entry};
use crate::core::{
    Action, Confirm, ConfirmRow, Cx, Field, Form, Icon, Item, ItemId, ListView, Outcome,
};
use crate::platform::hotkeys::Hotkey;

/// The browse view.
pub const ALL: &str = "all";
/// The key of the Create Snippet row in `all`. A snippet name is never blank, so it never
/// clashes with one.
pub const CREATE: &str = "";
const EDIT: &str = "edit/";
const DELETE: &str = "delete/";

/// Puts text on the clipboard; tests record it instead.
#[cfg(not(test))]
const COPY: fn(&str) = crate::platform::pasteboard::set_text;
#[cfg(test)]
const COPY: fn(&str) = super::tests::record_copy;

/// A snippet from raw input: name and hotkey trimmed, a blank hotkey left out. The text is
/// kept as typed: trailing spaces or newlines may be the point.
pub fn draft(name: &str, text: &str, hotkey: &str) -> Snippet {
    let hotkey = hotkey.trim();
    Snippet {
        name: name.trim().into(),
        text: text.into(),
        hotkey: (!hotkey.is_empty()).then(|| hotkey.into()),
    }
}

/// Why `s` cannot join `texts`, or `Ok`. `editing` names the snippet it replaces, which
/// does not clash with itself.
pub fn check(texts: &[Snippet], editing: Option<&str>, s: &Snippet) -> Result<(), String> {
    if s.name.is_empty() {
        return Err("Name is required".into());
    }
    if s.text.is_empty() {
        return Err("Text is required".into());
    }
    let others = || texts.iter().filter(|o| Some(o.name.as_str()) != editing);
    // Names compare without case, so "date" next to "Date" is never ambiguous.
    if others().any(|o| o.name.eq_ignore_ascii_case(&s.name)) {
        return Err(format!("A snippet named \"{}\" already exists", s.name));
    }
    if let Some(spec) = &s.hotkey {
        let key = spec.parse::<Hotkey>().map_err(|e| format!("Hotkey \"{spec}\": {e}"))?;
        let same = |o: &&Snippet| o.hotkey.as_ref().and_then(|h| h.parse::<Hotkey>().ok());
        if let Some(o) = others().find(|o| same(o).is_some_and(|h| h.id() == key.id())) {
            return Err(format!("Hotkey \"{spec}\" is used by {}", o.name));
        }
    }
    Ok(())
}

impl Snippet {
    /// The snippet as config fields, in the order a hand-written entry uses.
    fn entry(&self) -> Entry {
        let mut entry = vec![("name", self.name.clone()), ("text", self.text.clone())];
        entry.extend(self.hotkey.clone().map(|h| ("hotkey", h)));
        entry
    }
}

impl Snippets {
    /// Write `edit` to the file that holds the snippets: this Mac's overlay when it sets
    /// them, else config.toml (or the test files).
    fn write(&self, edit: &Edit) -> Result<Option<usize>, String> {
        match &self.file {
            Some((path, host)) => edit::edit_in(path, host.as_deref(), "snippet", "texts", edit),
            None => edit::edit_entries("snippet", "texts", edit),
        }
    }

    /// Add `s`, or replace snippet `editing` with it.
    pub(super) fn save(&mut self, editing: Option<&str>, s: Snippet) -> Result<(), String> {
        check(&self.texts, editing, &s)?;
        match editing.and_then(|name| self.texts.iter().position(|o| o.name == name)) {
            Some(i) => {
                let name = self.texts[i].name.clone();
                self.write(&Edit::Replace { name, entry: s.entry() })?;
                self.texts[i] = s;
            }
            None if editing.is_some() => return Err("That snippet no longer exists".into()),
            None => {
                self.write(&Edit::Append(s.entry()))?;
                self.texts.push(s);
            }
        }
        Ok(())
    }

    pub(super) fn remove(&mut self, name: &str) -> Result<(), String> {
        let i = self.texts.iter().position(|s| s.name == name);
        let i = i.ok_or_else(|| format!("No snippet named \"{name}\""))?;
        self.write(&Edit::Remove { name: name.into() })?;
        self.texts.remove(i);
        Ok(())
    }

    pub(super) fn browse() -> ListView {
        ListView {
            placeholder: "Search snippets…".into(),
            footer: "Type Snippet  ·  ⌘K to edit or delete  ·  esc to go back".into(),
            ..ListView::new("snippet", ALL)
        }
    }

    /// View `all`: the snippets matching the query in config order, then Create Snippet.
    pub(super) fn fill(&self, view: &mut ListView, cx: &mut Cx) {
        // The hotkey, if any, stands where root search says "Snippet".
        let row = |s: &Snippet| Item {
            accessory: s.hotkey.clone().unwrap_or_default(),
            ..self.item(s)
        };
        let items: Vec<Item> = self.texts.iter().map(row).collect();
        let order: HashMap<String, usize> =
            items.iter().enumerate().map(|(i, item)| (item.id.to_string(), i)).collect();
        let mut items = cx.ranker.rank(cx.query, items, |i| -(order[i.id.as_str()] as f64));
        items.push(Item {
            subtitle: "Name, text and an optional hotkey".into(),
            ..Item::new(
                ItemId::new("snippet", CREATE),
                "Create Snippet",
                "Open Form",
                Icon::symbol("plus.square"),
            )
        });
        view.items = items;
    }

    /// Form `new`, or `edit/<name>` filled with that snippet.
    pub(super) fn editor_form(&self, name: &str) -> Option<Form> {
        let (title, label, s) = match name.strip_prefix(EDIT) {
            Some(old) => ("Edit Snippet", "Save Snippet", self.find(old)?.clone()),
            None if name == "new" => ("Create Snippet", "Create Snippet", draft("", "", "")),
            None => return None,
        };
        let fields = vec![
            Field::new("name", "Name").required().value(s.name).placeholder("Snippet name"),
            Field::new("text", "Text")
                .required()
                .multiline()
                .value(s.text)
                .placeholder("The text to type; {date} and {time} fill in when typed"),
            Field::new("hotkey", "Hotkey")
                .value(s.hotkey.unwrap_or_default())
                .placeholder("Optional, e.g. cmd+ctrl+alt+shift+KeyD"),
        ];
        Some(Form { fields, submit_label: label.into(), ..Form::new("snippet", name, title) })
    }

    pub(super) fn submit_form(&mut self, form: &Form) -> Result<String, String> {
        let field = |key| form.value(key).unwrap_or_default();
        let s = draft(field("name"), field("text"), field("hotkey"));
        let name = s.name.clone();
        self.save(form.name.strip_prefix(EDIT), s)?;
        Ok(format!("Saved snippet {name}"))
    }

    pub(super) fn snippet_actions(&self, id: &ItemId) -> Vec<Action> {
        if self.find(id.key()).is_none() {
            return vec![];
        }
        vec![
            Action::new("type", "Type Snippet", Icon::symbol("text.cursor")),
            Action::new("copy", "Copy Text", Icon::symbol("doc.on.doc")),
            Action::new("edit", "Edit Snippet", Icon::symbol("pencil")),
            Action::new("delete", "Delete Snippet", Icon::symbol("trash")),
        ]
    }

    /// `copy`, `edit` and `delete`; `type` is the caller's (it activates the item).
    pub(super) fn snippet_act(&self, id: &ItemId, key: &str) -> Outcome {
        let Some(s) = self.find(id.key()) else { return Outcome::Stay(None) };
        match key {
            "copy" => {
                COPY(&self.render(s));
                Outcome::Stay(Some(format!("Copied {}", s.name)))
            }
            "edit" => Outcome::Form { module: "snippet", name: format!("{EDIT}{}", s.name) },
            "delete" => Outcome::Confirm(Confirm {
                rows: vec![ConfirmRow { subtitle: s.text.clone(), ..ConfirmRow::new(&s.name) }],
                label: "Delete Snippet".into(),
                ..Confirm::new(
                    "snippet",
                    format!("{DELETE}{}", s.name),
                    format!("Delete snippet \"{}\"?", s.name),
                )
            }),
            _ => Outcome::Stay(None),
        }
    }

    pub(super) fn delete_confirmed(&mut self, token: &str) -> Outcome {
        let Some(name) = token.strip_prefix(DELETE) else { return Outcome::Stay(None) };
        Outcome::Stay(Some(match self.remove(name) {
            Ok(()) => format!("Deleted snippet {name}"),
            Err(e) => e,
        }))
    }

    /// `add <name> <text> [--hotkey h]`, `remove <name>`, `list`.
    pub(super) fn edit_verb(&mut self, args: &[String]) -> Option<Result<String, String>> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        Some(match words.as_slice() {
            ["list"] => Ok(self
                .texts
                .iter()
                .map(|s| {
                    let text = s.text.replace('\n', "\\n");
                    format!("{}\t{text}\t{}", s.name, s.hotkey.clone().unwrap_or_default())
                })
                .collect::<Vec<_>>()
                .join("\n")),
            ["remove", name] => self.remove(name).map(|()| format!("Removed snippet {name}")),
            ["add", name, text] => self.add(name, text, ""),
            ["add", name, text, "--hotkey", hotkey] => self.add(name, text, hotkey),
            ["add" | "remove", ..] => Err(USAGE.into()),
            _ => return None,
        })
    }

    fn add(&mut self, name: &str, text: &str, hotkey: &str) -> Result<String, String> {
        let s = draft(name, text, hotkey);
        let name = s.name.clone();
        self.save(None, s).map(|()| format!("Added snippet {name}"))
    }
}

const USAGE: &str =
    "usage: flick snippet add <name> <text> [--hotkey h] | flick snippet remove <name>";
