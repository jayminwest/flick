//! The registered modules, and routing by module id.

use super::{Binding, Cx, Event, Item, ItemId, ListView, Module, Outcome};
use crate::config::Config;

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

    fn get(&mut self, id: &str) -> Option<&mut Box<dyn Module>> {
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
    /// stale.
    pub fn dispatch(&mut self, event: Event, cx: &mut Cx) -> Vec<&'static str> {
        let mut stale = vec![];
        for m in &mut self.modules {
            if m.on_event(event, cx) {
                stale.push(m.id());
            }
        }
        stale
    }

    /// Every module's hotkeys under `config`, in registration order, with the owning module.
    pub fn hotkeys(&self, config: &Config) -> Vec<(&'static str, Binding)> {
        self.modules
            .iter()
            .flat_map(|m| m.hotkeys(config).into_iter().map(|b| (m.id(), b)))
            .collect()
    }

    /// Run module `module`'s hotkey `key`. An unknown module does nothing.
    pub fn hotkey(&mut self, module: &str, key: &str, cx: &mut Cx) -> Option<ListView> {
        self.get(module)?.hotkey(key, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::core::{Icon, test_cx};

    /// Records calls; owns view "list".
    struct Toy {
        id: &'static str,
        events: usize,
    }

    impl Module for Toy {
        fn id(&self) -> &'static str {
            self.id
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
            event == Event::Tick
        }

        fn hotkeys(&self, config: &Config) -> Vec<Binding> {
            vec![Binding { spec: config.hotkey.clone(), key: Ok(format!("{}-key", self.id)) }]
        }

        fn hotkey(&mut self, key: &str, _cx: &mut Cx) -> Option<ListView> {
            (key == "show").then(|| ListView::new(self.id, "list"))
        }
    }

    /// Implements nothing but its id.
    struct Bare;

    impl Module for Bare {
        fn id(&self) -> &'static str {
            "bare"
        }
    }

    fn with_cx<R>(query: &str, f: impl FnOnce(&mut Cx) -> R) -> R {
        test_cx(query, Config::default(), |cx| {
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
            assert_eq!(r.dispatch(Event::Tick, cx), ["a", "b"]);
        });
        assert_eq!(toy.events, 1);
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
        let config = Config::default();
        let bindings = registry().hotkeys(&config);
        let owners: Vec<&str> = bindings.iter().map(|(m, _)| *m).collect();
        assert_eq!(owners, ["a", "b"]);
        assert_eq!(bindings[1].1, Binding { spec: config.hotkey.clone(), key: Ok("b-key".into()) });
        assert!(Bare.hotkeys(&config).is_empty());
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
