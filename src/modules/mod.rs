//! Features, one directory each. A module depends on `crate::core` and `crate::platform`,
//! never on another module (layer rule `modules-are-independent`); it talks to others only
//! through item ids and view names. Adding one is a directory plus one line in `modules!`.
//!
//! Config: each module reads its own `[<module id>]` table in `Module::configure`, into a
//! settings type private to the module. `enabled = false` in that table leaves the module
//! out of the registry, so it adds no items, views or hotkeys.

use crate::config::{Config, Section};
use crate::core::{Module, Registry};

/// Declares each module directory and registers it, one line per module:
/// `[pub] mod <dir> => "<id>", <constructor>;`. The id names the config table and must
/// equal `Module::id()`. `$apps` names the scanned app list for constructors that take it.
macro_rules! modules {
    (fn add_all($m:ident, $apps:ident) {
        $($vis:vis mod $dir:ident => $id:literal, $new:expr;)*
    }) => {
        $($vis mod $dir;)*

        /// Every registered module id, in registration order.
        #[cfg(test)]
        pub const IDS: &[&str] = &[$($id),*];

        fn add_all($m: &mut Modules, $apps: Vec<apps::App>) -> Result<(), String> {
            $($m.add($id, $new)?;)*
            Ok(())
        }
    };
}

// Order matters twice: root items keep it for equal ranks (apps, window commands,
// quicklinks, builtins), and hotkeys bind in it (desktop toggle, window switcher, then
// window keys). Item ids feed the `usage` table, so their format is pinned by
// `crate::characterization`.
modules! {
    fn add_all(m, apps) {
        pub mod apps => "app", || apps::Apps::new(apps);
        mod desktop => "desktop", desktop::Desktop::default;
        pub mod switcher => "switcher", switcher::Switcher::default;
        pub mod windows => "window", windows::Windows::default;
        pub mod quicklinks => "quicklink", quicklinks::Quicklinks::default;
        mod scripts => "script", scripts::Scripts::default;
        mod flick => "builtin", || flick::Flick;
        mod clipboard => "clip", || clipboard::Clipboard;
        mod activity => "activity", activity::Activity::default;
        mod herdr => "herdr", herdr::Herdr::default;
        mod kota => "kota", kota::Kota::default;
        mod llm => "llm", llm::Llm::default;
        mod sys => "sys", || sys::Sys::new(crate::cli::PEER);
        mod tasks => "task", tasks::Tasks::default;
        mod keys => "keys", || keys::Keys::new(crate::control::local);
        mod dictation => "dictation", dictation::Dictation::default;
        mod rebuild => "flick", rebuild::Rebuild::default;
        mod capture => "capture", capture::Capture::default;
        mod feedback => "feedback", feedback::Feedback::default;
        mod message => "message", message::Inbox::default;
        mod remote => "remote", || remote::Remote::new(crate::control::net::HOOKS);
        mod help => "help", help::Help::default;
    }
}

// Not registered yet: its `modules!` line lands with the module itself (flick-3ffa).
mod bridge;

#[cfg(test)]
pub use clipboard::store::Clips;

/// Every enabled module over `apps`, in `modules!` order. Errors name the bad table.
pub fn with_apps(config: &Config, apps: Vec<apps::App>) -> Result<Registry, String> {
    let mut m = Modules { config, list: vec![], skipped: None };
    add_all(&mut m, apps)?;
    Ok(Registry::new(m.list))
}

/// Every enabled module under `config`, with the app index scanned now, for startup.
/// Unlike `with_apps`, a bad module table does not stop the others. That module runs
/// on its defaults, as if the file had no such table. Returns one error per bad table.
pub fn lenient(config: &Config) -> (Registry, Vec<String>) {
    lenient_with_apps(config, apps::scan())
}

/// `lenient` over `apps`.
pub fn lenient_with_apps(config: &Config, apps: Vec<apps::App>) -> (Registry, Vec<String>) {
    let mut m = Modules { config, list: vec![], skipped: Some(vec![]) };
    // `add` fails only when `skipped` is `None`.
    let _ = add_all(&mut m, apps);
    (Registry::new(m.list), m.skipped.unwrap_or_default())
}

/// Each module's `flick help` verb line (`Module::verbs`), in registration order.
pub fn verbs() -> Vec<&'static str> {
    with_apps(&Config::default(), vec![]).map_or_else(
        |_| vec![],
        |r| r.into_modules().iter().map(|m| m.verbs()).filter(|v| !v.is_empty()).collect(),
    )
}

/// `registry` under a reloaded `config`, unchanged on error. Modules still enabled keep
/// their state (recent apps, the app index) and take their new table; newly enabled ones
/// start fresh. Returns the ids of the fresh ones, which the caller sends `Event::Started`.
pub fn reload(registry: &mut Registry, config: &Config) -> Result<Vec<&'static str>, String> {
    let fresh = with_apps(config, vec![])?.into_modules();
    let mut old = std::mem::replace(registry, Registry::new(vec![])).into_modules();
    let mut started = vec![];
    let list = fresh
        .into_iter()
        .map(|new| {
            if let Some(i) = old.iter().position(|m| m.id() == new.id()) {
                let mut kept = old.swap_remove(i);
                // `fresh` already took this table, so it cannot fail here.
                if let Ok(Some(table)) = config.section(kept.id())
                    && kept.configure(&table).is_ok()
                {
                    return kept;
                }
            }
            started.push(new.id());
            new
        })
        .collect();
    *registry = Registry::new(list);
    Ok(started)
}

