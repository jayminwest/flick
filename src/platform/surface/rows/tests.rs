use super::*;
use crate::core::card::{Origin, parse};

fn card(json: &str) -> Card {
    match parse(json, Origin::Local) {
        Ok(p) => p.card,
        Err(e) => panic!("bad test card: {e}"),
    }
}

fn id(kind: Kind, key: &str, version: u64) -> Ident {
    Ident { kind, key: key.into(), version }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn rows_become_owned_copies_with_their_idents() {
    let c = card(r#"{"id":"c1","title":"Deploy?"}"#);
    let ui = CardUi { pending: true, error: Some("e"), confirm: None, note: Some("n") };
    let rows = [
        Row::Bubble {
            key: "m1",
            version: 3,
            side: Side::Mine,
            header: "Me",
            time: "14:03",
            md: "**hi**",
            state: BubbleState::Done,
        },
        Row::Card { key: "c1", version: 7, card: &c, ui },
        Row::Divider { text: "Today" },
    ];
    let owned: Vec<Owned> = rows.iter().map(Owned::new).collect();
    assert_eq!(
        owned[0],
        Owned::Bubble {
            key: "m1".into(),
            version: 3,
            side: Side::Mine,
            header: "Me".into(),
            time: "14:03".into(),
            md: "**hi**".into(),
            state: BubbleState::Done,
        }
    );
    let Owned::Card { card: kept, ui: kept_ui, .. } = &owned[1] else { panic!("not a card") };
    assert_eq!(**kept, c);
    assert_eq!(kept_ui.get(), ui);
    assert_eq!(owned[2], Owned::Divider { text: "Today".into() });
    let ids: Vec<Ident> = owned.iter().map(Owned::ident).collect();
    assert_eq!(
        ids,
        vec![id(Kind::Bubble, "m1", 3), id(Kind::Card, "c1", 7), id(Kind::Divider, "Today", 0)]
    );
}

#[test]
fn a_ui_with_an_error_keeps_the_rest() {
    let ui = Ui::new(&CardUi { pending: false, error: None, confirm: Some("go"), note: None });
    let e = ui.with_error("too big");
    assert_eq!(
        e.get(),
        CardUi { pending: false, error: Some("too big"), confirm: Some("go"), note: None }
    );
    assert_eq!(Ui::default().get(), CardUi::default());
}

#[test]
fn diff_keeps_rows_whose_kind_key_and_version_match() {
    let old = [
        id(Kind::Bubble, "a", 1),
        id(Kind::Bubble, "b", 1),
        id(Kind::Card, "c", 2),
        id(Kind::Divider, "x", 0),
    ];
    let new = [
        id(Kind::Divider, "x", 0),
        id(Kind::Bubble, "a", 1),
        // A new version rebuilds.
        id(Kind::Bubble, "b", 2),
        // The same key as another kind is another row.
        id(Kind::Bubble, "c", 2),
        id(Kind::Card, "c", 2),
        id(Kind::Bubble, "new", 0),
    ];
    assert_eq!(
        diff(&old, &new),
        vec![Plan::Keep(3), Plan::Keep(0), Plan::Build, Plan::Build, Plan::Keep(2), Plan::Build]
    );
    assert!(diff(&old, &[]).is_empty());
    assert_eq!(diff(&[], &old[..1]), vec![Plan::Build]);
}

#[test]
fn diff_reuses_each_old_row_once_in_order() {
    let d = id(Kind::Divider, "Today", 0);
    let old = [d.clone(), id(Kind::Bubble, "a", 1), d.clone()];
    let new = [d.clone(), d.clone(), d];
    assert_eq!(diff(&old, &new), vec![Plan::Keep(0), Plan::Keep(2), Plan::Build]);
}

#[test]
fn the_scroll_is_pinned_only_at_the_bottom() {
    assert!(at_bottom(600.0, 400.0, 1000.0));
    assert!(at_bottom(597.0, 400.0, 1000.0));
    assert!(!at_bottom(500.0, 400.0, 1000.0));
    // A document shorter than the view is always at the bottom.
    assert!(at_bottom(0.0, 400.0, 100.0));
    assert!(close(bottom(400.0, 1000.0), 600.0));
    assert!(close(bottom(400.0, 100.0), 0.0));
    // A top-aligned transcript (a dashboard) never pins, even at the bottom.
    assert!(pinned(Align::Bottom, 600.0, 400.0, 1000.0));
    assert!(!pinned(Align::Bottom, 500.0, 400.0, 1000.0));
    assert!(!pinned(Align::Top, 600.0, 400.0, 1000.0));
    assert_eq!(Align::default(), Align::Bottom);
}

#[test]
fn coalescing_arms_once_per_redraw() {
    let mut c = Coalesce::default();
    assert!(c.request());
    assert!(!c.request());
    assert!(!c.request());
    c.done();
    assert!(c.request());
}

#[test]
fn badges_follow_the_state() {
    let b = |spinner, failed, dim| Badge { spinner, failed, dim };
    assert_eq!(badge(BubbleState::Done), b(false, false, false));
    assert_eq!(badge(BubbleState::Streaming), b(true, false, false));
    assert_eq!(badge(BubbleState::Pending), b(true, false, true));
    assert_eq!(badge(BubbleState::Failed), b(false, true, false));
}

#[test]
fn bubbles_wrap_at_most_of_the_row_and_system_text_at_all_of_it() {
    assert!(close(text_max(400.0, Side::Theirs), 372.0 * 0.8 - 20.0));
    assert!(close(text_max(400.0, Side::Mine), 372.0 * 0.8 - 20.0));
    assert!(close(text_max(400.0, Side::System), 352.0));
    assert!(close(text_max(0.0, Side::Mine), 1.0));
    assert!(close(full_width(400.0), 372.0));
    assert!(close(full_width(10.0), 1.0));
}

#[test]
fn rows_stack_top_down_and_sit_at_the_bottom_while_short() {
    let shapes = [
        Shape::Bubble {
            side: Side::Theirs,
            text: Size { w: 100.0, h: 30.0 },
            meta_w: 60.0,
            badge: true,
        },
        Shape::Full { h: 50.0 },
        Shape::Bubble {
            side: Side::Mine,
            text: Size { w: 80.0, h: 16.0 },
            meta_w: 40.0,
            badge: false,
        },
    ];
    let (f, doc) = stack(400.0, 1000.0, &shapes, Align::Bottom);
    // Heights: 15 + 3 + 42 = 60, 50, 18 + 28 = 46; content 10 + 60 + 10 + 50 + 10 + 46 + 10.
    let content = 196.0;
    assert!(close(doc, 1000.0));
    let shift = 1000.0 - content;
    assert_eq!(f[0].row, Rect::new(0.0, 10.0 + shift, 400.0, 60.0));
    assert_eq!(f[0].meta, Rect::new(14.0, 0.0, 60.0, 15.0));
    assert_eq!(f[0].badge, Rect::new(79.0, 1.5, 12.0, 12.0));
    assert_eq!(f[0].bubble, Rect::new(14.0, 18.0, 120.0, 42.0));
    assert_eq!(f[0].text, Rect::new(24.0, 24.0, 100.0, 30.0));

    assert_eq!(f[1].row, Rect::new(14.0, 80.0 + shift, 372.0, 50.0));
    assert_eq!(f[1].bubble, ZERO);

    assert_eq!(f[2].row, Rect::new(0.0, 140.0 + shift, 400.0, 46.0));
    assert_eq!(f[2].meta, Rect::new(386.0 - 40.0, 0.0, 40.0, 15.0));
    assert_eq!(f[2].badge, ZERO);
    assert_eq!(f[2].bubble, Rect::new(386.0 - 100.0, 18.0, 100.0, 28.0));

    // Taller than the view: no shift, the document is the content.
    let (f, doc) = stack(400.0, 100.0, &shapes, Align::Bottom);
    assert!(close(doc, content));
    assert!(close(f[0].row.y, 10.0));
}

#[test]
fn top_aligned_rows_sit_at_the_top_while_short() {
    let shapes = [Shape::Full { h: 50.0 }, Shape::Full { h: 30.0 }];
    // A dashboard: no shift; the document still fills the view.
    let (f, doc) = stack(400.0, 1000.0, &shapes, Align::Top);
    assert!(close(doc, 1000.0));
    assert_eq!(f[0].row, Rect::new(14.0, 10.0, 372.0, 50.0));
    assert_eq!(f[1].row, Rect::new(14.0, 70.0, 372.0, 30.0));
    // Taller than the view: the same as bottom-aligned.
    let (f, doc) = stack(400.0, 50.0, &shapes, Align::Top);
    assert!(close(doc, 110.0) && close(f[0].row.y, 10.0));
}

#[test]
fn a_bubble_without_a_header_starts_at_its_row_top_and_system_rows_centre() {
    let lone = [Shape::Bubble {
        side: Side::System,
        text: Size { w: 50.0, h: 10.0 },
        meta_w: 0.0,
        badge: false,
    }];
    let (f, doc) = stack(200.0, 0.0, &lone, Align::Bottom);
    assert!(close(doc, 10.0 + 22.0 + 10.0));
    assert_eq!(f[0].meta, ZERO);
    assert_eq!(f[0].bubble, Rect::new(65.0, 0.0, 70.0, 22.0));
    // Only a badge: it still gets the header line, centred.
    let badge_only = [Shape::Bubble {
        side: Side::System,
        text: Size { w: 50.0, h: 10.0 },
        meta_w: 0.0,
        badge: true,
    }];
    let (f, _) = stack(200.0, 0.0, &badge_only, Align::Bottom);
    assert_eq!(f[0].meta, ZERO);
    assert_eq!(f[0].badge, Rect::new(94.0, 1.5, 12.0, 12.0));
    assert!(close(f[0].bubble.y, 18.0));

    let (f, doc) = stack(200.0, 300.0, &[], Align::Top);
    assert!(f.is_empty() && close(doc, 300.0));
}

#[test]
fn copy_message_copies_the_source_without_the_blank_space_around_it() {
    assert_eq!(message_text("\n  **Two** calls:\n- a\n- `b`\n\n"), "**Two** calls:\n- a\n- `b`");
    assert_eq!(message_text("plain"), "plain");
    assert_eq!(message_text(" \n "), "");
}
