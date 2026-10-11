//! Global hotkeys via Carbon (through `global-hotkey`).

use std::cell::RefCell;
use std::str::FromStr;

use global_hotkey::hotkey::{HotKey, Modifiers};
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
    /// While the control layer is on: each layer hotkey, and the id of the hotkey it fires.
    static LAYER: RefCell<Vec<(HotKey, u32)>> = const { RefCell::new(Vec::new()) };
}

/// `CGEventFlags` bits, as the keys engine names hyper mods.
const FLAG_SHIFT: u64 = 0x2_0000;
const FLAG_CTRL: u64 = 0x4_0000;
const FLAG_ALT: u64 = 0x8_0000;
const FLAG_CMD: u64 = 0x10_0000;

/// Start listening; `on_press` gets the id of each pressed hotkey, on the main thread.
pub fn init(on_press: fn(u32)) -> Result<(), String> {
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    // Carbon delivers hotkey events on the main thread.
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        if e.state == HotKeyState::Pressed {
            on_press(layer_target(e.id).unwrap_or(e.id));
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

/// Turn the control layer on or off; main thread only. While on, each registered hotkey whose
/// modifiers are exactly `hyper` (`CGEventFlags`, e.g. cmd+ctrl+alt+shift) also fires for
/// ctrl plus its key. Caps Lock sends Right Control, so under secure input, when the key tap
/// cannot add the hyper flags, Caps+H still arrives as ctrl+H and fires the hyper+H binding
/// (flick-1271). Plain ctrl+H works as usual while the layer is off. A layer hotkey that
/// would take a registered hotkey's place is left out. Idempotent.
pub fn set_ctrl_layer(on: bool, hyper: u64) {
    MANAGER.with(|m| {
        let m = m.borrow();
        let Some(m) = m.as_ref() else { return };
        LAYER.with(|layer| {
            let mut layer = layer.borrow_mut();
            for (hotkey, _) in layer.drain(..) {
                let _ = m.unregister(hotkey);
            }
            if on {
                let wanted = REGISTERED.with(|r| layer_for(&r.borrow(), modifiers(hyper)));
                layer.extend(wanted.into_iter().filter(|(h, _)| m.register(*h).is_ok()));
            }
        });
    });
}

/// The hotkey a pressed layer hotkey stands in for.
fn layer_target(id: u32) -> Option<u32> {
    LAYER.with(|l| l.borrow().iter().find(|(h, _)| h.id() == id).map(|&(_, target)| target))
}

/// Layer hotkeys for `registered`: ctrl plus the key of each hotkey with exactly `hyper`
/// modifiers, unless that ctrl hotkey is itself registered.
fn layer_for(registered: &[HotKey], hyper: Modifiers) -> Vec<(HotKey, u32)> {
    registered
        .iter()
        .filter(|h| h.mods == hyper && hyper != Modifiers::CONTROL)
        .map(|h| (HotKey::new(Some(Modifiers::CONTROL), h.key), h.id()))
        .filter(|(l, _)| registered.iter().all(|r| r.id() != l.id()))
        .collect()
}

/// `CGEventFlags` modifier bits as hotkey modifiers.
fn modifiers(flags: u64) -> Modifiers {
    [
        (FLAG_CMD, Modifiers::SUPER),
        (FLAG_CTRL, Modifiers::CONTROL),
        (FLAG_ALT, Modifiers::ALT),
        (FLAG_SHIFT, Modifiers::SHIFT),
    ]
    .into_iter()
    .filter(|&(bit, _)| flags & bit != 0)
    .fold(Modifiers::empty(), |acc, (_, m)| acc | m)
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

    #[test]
    fn layer_maps_hyper_hotkeys_to_ctrl() {
        let hk = |spec: &str| spec.parse::<Hotkey>().unwrap().0;
        let hyper = modifiers(FLAG_CMD | FLAG_CTRL | FLAG_ALT | FLAG_SHIFT);
        let registered = [
            hk("cmd+ctrl+alt+shift+KeyH"),
            hk("cmd+KeyL"),
            hk("cmd+ctrl+alt+shift+Space"),
            hk("ctrl+Space"),
        ];
        let layer = layer_for(&registered, hyper);
        // ctrl+Space is the launcher's own hotkey, so hyper+Space gets no layer entry.
        assert_eq!(layer, vec![(hk("ctrl+KeyH"), registered[0].id())]);
        assert_eq!(layer_for(&[hk("ctrl+KeyH")], Modifiers::CONTROL), vec![]);
        assert_eq!(modifiers(0), Modifiers::empty());
        assert_eq!(layer_target(1), None);
    }
}
