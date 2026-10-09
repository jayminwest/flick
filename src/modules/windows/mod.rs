//! Module `window`: window commands in root search and on global hotkeys, through the
//! Accessibility API. Ids are `window:<title>`; hotkey keys are action slugs.

pub mod action;

use action::{WindowAction, frame_for};

use crate::config::Config;
use crate::core::{Binding, Cx, Icon, Item, ItemId, ListView, Module, Outcome};
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

pub struct Windows;

impl Module for Windows {
    fn id(&self) -> &'static str {
        "window"
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

    /// `[window_keys]`: `<action slug> = "<hotkey>"`, in key order.
    fn hotkeys(&self, config: &Config) -> Vec<Binding> {
        config
            .window_keys
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_keys_bind_action_slugs() {
        let mut config = Config::default();
        config.window_keys.insert("left-half".into(), "ctrl+alt+ArrowLeft".into());
        config.window_keys.insert("sideways".into(), "ctrl+alt+KeyS".into());
        assert_eq!(
            Windows.hotkeys(&config),
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
