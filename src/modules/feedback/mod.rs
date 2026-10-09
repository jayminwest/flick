//! Module `feedback`: quick notes about Flick, appended to a local JSON Lines file
//! (`entry.rs`; default `feedback.jsonl` in the checkout this Flick was built from, which
//! the repo's .gitignore lists). Ids are `feedback:new` (Add Feedback…, form `new`),
//! `feedback:list` (Recent Feedback, view `recent`), `feedback:save` (the `<keyword> <text>`
//! row and the `write` view's row; the text rides in the id's `arg`) and `feedback:entry/<n>`
//! (rows of `recent`, which do not record use). Table `[feedback]`: `file` (default the
//! checkout's), `keyword` (default `fb`), `hotkey` (unbound by default) opens view `write`.

mod entry;
#[cfg(test)]
mod tests;
mod wire;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Section;
use crate::core::{Binding, Cx, Event, Field, Form, Icon, Item, ItemId, ListView, Module, Outcome};
use crate::core::unknown_verb;
use entry::Entry;
use wire::{App, Env};

/// Rows in Recent Feedback.
const RECENT: usize = 50;
/// `flick feedback ls` without `--limit`.
const LS_LIMIT: usize = 20;
const ENTRY: &str = "entry/";

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    /// The file; empty means `feedback.jsonl` in the checkout.
    file: String,
    /// `<keyword> <text>` in root search saves `text`.
    keyword: String,
    /// Opens view `write`.
    hotkey: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { file: String::new(), keyword: "fb".into(), hotkey: None }
    }
}

#[derive(Default)]
pub struct Feedback {
    env: Env,
    settings: Settings,
    /// The app in front when the launcher (or the hotkey view) last opened.
    front: Option<App>,
    /// The root search text when "Add Feedback…" opened the form.
    query: Option<String>,
}

/// The JSON answer of `add`.
#[derive(Serialize)]
struct Saved<'a> {
    path: String,
    entry: &'a Entry,
}

impl Feedback {
    fn path(&self) -> Result<PathBuf, String> {
        entry::resolve(&self.settings.file, self.env.source.as_deref(), self.env.home.as_deref())
    }

    /// Append `text` with the current context; `launcher` adds the front app.
    fn save(&self, text: &str, launcher: bool, query: Option<String>) -> Result<(PathBuf, Entry), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Feedback is empty".into());
        }
        let path = self.path()?;
        let now = (self.env.now)();
        let front = self.front.clone().filter(|_| launcher);
        let (bundle_id, app) = front.map_or((None, None), |(id, name)| (id, Some(name)));
        let entry = Entry {
            ts: entry::rfc3339(now, (self.env.utc_offset)(now)),
            text: text.into(),
            build: self.env.build.clone(),
            app: app.filter(|a| !a.is_empty()),
            bundle_id,
            query: query.filter(|q| !q.trim().is_empty()),
        };
        entry::append(&path, &entry)?;
        Ok((path, entry))
    }

    /// Save from the launcher: hide on success, a status line on failure.
    fn save_and_hide(&self, text: &str, cx: &mut Cx) -> Outcome {
        match self.save(text, true, None) {
            Ok(_) => {
                cx.hide();
                Outcome::Hide
            }
            Err(e) => Outcome::Stay(Some(e)),
        }
    }

    fn save_item(&self, text: &str) -> Item {
        Item {
            subtitle: self.subtitle(),
            ..Item::new(
                ItemId::new("feedback", "save").with_arg(text),
                format!("Save feedback: {text}"),
                "Save Feedback",
                Icon::Symbol("text.bubble"),
            )
        }
    }

    fn subtitle(&self) -> String {
        self.path().map_or_else(|e| e, |_| "Feedback".into())
    }

    fn recent(&self, cx: &mut Cx) -> Vec<Item> {
        let entries = entry::read(&self.path().unwrap_or_default(), RECENT).unwrap_or_default();
        let items: Vec<Item> = entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| Item {
                subtitle: [Some(e.ts.as_str()), e.app.as_deref()].into_iter().flatten().collect::<Vec<_>>().join("  ·  "),
                ..Item::new(
                    ItemId::new("feedback", format!("{ENTRY}{i}")).with_arg(e.text.clone()),
                    e.text,
                    "Copy Text",
                    Icon::Symbol("text.bubble"),
                )
            })
            .collect();
        // Keep newest first for equal scores.
        let n = items.len();
        cx.ranker.rank(cx.query, items, |i| {
            let pos = i.id.key().strip_prefix(ENTRY).and_then(|k| k.parse::<usize>().ok()).unwrap_or(n);
            -(pos as f64) * 1e-3
        })
    }

    /// `add <text...>`, `ls [--limit n]`, `path`.
    fn run_verb(&self, args: &[String], json: bool) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["add", text @ ..] if !text.is_empty() => {
                let (path, entry) = self.save(&text.join(" "), false, None)?;
                if json {
                    let saved = Saved { path: path.display().to_string(), entry: &entry };
                    return serde_json::to_string(&saved).map_err(|e| e.to_string());
                }
                Ok(format!("Feedback saved to {}", path.display()))
            }
            ["add"] => Err("usage: flick feedback add <text...>".into()),
            ["ls", rest @ ..] => {
                let limit = match rest {
                    [] => LS_LIMIT,
                    ["--limit", n] => n.parse().map_err(|_| format!("--limit {n}: not a number"))?,
                    _ => return Err("usage: flick feedback ls [--limit n]".into()),
                };
                let entries = entry::read(&self.path()?, limit)?;
                if json {
                    return serde_json::to_string(&entries).map_err(|e| e.to_string());
                }
                Ok(entries.iter().map(|e| format!("{}\t{}", e.ts, e.text.replace('\n', " "))).collect::<Vec<_>>().join("\n"))
            }
            ["path"] => self.path().map(|p| p.display().to_string()),
            _ => Err(unknown_verb("feedback", args)),
        }
    }
}

