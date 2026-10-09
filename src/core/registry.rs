//! The registered modules, and routing by module id.

use std::panic::{self, AssertUnwindSafe};

use super::store::Store;
use super::{Binding, Cx, Event, Item, ItemId, ListView, Module, Outcome};

pub struct Registry {
    modules: Vec<Box<dyn Module>>,
}

impl Registry {
    /// Modules in root-search order. Ids must be unique.
    pub fn new(modules: Vec<Box<dyn Module>>) -> Registry {
        debug_assert!(
            modules.iter().enumerate().all(|(i, m)| modules[..i].iter().all(|o| o.id() != m.id())),
            "duplicate module id"
        );
        Registry { modules }
    }

    /// The modules in order, to build another registry from (on config reload).
    pub fn into_modules(self) -> Vec<Box<dyn Module>> {
        self.modules
    }

    /// Run every module's store migrations. Stops at the first failure, naming the module.
    pub fn migrate(&self, store: &Store) -> Result<(), String> {
        self.modules.iter().try_for_each(|m| {
            store.migrate(m.id(), m.migrations()).map_err(|e| format!("{}: {e}", m.id()))
        })
    }

    pub(super) fn get(&mut self, id: &str) -> Option<&mut Box<dyn Module>> {
        self.modules.iter_mut().find(|m| m.id() == id)
    }

    /// Every module's root-search items, in registration order.
    pub fn items(&mut self, cx: &mut Cx) -> Vec<Item> {
        self.modules.iter_mut().flat_map(|m| m.items(cx)).collect()
    }

    /// Every module's direct (unranked, shown first) root items, in registration order.
    pub fn direct(&mut self, cx: &mut Cx) -> Vec<Item> {
        self.modules.iter_mut().flat_map(|m| m.direct(cx)).collect()
    }

    /// Open `view` (a request naming the module and view) through its owning module.
    pub fn open(&mut self, view: &ListView, cx: &mut Cx) -> Option<ListView> {
        self.get(view.module)?.open(&view.name, cx)
    }

