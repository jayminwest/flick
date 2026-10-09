//! The registered modules, and routing by module id.

use super::{Cx, Event, Item, ItemId, ListView, Module, Outcome};

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

    pub fn dispatch(&mut self, event: Event, cx: &mut Cx) {
        for m in &mut self.modules {
            m.on_event(event, cx);
        }
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
                "pop" => Outcome::Pop(Some("back".into())),
                _ => Outcome::Hide,
            }
        }

        fn on_event(&mut self, _event: Event, _cx: &mut Cx) {
            self.events += 1;
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
                matches!(r.activate(&ItemId::new("a", "pop"), cx), Outcome::Pop(Some(s)) if s == "back")
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
            bare.on_event(Event::LauncherOpened, cx);
            assert!(bare.items(cx).is_empty());
            toy.on_event(Event::LauncherOpened, cx);
            let mut r = registry();
            r.dispatch(Event::LauncherOpened, cx);
        });
        assert_eq!(toy.events, 1);
    }

    #[test]
    #[should_panic(expected = "duplicate module id")]
    fn duplicate_ids_are_rejected() {
        let _ = Registry::new(vec![Box::new(Bare), Box::new(Bare)]);
    }
}
