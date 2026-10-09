//! Global hotkey via Carbon (through `global-hotkey`).

use std::cell::RefCell;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

thread_local! {
    static HOTKEY: RefCell<Option<(GlobalHotKeyManager, Option<HotKey>)>> = const { RefCell::new(None) };
}

pub fn init() -> Result<(), String> {
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    // Carbon delivers hotkey events on the main thread.
    GlobalHotKeyEvent::set_event_handler(Some(|e: GlobalHotKeyEvent| {
        if e.state == HotKeyState::Pressed {
            crate::app::toggle();
        }
    }));
    HOTKEY.with(|h| *h.borrow_mut() = Some((manager, None)));
    Ok(())
}

/// Replace the current hotkey with `spec`, e.g. "alt+Space".
pub fn register(spec: &str) -> Result<(), String> {
    let hotkey: HotKey = spec.parse().map_err(|e| format!("Bad hotkey \"{spec}\": {e}"))?;
    HOTKEY.with(|h| {
        let mut h = h.borrow_mut();
        let (manager, current) = h.as_mut().ok_or("Hotkey manager not initialized")?;
        if *current == Some(hotkey) {
            return Ok(());
        }
        if let Some(old) = current.take() {
            let _ = manager.unregister(old);
        }
        manager.register(hotkey).map_err(|e| format!("Can't register \"{spec}\": {e}"))?;
        *current = Some(hotkey);
        Ok(())
    })
}
