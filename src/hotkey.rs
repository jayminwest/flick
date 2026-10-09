//! Global hotkeys via Carbon (through `global-hotkey`): the launcher toggle and window actions.

use std::cell::RefCell;
use std::collections::HashMap;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::config::Config;
use crate::windows::WindowAction;

#[derive(Clone, Copy)]
enum Binding {
    Toggle,
    Window(WindowAction),
}

struct Hotkeys {
    manager: GlobalHotKeyManager,
    bound: HashMap<u32, (HotKey, Binding)>,
}

thread_local! {
    static HOTKEYS: RefCell<Option<Hotkeys>> = const { RefCell::new(None) };
}

pub fn init() -> Result<(), String> {
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    // Carbon delivers hotkey events on the main thread.
    GlobalHotKeyEvent::set_event_handler(Some(|e: GlobalHotKeyEvent| {
        if e.state == HotKeyState::Pressed {
            dispatch(e.id);
        }
    }));
    HOTKEYS.with(|h| *h.borrow_mut() = Some(Hotkeys { manager, bound: HashMap::new() }));
    Ok(())
}

fn dispatch(id: u32) {
    let binding = HOTKEYS.with(|h| h.borrow().as_ref().and_then(|h| h.bound.get(&id).map(|(_, b)| *b)));
    match binding {
        Some(Binding::Toggle) => crate::app::toggle(),
        Some(Binding::Window(action)) => crate::app::window_action(action),
        None => {}
    }
}

/// Replace all hotkeys with the ones in `config`. Registers what it can and reports the rest.
pub fn register(config: &Config) -> Result<(), String> {
    let mut wanted = vec![(config.hotkey.as_str(), Ok(Binding::Toggle))];
    for (name, spec) in &config.window_keys {
        let binding = WindowAction::from_slug(name).map(Binding::Window).ok_or(format!("Unknown window action \"{name}\""));
        wanted.push((spec.as_str(), binding));
    }

    HOTKEYS.with(|h| {
        let mut h = h.borrow_mut();
        let h = h.as_mut().ok_or("Hotkey manager not initialized")?;
        for (_, (hotkey, _)) in h.bound.drain() {
            let _ = h.manager.unregister(hotkey);
        }
        let mut errors = vec![];
        for (spec, binding) in wanted {
            let result = binding.and_then(|binding| {
                let hotkey: HotKey = spec.parse().map_err(|e| format!("Bad hotkey \"{spec}\": {e}"))?;
                if h.bound.contains_key(&hotkey.id()) {
                    return Err(format!("\"{spec}\" is bound twice"));
                }
                h.manager.register(hotkey).map_err(|e| format!("Can't register \"{spec}\": {e}"))?;
                h.bound.insert(hotkey.id(), (hotkey, binding));
                Ok(())
            });
            if let Err(e) = result {
                errors.push(e);
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkey_specs() {
        for spec in ["alt+Space", "cmd+KeyH", "cmd+shift+KeyK", "ctrl+alt+ArrowLeft", "cmd+ctrl+alt+shift+KeyL"] {
            assert!(spec.parse::<HotKey>().is_ok(), "{spec}");
        }
        assert_ne!("cmd+KeyK".parse::<HotKey>().unwrap().id(), "cmd+shift+KeyK".parse::<HotKey>().unwrap().id());
    }
}
