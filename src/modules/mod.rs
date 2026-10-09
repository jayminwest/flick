//! Features, one directory each. A module depends on `crate::core` and `crate::platform`,
//! never on another module (layer rule `modules-are-independent`); it talks to others only
//! through item ids and view names. Adding one is a directory plus one line below.
//!
//! Config: each module reads its own `[<module id>]` table in `Module::configure`, into a
//! settings type private to the module. `enabled = false` in that table leaves the module
//! out of the registry, so it adds no items, views or hotkeys.

pub mod apps;
mod clipboard;
mod desktop;
mod flick;
pub mod quicklinks;
pub mod switcher;
pub mod windows;

use crate::config::Config;
use crate::core::{Module, Registry};
#[cfg(test)]
pub use clipboard::store::Clips;

/// Every enabled module under `config`, with the app index scanned now.
pub fn registry(config: &Config) -> Result<Registry, String> {
    with_apps(config, apps::scan())
}

/// Every enabled module, one line each, over `apps`. Order matters twice: root items keep
/// it for equal ranks (apps, window commands, quicklinks, builtins), and hotkeys bind in it
/// (desktop toggle, window switcher, then window keys). Item ids feed the `usage` table, so
/// their format is pinned by `crate::characterization`. Errors name the bad table.
pub fn with_apps(config: &Config, apps: Vec<apps::App>) -> Result<Registry, String> {
    let mut m = Modules { config, list: vec![] };
    m.add("app", || apps::Apps::new(apps))?;
    m.add("desktop", desktop::Desktop::default)?;
    m.add("switcher", switcher::Switcher::default)?;
    m.add("window", windows::Windows::default)?;
    m.add("quicklink", quicklinks::Quicklinks::default)?;
    m.add("builtin", || flick::Flick)?;
    m.add("clip", || clipboard::Clipboard)?;
    Ok(Registry::new(m.list))
}

/// `registry` under a reloaded `config`, unchanged on error. Modules still enabled keep
/// their state (recent apps, the app index) and take their new table; newly enabled ones
/// start fresh, without `Event::Started`.
pub fn reload(registry: &mut Registry, config: &Config) -> Result<(), String> {
    let fresh = with_apps(config, vec![])?.into_modules();
    let mut old = std::mem::replace(registry, Registry::new(vec![])).into_modules();
    let list = fresh
        .into_iter()
        .map(|new| {
            let Some(i) = old.iter().position(|m| m.id() == new.id()) else { return new };
            let mut kept = old.swap_remove(i);
            // `fresh` already took this table, so it cannot fail here.
            match config.section(kept.id()) {
                Ok(Some(table)) if kept.configure(&table).is_ok() => kept,
                _ => new,
            }
        })
        .collect();
    *registry = Registry::new(list);
    Ok(())
}

struct Modules<'a> {
    config: &'a Config,
    list: Vec<Box<dyn Module>>,
}

impl Modules<'_> {
    /// Build module `id` and configure it from its table, unless the table sets
    /// `enabled = false`.
    fn add<M: Module + 'static>(
        &mut self,
        id: &str,
        new: impl FnOnce() -> M,
    ) -> Result<(), String> {
        if let Some(table) = self.config.section(id)? {
            let mut module = new();
            module.configure(&table)?;
            debug_assert_eq!(module.id(), id, "table name is the module id");
            self.list.push(Box::new(module));
        }
        Ok(())
    }
}

/// Opens flick.db as the app does: core's tables, then every module's.
#[cfg(test)]
pub struct AppStore;

#[cfg(test)]
impl AppStore {
    pub fn open(path: &std::path::Path) -> Result<crate::store::Store, String> {
        let store = crate::store::Store::open(path).map_err(|e| e.to_string())?;
        with_apps(&Config::default(), vec![])?.migrate(&store)?;
        Ok(store)
    }

    pub fn in_memory() -> crate::store::Store {
        let store = crate::store::Store::in_memory();
        with_apps(&Config::default(), vec![])
            .and_then(|r| r.migrate(&store))
            .expect("module migrations run on an empty database");
        store
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;
    use crate::core::test_cx;

    fn registry(text: &str) -> Result<Registry, String> {
        with_apps(&parse(text)?, vec![])
    }

    #[test]
    fn disabled_modules_add_nothing() {
        let text = "desktop_toggle = \"cmd+K\"\n[switcher]\nhotkey = \"cmd+J\"";
        let owners = |r: Registry| r.hotkeys().into_iter().map(|(m, _)| m).collect::<Vec<_>>();
        assert_eq!(owners(registry(text).unwrap()), ["desktop", "switcher"]);
        let off = format!("{text}\nenabled = false\n[desktop]\nenabled = false");
        assert!(owners(registry(&off).unwrap()).is_empty());
        let all = "[app]\nenabled = false\n[window]\nenabled = false\n[quicklink]\nenabled = false\n\
                   [builtin]\nenabled = false\n[clip]\nenabled = false";
        test_cx("", |cx| assert!(registry(all).unwrap().items(cx).is_empty()));
    }

    #[test]
    fn reload_keeps_state_and_takes_new_tables() {
        let app = apps::App { name: "Safari".into(), path: "/Applications/Safari.app".into() };
        let mut r =
            with_apps(&parse("[switcher]\nhotkey = \"cmd+J\"").unwrap(), vec![app]).unwrap();
        let ids = |r: &mut Registry| {
            test_cx("", |cx| r.items(cx)).iter().map(|i| i.id.to_string()).collect::<Vec<_>>()
        };
        let specs = |r: &Registry| r.hotkeys().into_iter().map(|(_, b)| b.spec).collect::<Vec<_>>();
        assert!(ids(&mut r).contains(&"app:/Applications/Safari.app".to_string()));
        let next = parse("[switcher]\nhotkey = \"cmd+K\"\n[clip]\nenabled = false").unwrap();
        reload(&mut r, &next).unwrap();
        assert!(ids(&mut r).contains(&"app:/Applications/Safari.app".to_string()));
        assert_eq!(specs(&r), ["cmd+K"]);
        assert!(reload(&mut r, &parse("[switcher]\nhotkey = 1").unwrap()).is_err());
        assert_eq!(specs(&r), ["cmd+K"]);
        reload(&mut r, &parse("[app]\nenabled = false").unwrap()).unwrap();
        assert!(!ids(&mut r).iter().any(|id| id.starts_with("app:")));
        reload(&mut r, &parse("").unwrap()).unwrap();
        assert!(ids(&mut r).iter().any(|id| id.starts_with("window:")));
    }

    #[test]
    fn a_bad_table_fails_the_whole_registry() {
        let err = registry("[[quicklinks]]\nname = \"No URL\"").err().unwrap();
        assert!(err.starts_with("[quicklink]: missing field `url`"), "{err}");
        assert!(registry("[window]\nenabled = 1").is_err());
        assert!(
            registry("[[quicklinks]]\nname = \"No URL\"\n[quicklink]\nenabled = false").is_ok()
        );
    }
}