struct Modules<'a> {
    config: &'a Config,
    list: Vec<Box<dyn Module>>,
    /// `Some` when lenient: the errors of the bad tables skipped so far.
    skipped: Option<Vec<String>>,
}

impl Modules<'_> {
    /// Build module `id` and configure it from its table, unless the table sets
    /// `enabled = false`. When lenient, a bad table is recorded and the module configured
    /// from an empty one instead.
    fn add<M: Module + 'static>(
        &mut self,
        id: &str,
        new: impl FnOnce() -> M,
    ) -> Result<(), String> {
        let table = match self.config.section(id) {
            Ok(None) => return Ok(()),
            Ok(Some(table)) => table,
            Err(e) => self.skip(id, e)?,
        };
        let mut module = new();
        if let Err(e) = module.configure(&table) {
            module.configure(&self.skip(id, e)?)?;
        }
        debug_assert_eq!(module.id(), id, "table name is the module id");
        self.list.push(Box::new(module));
        Ok(())
    }

    /// Error `e` in module `id`'s table: `Err(e)`, or when lenient the empty table to use
    /// instead.
    fn skip(&mut self, id: &str, e: String) -> Result<Section, String> {
        let Some(skipped) = self.skipped.as_mut() else { return Err(e) };
        skipped.push(e);
        Ok(Section::empty(id))
    }
}

/// Opens flick.db as the app does: core's tables, then every module's.
#[cfg(test)]
pub struct AppStore;

#[cfg(test)]
impl AppStore {
    pub fn open(path: &std::path::Path) -> Result<crate::core::store::Store, String> {
        let store = crate::core::store::Store::open(path).map_err(|e| e.to_string())?;
        with_apps(&Config::default(), vec![])?.migrate(&store)?;
        Ok(store)
    }

    pub fn in_memory() -> crate::core::store::Store {
        let store = crate::core::store::Store::in_memory();
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
        // Only the modules this test knows; a new module's hotkeys are its own business.
        let owners = |r: Registry| {
            let known = |m: &&str| ["desktop", "switcher", "window"].contains(m);
            r.hotkeys().into_iter().map(|(m, _)| m).filter(known).collect::<Vec<_>>()
        };
        assert_eq!(owners(registry(text).unwrap()), ["desktop", "switcher"]);
        let off = format!("{text}\nenabled = false\n[desktop]\nenabled = false");
        assert!(owners(registry(&off).unwrap()).is_empty());
        let all = IDS.iter().map(|id| format!("[{id}]\nenabled = false")).collect::<Vec<_>>();
        let all = all.join("\n");
        test_cx("", |cx| assert!(registry(&all).unwrap().items(cx).is_empty()));
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
        assert!(reload(&mut r, &next).unwrap().is_empty());
        assert!(ids(&mut r).contains(&"app:/Applications/Safari.app".to_string()));
        assert_eq!(specs(&r), ["cmd+K"]);
        assert!(reload(&mut r, &parse("[switcher]\nhotkey = 1").unwrap()).is_err());
        assert_eq!(specs(&r), ["cmd+K"]);
        // Re-enabled `clip` starts fresh; the caller sends it `Started`.
        assert_eq!(reload(&mut r, &parse("[app]\nenabled = false").unwrap()).unwrap(), ["clip"]);
        assert!(!ids(&mut r).iter().any(|id| id.starts_with("app:")));
        assert_eq!(reload(&mut r, &parse("").unwrap()).unwrap(), ["app"]);
        assert!(ids(&mut r).iter().any(|id| id.starts_with("window:")));
        // A fresh `app` has no index until `Started` scans one.
        assert!(!ids(&mut r).iter().any(|id| id.starts_with("app:")));
        test_cx("", |cx| r.dispatch_to(&["app"], crate::core::Event::Started, cx));
        assert!(ids(&mut r).iter().any(|id| id.starts_with("app:")));
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

    #[test]
    fn lenient_skips_only_the_bad_tables() {
        let text = "[[quicklinks]]\nname = \"No URL\"\n[window]\nenabled = 1\n\
                    [switcher]\nhotkey = \"cmd+J\"\n[desktop]\nenabled = false";
        let (mut r, errors) = lenient_with_apps(&parse(text).unwrap(), vec![]);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(errors[0], "[window] enabled: expected true or false");
        assert!(errors[1].starts_with("[quicklink]: missing field `url`"), "{errors:?}");
        // The good tables still apply: switcher keeps its hotkey, desktop stays off.
        let specs = r.hotkeys().into_iter().map(|(m, b)| (m, b.spec)).collect::<Vec<_>>();
        assert!(specs.contains(&("switcher", "cmd+J".into())), "{specs:?}");
        assert!(!specs.iter().any(|(m, _)| *m == "desktop"));
        // The bad tables' modules run on their defaults: window commands, no quicklinks.
        let items = test_cx("", |cx| r.items(cx));
        assert!(items.iter().any(|i| i.id.module() == "window"));
        assert!(!items.iter().any(|i| i.id.module() == "quicklink"));
        assert!(lenient_with_apps(&parse("").unwrap(), vec![]).1.is_empty());
    }
}
