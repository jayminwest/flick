//! Registry routing for forms, item actions and confirmations: each call goes to the owning
//! module by id. Like `activate`, these are not panic-isolated.

use super::{Action, Confirm, Cx, Form, ItemId, Outcome, Registry};

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the controller shows forms, actions and confirms in flick-1875")
)]
impl Registry {
    /// Build form `name` of `module`. An unknown module has no forms.
    pub fn form(&mut self, module: &str, name: &str, cx: &mut Cx) -> Option<Form> {
        self.get(module)?.form(name, cx)
    }

    /// Save `form` through its owning module. An unknown module is an error.
    pub fn submit(&mut self, form: &Form, cx: &mut Cx) -> Result<String, String> {
        match self.get(form.module) {
            Some(m) => m.submit(form, cx),
            None => Err(format!("unknown module \"{}\"", form.module)),
        }
    }

    /// Item `id`'s action menu, from the module that owns it. An unknown module has none.
    pub fn actions(&mut self, id: &ItemId, cx: &mut Cx) -> Vec<Action> {
        self.get(id.module()).map(|m| m.actions(id, cx)).unwrap_or_default()
    }

    /// Run action `key` on item `id` through its module. An unknown module does nothing.
    pub fn act(&mut self, id: &ItemId, key: &str, cx: &mut Cx) -> Outcome {
        match self.get(id.module()) {
            Some(m) => m.act(id, key, cx),
            None => Outcome::Stay(None),
        }
    }

    /// Hand a confirmed `confirm`'s token back to the module that asked. An unknown module
    /// does nothing.
    pub fn confirmed(&mut self, confirm: &Confirm, cx: &mut Cx) -> Outcome {
        match self.get(confirm.module) {
            Some(m) => m.confirmed(&confirm.token, cx),
            None => Outcome::Stay(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Field, Icon, Module, test_cx};

    /// Owns form "new"; every item has an "edit" action; "delete" asks to confirm.
    struct Toy(&'static str);

    impl Module for Toy {
        fn id(&self) -> &'static str {
            self.0
        }

        fn form(&mut self, name: &str, _cx: &mut Cx) -> Option<Form> {
            (name == "new").then(|| Form {
                fields: vec![Field::new("name", "Name").required()],
                ..Form::new(self.0, name, "New")
            })
        }

        fn submit(&mut self, form: &Form, _cx: &mut Cx) -> Result<String, String> {
            match form.value("name") {
                Some("bad") => Err("bad name".into()),
                v => Ok(format!("{} saved {}", self.0, v.unwrap_or_default())),
            }
        }

        fn actions(&mut self, id: &ItemId, _cx: &mut Cx) -> Vec<Action> {
            vec![Action::new("edit", format!("Edit {}", id.key()), Icon::Symbol("pencil"))]
        }

        fn act(&mut self, id: &ItemId, key: &str, _cx: &mut Cx) -> Outcome {
            match key {
                "edit" => Outcome::Form { module: self.0, name: format!("edit/{}", id.key()) },
                "delete" => Outcome::Confirm(Confirm::new(self.0, id.key(), "Delete?")),
                _ => Outcome::Hide,
            }
        }

        fn confirmed(&mut self, token: &str, _cx: &mut Cx) -> Outcome {
            Outcome::Stay(Some(format!("{} deleted {token}", self.0)))
        }
    }

    /// Implements nothing but its id.
    struct Bare;

    impl Module for Bare {
        fn id(&self) -> &'static str {
            "bare"
        }
    }

    fn registry() -> Registry {
        Registry::new(vec![Box::new(Toy("a")), Box::new(Bare), Box::new(Toy("b"))])
    }

    #[test]
    fn forms_open_and_submit_through_their_module() {
        let mut r = registry();
        test_cx("", |cx| {
            let mut form = r.form("b", "new", cx).unwrap();
            assert!(form.is("b", "new"));
            assert!(r.form("b", "missing", cx).is_none());
            assert!(r.form("bare", "new", cx).is_none());
            assert!(r.form("nobody", "new", cx).is_none());
            form.set_value(0, "Docs");
            assert_eq!(r.submit(&form, cx), Ok("b saved Docs".into()));
            form.set_value(0, "bad");
            assert_eq!(r.submit(&form, cx), Err("bad name".into()));
            let bare = Form::new("bare", "x", "X");
            assert_eq!(r.submit(&bare, cx), Err("bare: cannot submit form \"x\"".into()));
            let orphan = Form::new("nobody", "x", "X");
            assert_eq!(r.submit(&orphan, cx), Err("unknown module \"nobody\"".into()));
        });
    }

    #[test]
    fn actions_route_by_item_module() {
        let mut r = registry();
        test_cx("", |cx| {
            let id = ItemId::new("a", "Docs");
            let actions = r.actions(&id, cx);
            assert_eq!(actions.len(), 1);
            assert_eq!((actions[0].key, actions[0].title.as_str()), ("edit", "Edit Docs"));
            assert!(r.actions(&ItemId::new("bare", "x"), cx).is_empty());
            assert!(r.actions(&ItemId::new("nobody", "x"), cx).is_empty());
            let edit = r.act(&id, "edit", cx);
            assert!(matches!(edit, Outcome::Form { module: "a", name } if name == "edit/Docs"));
            assert!(matches!(r.act(&id, "other", cx), Outcome::Hide));
            assert!(matches!(r.act(&ItemId::new("bare", "x"), "edit", cx), Outcome::Stay(None)));
            assert!(matches!(r.act(&ItemId::new("nobody", "x"), "edit", cx), Outcome::Stay(None)));
        });
    }

    #[test]
    fn confirms_return_their_token_to_the_module_that_asked() {
        let mut r = registry();
        test_cx("", |cx| {
            let confirm = Confirm::new("b", "Docs", "Delete?");
            let asked = r.act(&ItemId::new("b", "Docs"), "delete", cx);
            assert!(matches!(asked, Outcome::Confirm(c) if c == confirm));
            let done = r.confirmed(&confirm, cx);
            assert!(matches!(done, Outcome::Stay(Some(s)) if s == "b deleted Docs"));
            for module in ["bare", "nobody"] {
                let c = Confirm::new(module, "t", "T");
                assert!(matches!(r.confirmed(&c, cx), Outcome::Stay(None)));
            }
        });
    }
}
