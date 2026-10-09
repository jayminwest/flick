//! The quicklink editor: forms `new` and `edit/<name>`, the Edit/Delete actions, and the
//! `add`/`remove`/`list` verbs. Saves go through the config writer, then update the links
//! in memory, so no reload rebinds hotkeys or resets the launcher.

use super::validate::{check, draft};
use super::{Quicklink, Quicklinks};
use crate::config::edit::{self, Edit, Entry};
use crate::core::{Action, Confirm, ConfirmRow, Field, Form, Icon, ItemId, Outcome, unknown_verb};

const EDIT: &str = "edit/";
const DELETE: &str = "delete/";

/// `link` as config fields, in the order a hand-written entry uses.
fn entry(link: &Quicklink) -> Entry {
    let mut entry = vec![("name", link.name.clone()), ("url", link.url.clone())];
    entry.extend(link.keyword.clone().map(|k| ("keyword", k)));
    entry.extend(link.app.clone().map(|a| ("app", a)));
    entry
}

impl Quicklinks {
    /// Write `edit` to config.toml (or the test file).
    fn write(&self, edit: &Edit) -> Result<(), String> {
        match &self.file {
            Some(path) => edit::edit_file(path, "quicklink", "links", edit),
            None => edit::edit_entries("quicklink", "links", edit),
        }
    }

    /// Add `link`, or replace link `editing` with it.
    pub(super) fn save(&mut self, editing: Option<&str>, link: Quicklink) -> Result<(), String> {
        check(&self.links, editing, &link)?;
        match editing.and_then(|name| self.links.iter().position(|q| q.name == name)) {
            Some(i) => {
                let name = self.links[i].name.clone();
                self.write(&Edit::Replace { name, entry: entry(&link) })?;
                self.links[i] = link;
            }
            None if editing.is_some() => return Err("That quicklink no longer exists".into()),
            None => {
                self.write(&Edit::Append(entry(&link)))?;
                self.links.push(link);
            }
        }
        Ok(())
    }

    pub(super) fn remove(&mut self, name: &str) -> Result<(), String> {
        let i = self.links.iter().position(|q| q.name == name);
        let i = i.ok_or_else(|| format!("No quicklink named \"{name}\""))?;
        self.write(&Edit::Remove { name: name.into() })?;
        self.links.remove(i);
        Ok(())
    }

    /// Form `new`, or `edit/<name>` filled with that link.
    pub(super) fn editor_form(&self, name: &str) -> Option<Form> {
        let (title, label, link) = match name.strip_prefix(EDIT) {
            Some(old) => ("Edit Quicklink", "Save Quicklink", self.find(old)?.clone()),
            None if name == "new" => ("Create Quicklink", "Create Quicklink", draft("", "", "", "")),
            None => return None,
        };
        let fields = vec![
            Field::new("name", "Name").required().value(link.name).placeholder("Quicklink name"),
            Field::new("url", "URL").required().value(link.url).placeholder(
                "https://example.com/search?q={query}, {query} for an argument, or a path",
            ),
            Field::new("keyword", "Keyword").value(link.keyword.unwrap_or_default()).placeholder(
                "Optional: type \"<keyword> <text>\" in root search",
            ),
            Field::new("app", "Open with")
                .value(link.app.unwrap_or_default())
                .placeholder("Optional: an app name or .app path"),
        ];
        Some(Form { fields, submit_label: label.into(), ..Form::new("quicklink", name, title) })
    }

    pub(super) fn submit_form(&mut self, form: &Form) -> Result<String, String> {
        let field = |key| form.value(key).unwrap_or_default();
        let link = draft(field("name"), field("url"), field("keyword"), field("app"));
        let name = link.name.clone();
        let editing = form.name.strip_prefix(EDIT);
        self.save(editing, link)?;
        Ok(format!("Saved quicklink {name}"))
    }

