//! Global hotkey bindings: the launcher toggle, desktop toggle, window switcher and window actions.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::config::Config;
use crate::platform::hotkeys::{self, Hotkey};
use crate::windows::WindowAction;

#[derive(Clone, Copy)]
enum Binding {
    Toggle,
    DesktopToggle,
    Windows,
    Window(WindowAction),
}

thread_local! {
    /// Registered hotkeys by id.
    static BOUND: RefCell<HashMap<u32, (Hotkey, Binding)>> = RefCell::new(HashMap::new());
}

pub fn init() -> Result<(), String> {
    hotkeys::init(dispatch)
}

fn dispatch(id: u32) {
    let binding = BOUND.with(|b| b.borrow().get(&id).map(|(_, b)| *b));
    match binding {
        Some(Binding::Toggle) => crate::app::toggle(),
        Some(Binding::Window(action)) => crate::app::window_action(action),
        Some(Binding::Windows) => crate::app::toggle_windows(),
        Some(Binding::DesktopToggle) => {
            if let Err(e) = crate::spaces::toggle() {
                eprintln!("flick: desktop toggle: {e}");
            }
        }
        None => {}
    }
}

/// Replace all hotkeys with the ones in `config`. Registers what it can and reports the rest.
pub fn register(config: &Config) -> Result<(), String> {
    let mut wanted = vec![(config.hotkey.as_str(), Ok(Binding::Toggle))];
    if let Some(spec) = &config.desktop_toggle {
        wanted.push((spec.as_str(), Ok(Binding::DesktopToggle)));
    }
    if let Some(spec) = &config.windows_hotkey {
        wanted.push((spec.as_str(), Ok(Binding::Windows)));
    }
    for (name, spec) in &config.window_keys {
        let binding = WindowAction::from_slug(name)
            .map(Binding::Window)
            .ok_or(format!("Unknown window action \"{name}\""));
        wanted.push((spec.as_str(), binding));
    }

    if !hotkeys::is_initialized() {
        return Err("Hotkey manager not initialized".into());
    }
    BOUND.with(|bound| {
        let mut bound = bound.borrow_mut();
        for (_, (hotkey, _)) in bound.drain() {
            hotkeys::unregister(hotkey);
        }
        let mut errors = vec![];
        for (spec, binding) in wanted {
            let result = binding.and_then(|binding| {
                let hotkey: Hotkey =
                    spec.parse().map_err(|e| format!("Bad hotkey \"{spec}\": {e}"))?;
                if bound.contains_key(&hotkey.id()) {
                    return Err(format!("\"{spec}\" is bound twice"));
                }
                hotkeys::register(hotkey).map_err(|e| format!("Can't register \"{spec}\": {e}"))?;
                bound.insert(hotkey.id(), (hotkey, binding));
                Ok(())
            });
            if let Err(e) = result {
                errors.push(e);
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    })
}
