//! Features, one directory each. A module depends on `crate::core` and `crate::platform`,
//! never on another module (layer rule `modules-are-independent`); it talks to others only
//! through item ids and view names. Adding one is a directory plus one line below.

pub mod apps;
mod clipboard;
mod desktop;
mod flick;
mod quicklinks;
pub mod switcher;
pub mod windows;

use crate::core::Registry;
#[cfg(test)]
pub use clipboard::store::Clips;

/// Every module, with the app index scanned now.
pub fn registry() -> Registry {
    with_apps(apps::scan())
}

/// Every module, one line each, over `apps`. Order matters twice: root items keep it for
/// equal ranks (apps, window commands, quicklinks, builtins), and hotkeys bind in it
/// (desktop toggle, window switcher, then `[window_keys]`). Item ids feed the `usage`
/// table, so their format is pinned by `crate::characterization`.
pub fn with_apps(apps: Vec<apps::App>) -> Registry {
    Registry::new(vec![
        Box::new(apps::Apps::new(apps)),
        Box::new(desktop::Desktop::default()),
        Box::new(switcher::Switcher::default()),
        Box::new(windows::Windows),
        Box::new(quicklinks::Quicklinks),
        Box::new(flick::Flick),
        Box::new(clipboard::Clipboard),
    ])
}

/// Opens flick.db as the app does: core's tables, then every module's.
#[cfg(test)]
pub struct AppStore;

#[cfg(test)]
impl AppStore {
    pub fn open(path: &std::path::Path) -> Result<crate::store::Store, String> {
        let store = crate::store::Store::open(path).map_err(|e| e.to_string())?;
        with_apps(vec![]).migrate(&store)?;
        Ok(store)
    }

    pub fn in_memory() -> crate::store::Store {
        let store = crate::store::Store::in_memory();
        with_apps(vec![]).migrate(&store).expect("module migrations run on an empty database");
        store
    }
}