    pub(super) fn link_actions(&self, id: &ItemId) -> Vec<Action> {
        if self.find(id.key()).is_none() {
            return vec![];
        }
        vec![
            Action::new("open", "Open Quicklink", Icon::Symbol("link")),
            Action::new("edit", "Edit Quicklink", Icon::Symbol("pencil")),
            Action::new("delete", "Delete Quicklink", Icon::Symbol("trash")),
        ]
    }

    /// `edit` and `delete`; `open` is the caller's (it activates the item).
    pub(super) fn link_act(&self, id: &ItemId, key: &str) -> Outcome {
        let Some(link) = self.find(id.key()) else { return Outcome::Stay(None) };
        match key {
            "edit" => Outcome::Form { module: "quicklink", name: format!("{EDIT}{}", link.name) },
            "delete" => Outcome::Confirm(Confirm {
                rows: vec![ConfirmRow { subtitle: link.url.clone(), ..ConfirmRow::new(&link.name) }],
                label: "Delete Quicklink".into(),
                ..Confirm::new(
                    "quicklink",
                    format!("{DELETE}{}", link.name),
                    format!("Delete quicklink \"{}\"?", link.name),
                )
            }),
            _ => Outcome::Stay(None),
        }
    }

    pub(super) fn delete_confirmed(&mut self, token: &str) -> Outcome {
        let Some(name) = token.strip_prefix(DELETE) else { return Outcome::Stay(None) };
        Outcome::Stay(Some(match self.remove(name) {
            Ok(()) => format!("Deleted quicklink {name}"),
            Err(e) => e,
        }))
    }

    /// `add <name> <url> [--keyword k] [--app a]`, `remove <name>`, `list`.
    pub(super) fn run_verb(&mut self, args: &[String]) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["list"] => Ok(self
                .links
                .iter()
                .map(|q| {
                    let opt = |o: &Option<String>| o.clone().unwrap_or_default();
                    format!("{}\t{}\t{}\t{}", q.name, q.url, opt(&q.keyword), opt(&q.app))
                })
                .collect::<Vec<_>>()
                .join("\n")),
            ["remove", name] => self.remove(name).map(|()| format!("Removed quicklink {name}")),
            ["add", name, url, rest @ ..] => {
                let (mut keyword, mut app) = ("", "");
                for pair in rest.chunks(2) {
                    match pair {
                        ["--keyword", k] => keyword = k,
                        ["--app", a] => app = a,
                        _ => return Err(ADD_USAGE.into()),
                    }
                }
                let link = draft(name, url, keyword, app);
                let name = link.name.clone();
                self.save(None, link).map(|()| format!("Added quicklink {name}"))
            }
            ["add", ..] => Err(ADD_USAGE.into()),
            _ => Err(unknown_verb("quicklink", args)),
        }
    }
}

