//! Global hotkey bindings: the launcher toggle, and each module's hotkeys. Feature-blind:
//! the controller supplies the bindings and receives the presses.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::platform::hotkeys::{self, Hotkey};

/// What a hotkey runs.
#[derive(Clone)]
pub enum Target {
    /// Show root search, or hide the launcher.
    Launcher,
    /// Module `.0`'s hotkey `.1`.
    Module(&'static str, String),
}

thread_local! {
    /// Registered hotkeys by id.
    static BOUND: RefCell<HashMap<u32, (Hotkey, Target)>> = RefCell::new(HashMap::new());
}

pub fn init() -> Result<(), String> {
    hotkeys::init(dispatch)
}

fn dispatch(id: u32) {
    let target = BOUND.with(|b| b.borrow().get(&id).map(|(_, t)| t.clone()));
    match target {
        Some(Target::Launcher) => crate::app::toggle(),
        Some(Target::Module(module, key)) => crate::app::hotkey(module, &key),
        None => {}
    }
}

/// Replace all hotkeys with `wanted`, `(spec, target)` pairs bound in order. Registers what
/// it can and reports the rest.
pub fn register(wanted: Vec<(String, Result<Target, String>)>) -> Result<(), String> {
    if !hotkeys::is_initialized() {
        return Err("Hotkey manager not initialized".into());
    }
    BOUND.with(|bound| {
        let mut bound = bound.borrow_mut();
        for (_, (hotkey, _)) in bound.drain() {
            hotkeys::unregister(hotkey);
        }
        let mut errors = vec![];
        for (spec, target) in wanted {
            let result = target.and_then(|target| {
                let hotkey: Hotkey =
                    spec.parse().map_err(|e| format!("Bad hotkey \"{spec}\": {e}"))?;
                if bound.contains_key(&hotkey.id()) {
                    return Err(format!("\"{spec}\" is bound twice"));
                }
                hotkeys::register(hotkey).map_err(|e| format!("Can't register \"{spec}\": {e}"))?;
                bound.insert(hotkey.id(), (hotkey, target));
                Ok(())
            });
            if let Err(e) = result {
                errors.push(e);
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    })
}
