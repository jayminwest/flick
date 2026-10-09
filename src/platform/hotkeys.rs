//! Global hotkeys via Carbon (through `global-hotkey`).

use std::cell::RefCell;
use std::str::FromStr;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

/// A parsed key combination such as `cmd+shift+KeyK`.
#[derive(Clone, Copy)]
pub struct Hotkey(HotKey);

impl FromStr for Hotkey {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, String> {
        spec.parse::<HotKey>().map(Hotkey).map_err(|e| e.to_string())
    }
}

impl Hotkey {
    /// Equal for equal key combinations; `init`'s handler receives it on a press.
    pub fn id(self) -> u32 {
        self.0.id()
    }
}

thread_local! {
    static MANAGER: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
    /// What `register` registered, for `registered_keys`.
    static REGISTERED: RefCell<Vec<HotKey>> = const { RefCell::new(Vec::new()) };
}

/// Start listening; `on_press` gets the id of each pressed hotkey, on the main thread.
pub fn init(on_press: fn(u32)) -> Result<(), String> {
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    // Carbon delivers hotkey events on the main thread.
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        if e.state == HotKeyState::Pressed {
            on_press(e.id);
        }
    }));
    MANAGER.with(|m| *m.borrow_mut() = Some(manager));
    Ok(())
}

pub fn register(hotkey: Hotkey) -> Result<(), String> {
    MANAGER.with(|m| {
        let m = m.borrow();
        let m = m.as_ref().ok_or("Hotkey manager not initialized")?;
        m.register(hotkey.0).map_err(|e| e.to_string())?;
        REGISTERED.with(|r| r.borrow_mut().push(hotkey.0));
        Ok(())
    })
}

pub fn unregister(hotkey: Hotkey) {
    MANAGER.with(|m| {
        if let Some(m) = m.borrow().as_ref() {
            let _ = m.unregister(hotkey.0);
        }
    });
    REGISTERED.with(|r| r.borrow_mut().retain(|h| h.id() != hotkey.id()));
}

/// The key of each registered hotkey, as hotkey specs name it (`KeyH`, `F18`), modifiers
/// left out.
pub fn registered_keys() -> Vec<String> {
    REGISTERED.with(|r| r.borrow().iter().map(|h| h.key.to_string()).collect())
}

pub fn is_initialized() -> bool {
    MANAGER.with(|m| m.borrow().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkey_specs() {
        for spec in [
            "alt+Space",
            "cmd+KeyH",
            "cmd+shift+KeyK",
            "ctrl+alt+ArrowLeft",
            "cmd+ctrl+alt+shift+KeyL",
            "cmd+Backquote",
        ] {
            assert!(spec.parse::<Hotkey>().is_ok(), "{spec}");
        }
        assert_ne!(
            "cmd+KeyK".parse::<Hotkey>().unwrap().id(),
            "cmd+shift+KeyK".parse::<Hotkey>().unwrap().id()
        );
    }
}