/// View `write` (the hotkey): the search field is the feedback.
fn write_view() -> ListView {
    ListView {
        placeholder: "Type feedback, ↵ to save…".into(),
        footer: "Feedback  ·  esc to close".into(),
        empty: "Type feedback, then ↵".into(),
        escape_hides: true,
        ..ListView::new("feedback", "write")
    }
}

impl Module for Feedback {
    fn id(&self) -> &'static str {
        "feedback"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        let settings = table.get::<Settings>()?;
        if settings.keyword.trim().is_empty() || settings.keyword.contains(' ') {
            return Err("[feedback]: keyword must be one word".into());
        }
        self.settings = settings;
        Ok(())
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        vec![
            Item {
                subtitle: self.subtitle(),
                accessory: "Command".into(),
                keywords: vec!["note".into(), "bug".into(), self.settings.keyword.clone()],
                ..Item::new(ItemId::new("feedback", "new"), "Add Feedback…", "Write Feedback", Icon::Symbol("text.bubble"))
            },
            Item {
                subtitle: "Feedback".into(),
                accessory: "Command".into(),
                ..Item::new(ItemId::new("feedback", "list"), "Recent Feedback", "Open", Icon::Symbol("list.bullet"))
            },
        ]
    }

    /// "<keyword> <text>" saves `text` on Enter.
    fn direct(&mut self, cx: &mut Cx) -> Vec<Item> {
        let Some((keyword, rest)) = cx.query.split_once(' ') else { return vec![] };
        if keyword != self.settings.keyword || rest.trim().is_empty() {
            return vec![];
        }
        vec![self.save_item(rest.trim())]
    }

    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        match view {
            "recent" => Some(ListView {
                placeholder: "Search feedback…".into(),
                footer: "Recent Feedback  ·  ↵ copies".into(),
                empty: "No feedback yet".into(),
                ..ListView::new("feedback", "recent")
            }),
            "write" => Some(write_view()),
            _ => None,
        }
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        view.items = match view.name.as_str() {
            "recent" => {
                if let Err(e) = self.path() {
                    view.empty = e;
                }
                self.recent(cx)
            }
            "write" if !cx.query.trim().is_empty() => vec![self.save_item(cx.query.trim())],
            _ => vec![],
        };
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        match (id.key(), id.arg()) {
            ("new", _) => {
                self.query = Some(cx.query.to_string());
                Outcome::Form { module: "feedback", name: "new".into() }
            }
            ("list", _) => Outcome::Push(ListView::new("feedback", "recent")),
            ("save", Some(text)) => self.save_and_hide(text, cx),
            (key, Some(text)) if key.starts_with(ENTRY) => {
                (self.env.copy)(text);
                Outcome::Stay(Some("Copied feedback".into()))
            }
            _ => Outcome::Stay(None),
        }
    }

    fn form(&mut self, name: &str, _cx: &mut Cx) -> Option<Form> {
        (name == "new").then(|| Form {
            fields: vec![
                Field::new("text", "Feedback").required().placeholder("What worked, what got in the way…"),
            ],
            submit_label: "Save Feedback".into(),
            ..Form::new("feedback", "new", "Add Feedback")
        })
    }

    fn submit(&mut self, form: &Form, _cx: &mut Cx) -> Result<String, String> {
        let text = form.value("text").unwrap_or_default();
        self.save(text, true, self.query.clone())?;
        self.query = None;
        Ok("Feedback saved".into())
    }

    fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
        if event == Event::LauncherOpened {
            self.front = (self.env.frontmost)();
        }
        false
    }

    fn hotkeys(&self) -> Vec<Binding> {
        let spec = self.settings.hotkey.as_ref().filter(|s| !s.trim().is_empty());
        spec.map(|spec| Binding { spec: spec.clone(), key: Ok("write".into()) }).into_iter().collect()
    }

    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        (key == "write").then(|| {
            self.front = (self.env.frontmost)();
            write_view()
        })
    }

    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        self.run_verb(args, cx.json)
    }

    fn verbs(&self) -> &'static str {
        "feedback add <text...> | feedback ls [--limit n] | feedback path"
    }
}
