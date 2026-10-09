//! Module `window`: window commands in root search and on global hotkeys, through the
//! Accessibility API. Ids are `window:<title>`; hotkey keys are action slugs.

pub mod action;

use action::{WindowAction, frame_for};

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::config::Section;
use crate::core::{Binding, Cx, Icon, Item, ItemId, ListView, Module, Outcome, unknown_verb};
use crate::platform::{ax, screens, workspace};

pub fn apply(action: WindowAction) -> Result<(), &'static str> {
    // Hide acts on the app, like cmd+H: instant, and needs no Accessibility permission.
    if action == WindowAction::Hide {
        return workspace::hide_frontmost();
    }
    if !ax::ensure_trusted() {
        return Err("Flick needs Accessibility permission");
    }
    let win = ax::focused_window().ok_or("No focused window")?;
    if action == WindowAction::Minimize {
        win.minimize();
        return Ok(());
    }
    let current = win.frame().ok_or("Can't read window frame")?;
    let target = frame_for(action, current, &screens::visible_areas())?;
    win.set_frame(target);
    Ok(())
}

/// `apply`, logging a failure (there is no panel to show it in).
fn run(action: WindowAction) {
    if let Err(e) = apply(action) {
        eprintln!("flick: {}: {e}", action.title());
    }
}

/// Table `[window]`: `[window.keys]`, `<action slug> = "<hotkey>"` (legacy: top-level
/// `[window_keys]`).
#[derive(Default, Deserialize)]
#[serde(default)]
struct Settings {
    keys: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct Windows {
    keys: BTreeMap<String, String>,
}

impl Module for Windows {
    fn id(&self) -> &'static str {
        "window"
    }

    fn configure(&mut self, table: &Section) -> Result<(), String> {
        self.keys = table.get::<Settings>()?.keys;
        Ok(())
    }

    fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
        WindowAction::ALL
            .iter()
            .map(|&w| Item {
                subtitle: "Window Management".into(),
                accessory: "Command".into(),
                keywords: vec!["window".into()],
                ..Item::new(
                    ItemId::new("window", w.title()),
                    w.title(),
                    "Move Window",
                    Icon::Symbol(w.symbol()),
                )
            })
            .collect()
    }

    fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        let Some(action) = WindowAction::ALL.into_iter().find(|w| w.title() == id.key()) else {
            return Outcome::Stay(None);
        };
        cx.hide();
        run(action);
        Outcome::Hide
    }

    /// `keys`, in key order.
    fn hotkeys(&self) -> Vec<Binding> {
        self.keys
            .iter()
            .map(|(name, spec)| Binding {
                spec: spec.clone(),
                key: WindowAction::from_slug(name)
                    .map(WindowAction::slug)
                    .ok_or(format!("Unknown window action \"{name}\"")),
            })
            .collect()
    }

    /// A window action without the panel.
    fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
        if let Some(action) = WindowAction::from_slug(key) {
            run(action);
        }
        None
    }

    fn verbs(&self) -> &'static str {
        "window list | window <action>, e.g. window left-half"
    }

    /// `list`: every action slug. `<slug>`: apply it to the focused window.
    fn command(&mut self, args: &[String], _cx: &mut Cx) -> Result<String, String> {
        match args {
            [verb] if verb == "list" => {
                Ok(WindowAction::ALL.iter().map(|a| a.slug()).collect::<Vec<_>>().join("\n"))
            }
            [slug] => match WindowAction::from_slug(slug) {
                Some(action) => {
                    apply(action).map(|()| String::new()).map_err(|e| format!("window: {e}"))
                }
                None => Err(unknown_verb("window", args)),
            },
            _ => Err(unknown_verb("window", args)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_list_actions_and_reject_unknown_ones() {
        crate::core::test_cx("", |cx| {
            let list = Windows::default().command(&["list".into()], cx).unwrap();
            assert_eq!(list.lines().count(), WindowAction::ALL.len());
            assert!(list.lines().any(|l| l == "left-half"));
            let err = Windows::default().command(&["sideways".into()], cx).unwrap_err();
            assert_eq!(err, "window: unknown command \"sideways\"");
            assert!(Windows::default().command(&["left-half".into(), "x".into()], cx).is_err());
        });
    }

    #[test]
    fn window_keys_bind_action_slugs() {
        let text =
            "[window.keys]\nleft-half = \"ctrl+alt+ArrowLeft\"\nsideways = \"ctrl+alt+KeyS\"";
        let config = crate::config::parse(text).unwrap();
        let mut windows = Windows::default();
        windows.configure(&config.section("window").unwrap().unwrap()).unwrap();
        assert_eq!(
            windows.hotkeys(),
            [
                Binding { spec: "ctrl+alt+ArrowLeft".into(), key: Ok("left-half".into()) },
                Binding {
                    spec: "ctrl+alt+KeyS".into(),
                    key: Err("Unknown window action \"sideways\"".into())
                },
            ]
        );
    }
}