    pub fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
        if let Some(m) = self.get(view.module) {
            m.refresh(view, cx);
        }
    }

    /// Route `id` to the module that owns it. An unknown module does nothing.
    pub fn activate(&mut self, id: &ItemId, cx: &mut Cx) -> Outcome {
        match self.get(id.module()) {
            Some(m) => m.activate(id, cx),
            None => Outcome::Stay(None),
        }
    }

    /// Send `event` to every module in order. Returns the ids of modules whose views went
    /// stale, which always include the module a `ModuleChanged` names. A module that panics is logged and skipped, so the others still get the event.
    pub fn dispatch(&mut self, event: Event, cx: &mut Cx) -> Vec<&'static str> {
        self.send(event, cx, |_| true)
    }

    /// `dispatch` to modules `ids` only, e.g. `Started` to the modules a reload enabled.
    pub fn dispatch_to(&mut self, ids: &[&str], event: Event, cx: &mut Cx) -> Vec<&'static str> {
        self.send(event, cx, |id| ids.contains(&id))
    }

    fn send(&mut self, event: Event, cx: &mut Cx, to: impl Fn(&str) -> bool) -> Vec<&'static str> {
        let mut stale = vec![];
        for m in self.modules.iter_mut().filter(|m| to(m.id())) {
            match panic::catch_unwind(AssertUnwindSafe(|| m.on_event(event, cx))) {
                Ok(true) => stale.push(m.id()),
                Ok(false) => {}
                Err(_) => eprintln!("flick: module {} panicked handling {event:?}", m.id()),
            }
            if matches!(event, Event::ModuleChanged { module } if module == m.id())
                && !stale.contains(&m.id())
            {
                stale.push(m.id());
            }
        }
        stale
    }

    /// Every module's hotkeys, in registration order, with the owning module.
    pub fn hotkeys(&self) -> Vec<(&'static str, Binding)> {
        self.modules.iter().flat_map(|m| m.hotkeys().into_iter().map(|b| (m.id(), b))).collect()
    }

    /// Run control command `args` (`<module> <verb> [args...]`) on its module. A module
    /// that panics is reported as an error, like one that fails.
    pub fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
        let (module, rest) = args.split_first().ok_or("empty request")?;
        let ids: Vec<&str> = self.modules.iter().map(|m| m.id()).collect();
        let unknown = || format!("unknown module \"{module}\" (modules: {})", ids.join(", "));
        let m = self.modules.iter_mut().find(|m| m.id() == module).ok_or_else(unknown)?;
        panic::catch_unwind(AssertUnwindSafe(|| m.command(rest, cx)))
            .unwrap_or_else(|_| Err(format!("{module}: command panicked")))
    }

    /// Run module `module`'s hotkey `key`. An unknown module does nothing.
    pub fn hotkey(&mut self, module: &str, key: &str, cx: &mut Cx) -> Option<ListView> {
        self.get(module)?.hotkey(key, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Icon, test_cx, unknown_verb};

    /// Records calls; owns view "list".
    struct Toy {
        id: &'static str,
        events: usize,
    }

    impl Module for Toy {
        fn id(&self) -> &'static str {
            self.id
        }

        fn migrations(&self) -> &'static [&'static str] {
            &["CREATE TABLE IF NOT EXISTS toy (a INTEGER);"]
        }

        fn items(&mut self, _cx: &mut Cx) -> Vec<Item> {
            vec![Item::new(ItemId::new(self.id, "one"), "One", "Run", Icon::Symbol("app"))]
        }

        fn open(&mut self, view: &str, _cx: &mut Cx) -> Option<ListView> {
            (view == "list")
                .then(|| ListView { placeholder: "Toy…".into(), ..ListView::new(self.id, view) })
        }

        fn refresh(&mut self, view: &mut ListView, cx: &mut Cx) {
            view.footer = format!("query {}", cx.query);
        }

        fn activate(&mut self, id: &ItemId, _cx: &mut Cx) -> Outcome {
            match id.key() {
                "push" => Outcome::Push(ListView::new(self.id, "list")),
                "stay" => Outcome::Stay(Some("here".into())),
                _ => Outcome::Hide,
            }
        }

        fn direct(&mut self, cx: &mut Cx) -> Vec<Item> {
            (cx.query == "now")
                .then(|| Item::new(ItemId::new(self.id, "now"), "Now", "Run", Icon::Symbol("app")))
                .into_iter()
                .collect()
        }

        fn on_event(&mut self, event: Event, _cx: &mut Cx) -> bool {
            self.events += 1;
            event == Event::PasteboardChanged
        }

        fn hotkeys(&self) -> Vec<Binding> {
            vec![Binding { spec: "cmd+K".into(), key: Ok(format!("{}-key", self.id)) }]
        }

        fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
            (key == "show").then(|| ListView::new(self.id, "list"))
        }

        fn command(&mut self, args: &[String], cx: &mut Cx) -> Result<String, String> {
            match args {
                [verb, rest @ ..] if verb == "echo" => {
                    Ok(format!("{} {}", cx.query, rest.join(" ")))
                }
                _ => Err(unknown_verb(self.id, args)),
            }
        }
    }

    /// Implements nothing but its id.
    struct Bare;

    impl Module for Bare {
        fn id(&self) -> &'static str {
            "bare"
        }
    }

    /// Its one migration is not SQL.
    struct Broken;

    impl Module for Broken {
        fn id(&self) -> &'static str {
            "broken"
        }

        fn migrations(&self) -> &'static [&'static str] {
            &["NOT SQL;"]
        }
    }

    #[test]
    fn migrate_runs_each_modules_migrations_and_names_a_failure() {
        let store = Store::in_memory();
        registry().migrate(&store).unwrap();
        registry().migrate(&store).unwrap();
        let versions: Vec<usize> =
            ["a", "bare", "b"].iter().map(|m| store.version(m).unwrap()).collect();
        assert_eq!(versions, [1, 0, 1]);
        let err = Registry::new(vec![Box::new(Broken)]).migrate(&store).unwrap_err();
        assert!(err.starts_with("broken: "), "{err}");
    }

    fn with_cx<R>(query: &str, f: impl FnOnce(&mut Cx) -> R) -> R {
        test_cx(query, |cx| {
            cx.hide();
            f(cx)
        })
    }

    fn registry() -> Registry {
        Registry::new(vec![
            Box::new(Toy { id: "a", events: 0 }),
            Box::new(Bare),
            Box::new(Toy { id: "b", events: 0 }),
        ])
    }

    #[test]
    fn items_come_in_registration_order() {
        let ids: Vec<String> =
            with_cx("", |cx| registry().items(cx)).into_iter().map(|i| i.id.to_string()).collect();
        assert_eq!(ids, ["a:one", "b:one"]);
    }

    #[test]
    fn activation_routes_by_module() {
        let mut r = registry();
        with_cx("", |cx| {
            assert!(matches!(r.activate(&ItemId::new("b", "x"), cx), Outcome::Hide));
            assert!(
                matches!(r.activate(&ItemId::new("a", "stay"), cx), Outcome::Stay(Some(s)) if s == "here")
            );
            assert!(matches!(r.activate(&ItemId::new("bare", "x"), cx), Outcome::Stay(None)));
            assert!(matches!(r.activate(&ItemId::new("nobody", "x"), cx), Outcome::Stay(None)));
        });
    }

    #[test]
    fn pushed_views_open_and_refresh_through_their_module() {
        let mut r = registry();
        with_cx("hi", |cx| {
            let outcome = r.activate(&ItemId::new("a", "push"), cx);
            assert!(matches!(&outcome, Outcome::Push(v) if v.is("a", "list")));
            let mut opened: Vec<ListView> =
                r.open(&ListView::new("a", "list"), cx).into_iter().collect();
            assert_eq!(opened.len(), 1);
            let view = &mut opened[0];
            assert!(view.is("a", "list"));
            assert_eq!(view.placeholder, "Toy…");
            r.refresh(view, cx);
            assert_eq!(view.footer, "query hi");
            assert!(r.open(&ListView::new("a", "missing"), cx).is_none());
            assert!(r.open(&ListView::new("bare", "list"), cx).is_none());
            assert!(r.open(&ListView::new("nobody", "list"), cx).is_none());
            let mut orphan = ListView::new("nobody", "list");
            r.refresh(&mut orphan, cx);
            let mut bare = ListView::new("bare", "list");
            r.refresh(&mut bare, cx);
            assert!(orphan.footer.is_empty() && bare.footer.is_empty());
        });
    }

    #[test]
    fn events_reach_every_module() {
        let mut toy = Toy { id: "a", events: 0 };
        with_cx("", |cx| {
            let mut bare = Bare;
            assert!(!bare.on_event(Event::LauncherOpened, cx));
            assert!(bare.items(cx).is_empty());
            assert!(!toy.on_event(Event::LauncherOpened, cx));
            let mut r = registry();
            assert!(r.dispatch(Event::Started, cx).is_empty());
            assert_eq!(r.dispatch(Event::PasteboardChanged, cx), ["a", "b"]);
            assert_eq!(r.dispatch_to(&["b", "nobody"], Event::PasteboardChanged, cx), ["b"]);
            assert!(r.dispatch_to(&[], Event::PasteboardChanged, cx).is_empty());
        });
        assert_eq!(toy.events, 1);
    }

    #[test]
    fn module_changed_makes_only_the_named_module_stale() {
        let mut r = Registry::new(vec![
            Box::new(Toy { id: "a", events: 0 }),
            Box::new(Bare),
            Box::new(Faulty),
        ]);
        let changed = |module| Event::ModuleChanged { module };
        with_cx("", |cx| {
            assert_eq!(r.dispatch(changed("a"), cx), ["a"]);
            assert_eq!(r.dispatch(changed("bare"), cx), ["bare"]);
            assert_eq!(r.dispatch(changed("faulty"), cx), ["faulty"]);
            assert!(r.dispatch(changed("nobody"), cx).is_empty());
            assert!(r.dispatch_to(&["a"], changed("bare"), cx).is_empty());
        });
    }

    /// Panics on every event.
    struct Faulty;

    impl Module for Faulty {
        fn id(&self) -> &'static str {
            "faulty"
        }

        fn on_event(&mut self, _event: Event, _cx: &mut Cx) -> bool {
            panic!("faulty module")
        }

        fn command(&mut self, _args: &[String], _cx: &mut Cx) -> Result<String, String> {
            panic!("faulty command")
        }
    }

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn commands_route_by_module_and_fail_as_errors() {
        let mut r = Registry::new(vec![
            Box::new(Toy { id: "a", events: 0 }),
            Box::new(Bare),
            Box::new(Faulty),
        ]);
        with_cx("q", |cx| {
            assert_eq!(r.command(&words(&["a", "echo", "x", "y"]), cx).unwrap(), "q x y");
            let mut err = |w: &[&str], cx: &mut Cx| r.command(&words(w), cx).unwrap_err();
            assert_eq!(err(&["a", "nope"], cx), "a: unknown command \"nope\"");
            assert_eq!(err(&["a"], cx), "a: missing command");
            assert_eq!(err(&["bare", "list"], cx), "bare: unknown command \"list\"");
            assert_eq!(err(&["faulty", "x"], cx), "faulty: command panicked");
            assert_eq!(
                err(&["nobody", "x"], cx),
                "unknown module \"nobody\" (modules: a, bare, faulty)"
            );
            assert_eq!(err(&[], cx), "empty request");
            // The faulty module stays registered and keeps being isolated.
            assert_eq!(err(&["faulty"], cx), "faulty: command panicked");
        });
    }

    #[test]
    fn a_panicking_module_does_not_stop_dispatch() {
        let mut r = Registry::new(vec![
            Box::new(Toy { id: "a", events: 0 }),
            Box::new(Faulty),
            Box::new(Toy { id: "b", events: 0 }),
        ]);
        with_cx("", |cx| {
            assert_eq!(r.dispatch(Event::PasteboardChanged, cx), ["a", "b"]);
            // The faulty module stays registered and keeps being isolated.
            assert_eq!(r.dispatch(Event::Wake, cx), Vec::<&str>::new());
        });
    }

    #[test]
    fn direct_items_come_in_registration_order() {
        let ids = |q: &str| -> Vec<String> {
            with_cx(q, |cx| registry().direct(cx)).into_iter().map(|i| i.id.to_string()).collect()
        };
        assert_eq!(ids("now"), ["a:now", "b:now"]);
        assert!(ids("later").is_empty());
    }

    #[test]
    fn hotkeys_carry_their_module_and_route_back() {
        let bindings = registry().hotkeys();
        let owners: Vec<&str> = bindings.iter().map(|(m, _)| *m).collect();
        assert_eq!(owners, ["a", "b"]);
        assert_eq!(bindings[1].1, Binding { spec: "cmd+K".into(), key: Ok("b-key".into()) });
        assert!(Bare.hotkeys().is_empty());
        let mut r = registry();
        with_cx("", |cx| {
            assert!(r.hotkey("b", "show", cx).is_some_and(|v| v.is("b", "list")));
            assert!(r.hotkey("b", "other", cx).is_none());
            assert!(r.hotkey("bare", "show", cx).is_none());
            assert!(r.hotkey("nobody", "show", cx).is_none());
            assert!(Bare.direct(cx).is_empty());
        });
    }

    #[test]
    #[should_panic(expected = "duplicate module id")]
    fn duplicate_ids_are_rejected() {
        let _ = Registry::new(vec![Box::new(Bare), Box::new(Bare)]);
    }
}