const ADD_USAGE: &str = "usage: flick quicklink add <name> <url> [--keyword k] [--app a]";

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::config::parse;
    use crate::core::{Module, test_cx};

    const CONFIG: &str = "# mine\n\
        [[quicklinks]]\nname = \"Old\"   # legacy\nurl = \"https://old.example\"\n\n\
        [[quicklink.links]]\nname = \"Docs\"\nurl = \"https://docs.rs/{query}\"\nkeyword = \"d\"\n";

    /// A module configured from `text`, writing to its own temp copy (never ~/.config).
    fn module(name: &str, text: &str) -> (Quicklinks, PathBuf) {
        let dir = std::env::temp_dir().join(format!("flk-{}-ql-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, text).unwrap();
        let mut m = Quicklinks { file: Some(path.clone()), ..Quicklinks::default() };
        m.configure(&parse(text).unwrap().section("quicklink").unwrap().unwrap()).unwrap();
        (m, path)
    }

    fn run(m: &mut Quicklinks, words: &[&str]) -> Result<String, String> {
        let args: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
        test_cx("", |cx| m.command(&args, cx))
    }

    fn names(m: &Quicklinks) -> Vec<&str> {
        m.links.iter().map(|q| q.name.as_str()).collect()
    }

    /// The links a fresh load of `path` sees.
    fn reloaded(path: &PathBuf) -> Vec<String> {
        let text = fs::read_to_string(path).unwrap();
        let (m, _) = module("reload", &text);
        m.links.iter().map(|q| format!("{}={}", q.name, q.url)).collect()
    }

    /// `Stay`'s status; panics on any other outcome.
    fn status(out: Outcome) -> Option<String> {
        let Outcome::Stay(status) = out else { panic!("{out:?}") };
        status
    }

    #[test]
    fn add_verb_appends_and_applies_at_once() {
        let (mut m, path) = module("add", CONFIG);
        let out = run(&mut m, &["add", "Crates", "https://crates.io/search?q={query}"]);
        assert_eq!(out.unwrap(), "Added quicklink Crates");
        let args = ["add", "Rs", "https://rs.io/{query}", "--keyword", "r", "--app", "Safari"];
        assert_eq!(run(&mut m, &args).unwrap(), "Added quicklink Rs");
        // "<keyword> <text>" works at once, without a reload.
        let direct = test_cx("r serde", |cx| m.direct(cx));
        assert_eq!(direct[0].id.arg(), Some("serde"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(CONFIG), "{text}");
        assert!(text.ends_with("keyword = \"r\"\napp = \"Safari\"\n"), "{text}");
        assert_eq!(reloaded(&path).len(), 4);
        let dup = run(&mut m, &["add", "docs", "x"]).unwrap_err();
        assert_eq!(dup, "A quicklink named \"docs\" already exists");
    }

    #[test]
    fn list_and_remove_verbs() {
        let (mut m, path) = module("remove", CONFIG);
        let list = run(&mut m, &["list"]).unwrap();
        assert_eq!(list, "Docs\thttps://docs.rs/{query}\td\t\nOld\thttps://old.example\t\t");
        assert_eq!(run(&mut m, &["remove", "Docs"]).unwrap(), "Removed quicklink Docs");
        let again = run(&mut m, &["remove", "Docs"]).unwrap_err();
        assert_eq!(again, "No quicklink named \"Docs\"");
        assert_eq!(names(&m), ["Old"]);
        assert_eq!(reloaded(&path), ["Old=https://old.example"]);
    }

    #[test]
    fn bad_verbs_say_how() {
        let (mut m, _) = module("usage", CONFIG);
        let bad: [&[&str]; 3] =
            [&["add", "X"], &["add", "X", "/", "--keyword"], &["add", "X", "/", "--k", "v"]];
        for args in bad {
            assert_eq!(run(&mut m, args).unwrap_err(), ADD_USAGE);
        }
        assert_eq!(run(&mut m, &["nope"]).unwrap_err(), "quicklink: unknown command \"nope\"");
        assert!(m.verbs().contains("quicklink add <name> <url>"));
    }

    #[test]
    fn forms_are_new_and_edit_slash_name() {
        let (mut m, _) = module("forms", CONFIG);
        let new = test_cx("", |cx| m.form("new", cx)).unwrap();
        let keys: Vec<_> = new.fields.iter().map(|f| f.key).collect();
        assert_eq!(keys, ["name", "url", "keyword", "app"]);
        assert!(new.fields.iter().all(|f| f.value.is_empty()));
        assert_eq!((new.title.as_str(), new.focused), ("Create Quicklink", 0));
        let edit = test_cx("", |cx| m.form("edit/Docs", cx)).unwrap();
        assert_eq!((edit.value("keyword"), edit.submit_label.as_str()), (Some("d"), "Save Quicklink"));
        assert!(test_cx("", |cx| m.form("edit/Nope", cx)).is_none());
        assert!(test_cx("", |cx| m.form("other", cx)).is_none());
    }

    #[test]
    fn editing_renames_in_place() {
        let (mut m, path) = module("edit", CONFIG);
        let mut edit = test_cx("", |cx| m.form("edit/Docs", cx)).unwrap();
        edit.set_value(0, " Rust Docs ");
        edit.set_value(2, "rd");
        edit.set_value(3, "Safari");
        assert_eq!(test_cx("", |cx| m.submit(&edit, cx)).unwrap(), "Saved quicklink Rust Docs");
        assert_eq!(names(&m), ["Rust Docs", "Old"]);
        assert!(test_cx("d x", |cx| m.direct(cx)).is_empty());
        assert_eq!(test_cx("rd x", |cx| m.direct(cx)).len(), 1);
        // A legacy link is edited where it is, its comment kept.
        let mut old = test_cx("", |cx| m.form("edit/Old", cx)).unwrap();
        old.set_value(1, "https://new.example");
        test_cx("", |cx| m.submit(&old, cx)).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let legacy = "[[quicklinks]]\nname = \"Old\"   # legacy\nurl = \"https://new.example\"\n";
        assert!(text.contains(legacy), "{text}");
        let links = ["Rust Docs=https://docs.rs/{query}", "Old=https://new.example"];
        assert_eq!(reloaded(&path), links);
    }

    #[test]
    fn submit_errors_keep_the_form() {
        let (mut m, _) = module("errors", CONFIG);
        let mut bad = test_cx("", |cx| m.form("new", cx)).unwrap();
        bad.set_value(0, "old");
        bad.set_value(1, "/");
        let err = test_cx("", |cx| m.submit(&bad, cx)).unwrap_err();
        assert_eq!(err, "A quicklink named \"old\" already exists");
        // The link went away behind the open form.
        let edit = test_cx("", |cx| m.form("edit/Docs", cx)).unwrap();
        m.links.retain(|q| q.name != "Docs");
        let gone = test_cx("", |cx| m.submit(&edit, cx)).unwrap_err();
        assert_eq!(gone, "That quicklink no longer exists");
    }

    #[test]
    fn actions_are_open_edit_delete() {
        let (mut m, _) = module("actions", CONFIG);
        let id = ItemId::new("quicklink", "Docs");
        let keys: Vec<_> = test_cx("", |cx| m.actions(&id, cx)).iter().map(|a| a.key).collect();
        assert_eq!(keys, ["open", "edit", "delete"]);
        let nope = ItemId::new("quicklink", "Nope");
        assert!(test_cx("", |cx| m.actions(&nope, cx)).is_empty());
        assert_eq!(status(test_cx("", |cx| m.act(&nope, "edit", cx))), None);
        assert_eq!(status(test_cx("", |cx| m.act(&id, "zap", cx))), None);
        let edit = test_cx("", |cx| m.act(&id, "edit", cx));
        assert!(matches!(edit, Outcome::Form { module: "quicklink", name } if name == "edit/Docs"));
        // "open" on a link taking a query asks for the argument.
        assert!(matches!(test_cx("", |cx| m.act(&id, "open", cx)), Outcome::Push(_)));
    }

    #[test]
    fn delete_asks_first_then_removes() {
        let (mut m, path) = module("delete", CONFIG);
        let id = ItemId::new("quicklink", "Docs");
        let Outcome::Confirm(c) = test_cx("", |cx| m.act(&id, "delete", cx)) else { panic!() };
        assert_eq!((c.token.as_str(), c.label.as_str()), ("delete/Docs", "Delete Quicklink"));
        assert!(!c.destructive);
        assert_eq!(c.rows[0].subtitle, "https://docs.rs/{query}");
        let out = status(test_cx("", |cx| m.confirmed(&c.token, cx)));
        assert_eq!(out.as_deref(), Some("Deleted quicklink Docs"));
        assert_eq!(names(&m), ["Old"]);
        assert_eq!(reloaded(&path), ["Old=https://old.example"]);
        let again = status(test_cx("", |cx| m.confirmed(&c.token, cx)));
        assert_eq!(again.as_deref(), Some("No quicklink named \"Docs\""));
        assert_eq!(status(test_cx("", |cx| m.confirmed("other", cx))), None);
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let (mut m, path) = module("badfile", CONFIG);
        fs::write(&path, "not = [valid").unwrap();
        assert!(run(&mut m, &["add", "X", "/"]).unwrap_err().contains("does not load"));
        assert_eq!(names(&m), ["Docs", "Old"]);
    }
}
