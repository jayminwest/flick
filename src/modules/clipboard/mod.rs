//! Module `clip`: records copied text, shows the clipboard history view and pastes a clip.
//! Ids are `clip:<row id>`.

pub(super) mod store;

use std::collections::HashMap;

use crate::core::{Cx, Event, Icon, Item, ItemId, ListView, Module, Outcome, unknown_verb};
use crate::platform::{ax, pasteboard, timer};
use crate::store::now;
use store::{Clips, MIGRATIONS};

pub struct Clipboard;

fn relative_time(ts: i64) -> String {
    let secs = (now() - ts).max(0);
    match secs {
        0..60 => "Just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

impl Module for Clipboard {
    fn id(&self) -> &'static str {
        "clip"
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    /// View `history`.
    fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
        (view == "history").then(|| ListView {
            placeholder: "Search clipboard history…".into(),
            footer: "Clipboard History  ·  esc to go back".into(),
            ..ListView::new("clip", view)
        })
    }

    fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        let items: Vec<Item> = cx
            .store
            .clips()
            .iter()
            .map(|c| {
                let mut lines = c.text.trim().lines();
                let first: String = lines.next().unwrap_or("").trim().chars().take(100).collect();
                let more = lines.count();
                Item {
                    subtitle: if more > 0 { format!("+{more} lines") } else { String::new() },
                    accessory: relative_time(c.ts),
                    keywords: vec![c.text.chars().take(2000).collect()],
                    ..Item::new(ItemId::new("clip", c.id), first, "Paste", Icon::Symbol("doc.text"))
                }
            })
            .collect();
        // Bonus keeps newest-first order for equal scores.
        let order: HashMap<String, usize> =
            items.iter().enumerate().map(|(i, item)| (item.id.to_string(), i)).collect();
        view.items = cx.ranker.rank(cx.query, items, |i| -(order[i.id.as_str()] as f64) * 1e-3);
        view.empty =
            if cx.query.is_empty() { "Clipboard history is empty" } else { "No Results" }.into();
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        let Some(text) = id.key().parse().ok().and_then(|id| cx.store.clip_text(id)) else {
            return Outcome::Stay(None);
        };
        pasteboard::set_text(&text);
        cx.hide();
        // Give focus a moment to return to the previous app, then paste there.
        if ax::ensure_trusted() {
            timer::after(0.08, ax::send_paste);
        }
        Outcome::Hide
    }

    /// Record new clipboard text. Skips content that password managers mark as concealed or
    /// transient.
    fn on_event(&mut self, event: Event, cx: &mut Cx) -> bool {
        if event != Event::PasteboardChanged {
            return false;
        }
        let Some(text) = pasteboard::copied_text() else { return false };
        cx.store.add_clip(&text);
        true
    }

    fn verbs(&self) -> &'static str {
        "clip list | clip get <id>"
    }

    /// `list`: `<id>\t<first line>`, newest first. `get <id>`: that clip's full text.
    fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        match args {
            [verb] if verb == "list" => Ok(cx
                .store
                .clips()
                .iter()
                .map(|c| format!("{}\t{}", c.id, c.text.trim().lines().next().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n")),
            [verb, id] if verb == "get" => id
                .parse()
                .ok()
                .and_then(|id| cx.store.clip_text(id))
                .ok_or_else(|| format!("clip: no clip {id}")),
            _ => Err(unknown_verb("clip", args)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_cx;

    #[test]
    fn commands_list_and_get_clips() {
        test_cx("", |cx| {
            cx.store.migrate("clip", MIGRATIONS).unwrap();
            cx.store.add_clip("one\nmore");
            cx.store.add_clip("two");
            let mut run = |w: &[&str]| {
                Clipboard.command(&w.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(), cx)
            };
            let list = run(&["list"]).unwrap();
            assert_eq!(
                list.lines().map(|l| l.split('\t').nth(1).unwrap()).collect::<Vec<_>>(),
                ["two", "one"]
            );
            let id = list.lines().nth(1).unwrap().split('\t').next().unwrap().to_string();
            assert_eq!(run(&["get", &id]).unwrap(), "one\nmore");
            assert_eq!(run(&["get", "x"]).unwrap_err(), "clip: no clip x");
            assert_eq!(run(&["paste"]).unwrap_err(), "clip: unknown command \"paste\"");
        });
    }
}
