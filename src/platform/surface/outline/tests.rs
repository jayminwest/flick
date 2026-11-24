use super::*;
use crate::core::card::{Origin, parse};
use crate::platform::hud::CardUi;
use crate::platform::surface::rows::Row;

fn bubble(side: Side, header: &str, md: &str, state: BubbleState) -> Owned {
    Owned::new(&Row::Bubble { key: "k", version: 1, side, header, time: "", md, state })
}

#[test]
fn a_shown_surface_lists_header_rows_notice_chips_and_input() {
    let card = match parse(
        r#"{"id":"c1","title":"Go?","actions":[{"id":"go","label":"Go"}]}"#,
        Origin::Local,
    ) {
        Ok(p) => p.card,
        Err(e) => panic!("bad test card: {e}"),
    };
    let rows = [
        Owned::Divider { text: "Today".into() },
        Owned::new(&Row::Bubble {
            key: "1",
            version: 1,
            side: Side::Mine,
            header: "",
            time: "14:03",
            md: "hi",
            state: BubbleState::Done,
        }),
        bubble(Side::Theirs, "KOTA", "one\ntwo", BubbleState::Streaming),
        bubble(Side::System, "", "", BubbleState::Pending),
        bubble(Side::Theirs, "KOTA", "x", BubbleState::Failed),
        Owned::new(&Row::Card { key: "c1", version: 1, card: &card, ui: CardUi::default() }),
    ];
    let chips = ["notes.md".to_string(), "shot".to_string()];
    let o = Outline {
        name: "chat",
        shown: true,
        private: false,
        title: "KOTA",
        subtitle: "thread t1",
        status: Status::Busy,
        rows: &rows,
        notice: Some("No permission"),
        chips: &chips,
        input: "draft",
    };
    assert_eq!(
        surface(&o),
        "surface: chat (shown)\n  title: KOTA — thread t1 (busy)\n  rows: 6\n    divider: Today\n    bubble mine: 14:03\n      | hi\n    bubble theirs: KOTA (streaming)\n      | one\n      | two\n    bubble system (pending)\n    bubble theirs: KOTA (failed)\n      | x\n    card: c1\n      title: Go?\n      actions: go \"Go\"\n  notice: No permission\n  chips: notes.md, shot\n  input: \"draft\"\n"
    );
    let quiet =
        Outline { subtitle: "", status: Status::Idle, rows: &[], notice: None, chips: &[], ..o };
    assert_eq!(
        surface(&quiet),
        "surface: chat (shown)\n  title: KOTA\n  rows: 0\n  input: \"draft\"\n"
    );
    let error = Outline { status: Status::Error, ..quiet };
    assert!(surface(&error).contains("  title: KOTA (error)\n"));
}

#[test]
fn a_hidden_or_private_surface_shows_only_its_name() {
    let o = Outline {
        name: "llm-private",
        shown: true,
        private: true,
        title: "Secret",
        subtitle: "",
        status: Status::Idle,
        rows: &[],
        notice: None,
        chips: &[],
        input: "secret",
    };
    assert_eq!(surface(&o), "surface: llm-private (shown, private)\n");
    assert_eq!(
        surface(&Outline { shown: false, private: false, ..o }),
        "surface: llm-private (hidden)\n"
    );
}
