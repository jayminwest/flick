//! Module `script`: shell commands in root search, the first step toward script commands
//! (README roadmap). Each `[[script.commands]]` entry has a `name`, a `shell` command line
//! and an optional `keyword`. A command whose `shell` holds `{query}` takes an argument:
//! "<keyword> <text>" in root search runs it at once, and Enter or Tab on it asks for the
//! text. Flick puts the text in single quotes, so it is one shell word whatever it holds.
//! Ids are `script:<name>`; the typed text rides in the id's `arg`.

mod wire;

use serde::Deserialize;

use crate::config::Section;
use crate::core::{Cx, Icon, Item, ItemId, ListView, Module, Outcome};

/// The longest argument, in bytes, that Flick passes to a command.
const MAX_QUERY: usize = 4000;

#[derive(Clone, Debug, Deserialize)]
pub struct Script {
    pub name: String,
    /// Run with `/bin/sh -c`.
    pub shell: String,
    #[serde(default)]
    pub keyword: Option<String>,
}

impl Script {
    fn takes_query(&self) -> bool {
        self.shell.contains("{query}")
    }

    /// The command line for `query`: each `{query}` becomes `query` as one quoted word.
    fn command(&self, query: &str) -> String {
        self.shell.replace("{query}", &quote(query))
    }
}

/// `s` as one `/bin/sh` word: in single quotes, each `'` written as `'\''`.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Table `[script]`: `[[script.commands]]`, unique names and keywords.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Settings {
    commands: Vec<Script>,
}

pub struct Scripts {
    commands: Vec<Script>,
    /// Starts command line `.1` for command `.0` and returns at once. Tests record it.
    run: fn(&str, &str),
}

impl Default for Scripts {
    fn default() -> Self {
        Scripts { commands: vec![], run: wire::run }
    }
}

impl Scripts {
    fn find(&self, name: &str) -> Option<&Script> {
        self.commands.iter().find(|s| s.name == name)
    }
}

/// The table's commands. The name is the item id key and the keyword picks the command for
/// "<keyword> <text>", so both must be unique.
fn read(table: &Section) -> Result<Vec<Script>, String> {
    let commands = table.get::<Settings>()?.commands;
    for (i, s) in commands.iter().enumerate() {
        let name = &s.name;
        let bad = |why: &str| Err(format!("[script] commands: \"{name}\": {why}"));
        if name.trim().is_empty() {
            return Err("[script] commands: a command has no name".into());
        }
        if s.shell.trim().is_empty() {
            return bad("shell is empty");
        }
        if commands[..i].iter().any(|p| p.name == *name) {
            return bad("duplicate name; keep one");
        }
        if let Some(keyword) = &s.keyword {
            if keyword.is_empty() || keyword.contains(char::is_whitespace) {
                return bad("keyword must be one word");
            }
            if commands[..i].iter().any(|p| p.keyword.as_ref() == Some(keyword)) {
                return bad(&format!("keyword \"{keyword}\" is used twice"));
            }
        }
    }
    Ok(commands)
}

fn item(s: &Script, arg: Option<String>) -> Item {
    let id = ItemId::new("script", &s.name);
    let tab = arg.is_none();
    Item {
        accessory: "Script".into(),
        tab,
        ..Item::new(
            match arg {
                Some(arg) => id.with_arg(arg),
                None => id,
            },
            s.name.clone(),
            if tab { "Enter Argument" } else { "Run Command" },
            Icon::Symbol("terminal"),
        )
    }
}

impl Module for Scripts {
    fn id(&self) -> &'static str {
        "script"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.commands = read(table)?;
        Ok(())
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        self.commands
            .iter()
            .map(|s| Item {
                subtitle: s.keyword.clone().unwrap_or_default(),
                keywords: s.keyword.iter().cloned().collect(),
                // A command with no "{query}" runs as is; one with it asks for its argument.
                ..item(s, (!s.takes_query()).then(String::new))
            })
            .collect()
    }

    /// "<keyword> <text>" runs the command with that keyword, if it takes a query.
    fn direct(&mut self, cx: &mut Cx) -> Vec<Item> {
        let Some((keyword, rest)) = cx.query.split_once(' ') else { return vec![] };
        let rest = rest.trim();
        let found = self.commands.iter().find(|s| {
            s.takes_query() && s.keyword.as_deref() == Some(keyword) && !rest.is_empty()
        });
        found
            .map(|s| Item { subtitle: format!("“{rest}”"), ..item(s, Some(rest.to_string())) })
            .into_iter()
            .collect()
    }

    /// View `<name>`: type the argument for that command.
    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        let s = self.find(view)?;
        Some(ListView {
            placeholder: format!("{} argument…", s.name),
            footer: format!("{}  ·  esc to go back", s.name),
            record_use: true,
            ..ListView::new("script", view)
        })
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        view.items = self
            .find(&view.name)
            .map(|s| Item {
                subtitle: if cx.query.is_empty() { "Type an argument".into() } else { s.command(cx.query.trim()) },
                ..item(s, Some(cx.query.to_string()))
            })
            .into_iter()
            .collect();
    }

    fn activate(&mut self, id: &ItemId, _cx: &mut Cx) -> Outcome {
        let Some(s) = self.find(id.key()) else { return Outcome::Stay(None) };
        let Some(query) = id.arg() else {
            return Outcome::Push(ListView::new("script", id.key()));
        };
        let query = query.trim();
        if s.takes_query() && query.is_empty() {
            return Outcome::Stay(None);
        }
        if query.len() > MAX_QUERY {
            return Outcome::Stay(Some(format!("{}: argument over {MAX_QUERY} bytes", s.name)));
        }
        (self.run)(&s.name, &s.command(query));
        Outcome::Hide
    }
}

#[cfg(test)]
mod tests;
