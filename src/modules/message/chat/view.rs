//! `model` rows onto `platform::surface` rows (flick-eedd): the same fields borrowed, a
//! card's press state (`dispatch.rs`) added as its `CardUi` and folded into its version so
//! the surface redraws it when the press state changes; and the header's subtitle and dot.

use super::model::{self, Row, Side, State, Status};
use crate::modules::message::dispatch::Uis;
use crate::platform::surface::{self, rows::BubbleState};

/// The surface rows of `rows`, with each card's press state from `uis`.
pub fn rows<'a>(rows: &'a [Row], uis: &'a Uis) -> Vec<surface::Row<'a>> {
    rows.iter()
        .map(|r| match r {
            Row::Bubble { key, version, side, header, time, md, state } => surface::Row::Bubble {
                key,
                version: *version,
                side: match side {
                    Side::Mine => surface::rows::Side::Mine,
                    Side::Theirs => surface::rows::Side::Theirs,
                },
                header,
                time,
                md,
                state: match state {
                    State::Done => BubbleState::Done,
                    State::Streaming => BubbleState::Streaming,
                    State::Pending => BubbleState::Pending,
                    State::Failed => BubbleState::Failed,
                },
            },
            Row::Card { key, version, card } => {
                let ui = uis.get(key);
                let version = ui.map_or(*version, |ui| model::fold(*version, &format!("{ui:?}")));
                surface::Row::Card { key, version, card, ui: ui.map(|u| u.card_ui()).unwrap_or_default() }
            }
            Row::Divider { text } => surface::Row::Divider { text },
        })
        .collect()
}

/// The header's status dot.
pub fn status(s: Status) -> surface::Status {
    match s {
        Status::Idle => surface::Status::Idle,
        Status::Busy => surface::Status::Busy,
        Status::Error => surface::Status::Error,
    }
}

/// The line under the header's title.
pub fn subtitle(s: Status) -> &'static str {
    match s {
        Status::Idle => "⌘N new thread  ·  ⌘[ ⌘] switch  ·  Esc hides",
        Status::Busy => "KOTA is on it…",
        Status::Error => "Last question not sent  ·  ⌘R retries",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::card::Card;
    use crate::modules::message::dispatch::{Phase, Ui};

    fn card() -> Box<Card> {
        let parsed = crate::core::card::parse(r#"{"id":"c1","title":"Ship?"}"#, crate::core::card::Origin::Local).unwrap();
        Box::new(parsed.card)
    }

    #[test]
    fn rows_map_field_for_field() {
        let model = [
            Row::Divider { text: "Today".into() },
            Row::Bubble { key: "q".into(), version: 7, side: Side::Mine, header: String::new(), time: "09:00".into(), md: "hi".into(), state: State::Done },
            Row::Bubble { key: "a".into(), version: 8, side: Side::Theirs, header: "KOTA".into(), time: "09:01".into(), md: "**yo**".into(), state: State::Streaming },
            Row::Bubble { key: "p".into(), version: 0, side: Side::Theirs, header: "KOTA".into(), time: String::new(), md: "…".into(), state: State::Pending },
            Row::Bubble { key: "f".into(), version: 1, side: Side::Mine, header: "x".into(), time: String::new(), md: "no".into(), state: State::Failed },
        ];
        let none = Uis::new();
        let got = rows(&model, &none);
        assert!(matches!(got[0], surface::Row::Divider { text: "Today" }));
        let bubble = |r: &surface::Row| match *r {
            surface::Row::Bubble { key, version, side, header, time, md, state } => format!("{key}|{version}|{side:?}|{header}|{time}|{md}|{state:?}"),
            _ => panic!("not a bubble"),
        };
        let got: Vec<String> = got[1..].iter().map(bubble).collect();
        assert_eq!(got, ["q|7|Mine||09:00|hi|Done", "a|8|Theirs|KOTA|09:01|**yo**|Streaming", "p|0|Theirs|KOTA||…|Pending", "f|1|Mine|x||no|Failed"]);
    }

    #[test]
    fn a_card_carries_its_press_state_in_ui_and_version() {
        let model = [Row::Card { key: "c1".into(), version: 5, card: card() }];
        let none = Uis::new();
        let idle = rows(&model, &none);
        let surface::Row::Card { key, version, card, ui } = idle[0] else { panic!("not a card") };
        assert_eq!((key, version, card.title.as_str(), ui), ("c1", 5, "Ship?", crate::platform::hud::CardUi::default()));
        let mut uis = Uis::new();
        uis.insert("c1".into(), Ui { phase: Phase::Sending { press: 1 }, ..Ui::default() });
        let sending = rows(&model, &uis);
        let surface::Row::Card { version: v2, ui, .. } = sending[0] else { panic!("not a card") };
        assert!(ui.pending);
        assert_ne!(v2, 5);
        uis.insert("c1".into(), Ui { error: Some("boom".into()), ..Ui::default() });
        let failed = rows(&model, &uis);
        let surface::Row::Card { version: v3, ui, .. } = failed[0] else { panic!("not a card") };
        assert_eq!(ui.error, Some("boom"));
        assert_ne!(v3, v2);
    }

    #[test]
    fn the_header_follows_the_thread_status() {
        assert_eq!(status(Status::Idle), surface::Status::Idle);
        assert_eq!(status(Status::Busy), surface::Status::Busy);
        assert_eq!(status(Status::Error), surface::Status::Error);
        assert!(subtitle(Status::Idle).contains("⌘N"));
        assert_eq!(subtitle(Status::Busy), "KOTA is on it…");
        assert!(subtitle(Status::Error).contains("⌘R"));
    }
}
