//! Extension answers as launcher rows. An item is `ext:<name>/<key>` (permanent: it keys
//! the item's usage history); a view is `<name>/<view>`. Pure: the module hands in what it
//! has cached.

use super::proto::item::ExtItem;
use super::proto::{Filter, View};
use crate::core::store;
use crate::core::{Cx, Icon, Item, ItemId, ListView, Tab, frecency};

/// The icon of an item that names none.
pub const ICON: &str = "puzzlepiece.extension";
/// A view's empty text until its first answer.
pub const LOADING: &str = "Loading…";

/// Extension `ext`'s item `e`. Enter's verb is its first action's title.
pub fn item(ext: &str, e: &ExtItem) -> Item {
    let verb = e.actions.first().map_or_else(|| "Open".to_string(), |a| a.title.clone());
    Item {
        id: ItemId::new("ext", format!("{ext}/{}", e.key)),
        title: e.title.clone(),
        subtitle: e.subtitle.clone(),
        accessory: e.accessory.clone(),
        icon: e.icon.clone().unwrap_or_else(|| Icon::symbol(ICON)),
        keywords: e.keywords.clone(),
        verb: verb.into(),
        // Tab runs `e.tab` once `Tab::Act` can hold a key from outside Flick (flick-de94).
        tab: Tab::None,
        tone: e.tone,
    }
}

/// `<name>/<view>` as `(name, view)`; None when either part is empty.
pub fn split(view: &str) -> Option<(&str, &str)> {
    view.split_once('/').filter(|(ext, v)| !ext.is_empty() && !v.is_empty())
}

/// The view `name` (`<ext>/<view>`) as it opens: from `cached`, or empty and loading.
/// Activating its items counts toward their frecency.
pub fn list(name: &str, ext: &str, cached: Option<&View>) -> ListView {
    let placeholder = cached.and_then(|v| v.placeholder.clone());
    ListView {
        placeholder: placeholder.unwrap_or_else(|| format!("Search {ext}…")),
        empty: if cached.is_some() { "No Results" } else { LOADING }.into(),
        record_use: true,
        ..ListView::new("ext", name)
    }
}

/// Fill `view` of extension `ext` for `cx.query` from `cached`. A `filter: "flick"` view is
/// ranked here, by fuzzy score plus frecency; an `"ext"` one keeps the extension's order.
/// The footer is `error` (the view's last failed call), else the view's title.
pub fn fill(view: &mut ListView, ext: &str, cached: Option<&View>, error: Option<&str>, cx: &mut Cx) {
    let title = cached.and_then(|v| v.title.as_deref());
    view.footer = error.or(title).unwrap_or(ext).to_string();
    view.empty = if cached.is_some() || error.is_some() { "No Results" } else { LOADING }.into();
    let Some(cached) = cached else {
        view.items = vec![];
        return;
    };
    let items: Vec<Item> = cached.items.iter().map(|e| item(ext, e)).collect();
    view.items = match cached.filter {
        Filter::Flick => {
            let (usage, now) = (cx.store.usage(), store::now());
            cx.ranker.rank(cx.query, items, |i| frecency(&usage, i.id.as_str(), now))
        }
        // The extension filters (`op: query`, flick-f4e6).
        Filter::Ext => items,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Tone, test_cx};
    use crate::modules::ext::proto::{self, Op, Reply};

    fn items(json: &str) -> Vec<ExtItem> {
        let parsed = proto::parse(Op::Root, json.as_bytes()).unwrap();
        let Reply::Items(items) = parsed.reply else { panic!("{:?}", parsed.reply) };
        items
    }

    fn view(filter: Filter) -> View {
        let items = items(r#"{"items": [{"key": "b", "title": "Beta"}, {"key": "a", "title": "Alpha"}]}"#);
        View { title: Some("Notes".into()), placeholder: Some("Find…".into()), filter, items }
    }

    fn titles(view: &ListView) -> Vec<&str> {
        view.items.iter().map(|i| i.title.as_str()).collect()
    }

    #[test]
    fn items_carry_the_extension_and_key_in_their_id() {
        let json = r#"{"items": [{"key": "n/1", "title": "One", "subtitle": "s", "accessory": "a",
            "icon": "star", "keywords": ["k"], "tone": "warn",
            "actions": [{"key": "copy", "title": "Copy"}]}, {"key": "2", "title": "Two"}]}"#;
        let got: Vec<Item> = items(json).iter().map(|e| item("notes", e)).collect();
        assert_eq!((got[0].id.as_str(), got[0].id.module(), got[0].id.key()), ("ext:notes/n/1", "ext", "notes/n/1"));
        assert_eq!((got[0].subtitle.as_str(), got[0].accessory.as_str()), ("s", "a"));
        assert_eq!((&got[0].icon, &*got[0].verb, got[0].tone), (&Icon::symbol("star"), "Copy", Tone::Warn));
        assert_eq!(got[0].keywords, ["k"]);
        assert_eq!((&got[1].icon, &*got[1].verb, got[1].tab), (&Icon::symbol(ICON), "Open", Tab::None));
        let none = ExtItem { actions: vec![], ..items(json)[1].clone() };
        assert_eq!(item("x", &none).verb, "Open");
    }

    #[test]
    fn view_names_split_at_the_first_slash() {
        assert_eq!(split("notes/inbox"), Some(("notes", "inbox")));
        assert_eq!(split("notes/a/b"), Some(("notes", "a/b")));
        for bad in ["notes", "/inbox", "notes/", ""] {
            assert_eq!(split(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_view_opens_loading_until_it_has_an_answer() {
        let fresh = list("notes/inbox", "notes", None);
        assert!(fresh.is("ext", "notes/inbox") && fresh.record_use);
        assert_eq!((fresh.placeholder.as_str(), fresh.empty.as_str()), ("Search notes…", LOADING));
        let cached = list("notes/inbox", "notes", Some(&view(Filter::Flick)));
        assert_eq!((cached.placeholder.as_str(), cached.empty.as_str()), ("Find…", "No Results"));
        let mut v = fresh;
        test_cx("", |cx| fill(&mut v, "notes", None, None, cx));
        assert_eq!((v.footer.as_str(), v.empty.as_str(), v.items.len()), ("notes", LOADING, 0));
        test_cx("", |cx| fill(&mut v, "notes", None, Some("notes: boom"), cx));
        assert_eq!((v.footer.as_str(), v.empty.as_str()), ("notes: boom", "No Results"));
    }

    #[test]
    fn flick_filters_and_ranks_a_flick_view_with_frecency() {
        let mut v = list("notes/inbox", "notes", None);
        test_cx("", |cx| {
            fill(&mut v, "notes", Some(&view(Filter::Flick)), None, cx);
            assert_eq!((titles(&v), v.footer.as_str()), (vec!["Alpha", "Beta"], "Notes"));
            cx.store.record_use("ext:notes/b");
            fill(&mut v, "notes", Some(&view(Filter::Flick)), None, cx);
            assert_eq!(titles(&v), ["Beta", "Alpha"]);
        });
        test_cx("alp", |cx| fill(&mut v, "notes", Some(&view(Filter::Flick)), Some("e"), cx));
        assert_eq!((titles(&v), v.footer.as_str()), (vec!["Alpha"], "e"));
        test_cx("alp", |cx| fill(&mut v, "notes", Some(&view(Filter::Ext)), None, cx));
        assert_eq!(titles(&v), ["Beta", "Alpha"]);
    }
}
