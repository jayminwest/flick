//! View `cards` (flick-c3eb): what the KOTA menu bar badge counts, for the KOTA menu's
//! Inbox, which opens it by name. It lists the stored cards and the unread posts
//! (`seen.rs`) among the newest `max_history` messages, newest first. Rows, Enter and ⌘K
//! are those of `recent`. Opening it reads the unread posts, as opening `recent` does, so
//! they show `Unread` there and leave the badge.

use super::Inbox;
use super::store::{Message, Messages};
use crate::core::{Cx, ListView};

/// The view's name.
pub const VIEW: &str = "cards";

impl Inbox {
    /// Open the view: read the unread posts (`open_recent`), then the view.
    pub(super) fn open_cards(&mut self, cx: &Cx) -> ListView {
        self.open_recent(cx);
        let name = &self.settings.name;
        ListView {
            placeholder: "Search cards…".into(),
            footer: format!("{name} cards  ·  ↵ copies  ·  ⌘K show or open"),
            empty: "No cards or unread posts".into(),
            ..ListView::new("message", VIEW)
        }
    }

    /// The rows of `cards`: cards, and the posts unread or read by this opening.
    pub(super) fn card_rows(&self, cx: &Cx) -> Vec<Message> {
        let shows = |m: &Message| m.card.is_some() || m.unread || self.fresh.contains(&m.id);
        cx.store.messages(self.settings.max_history).into_iter().filter(shows).collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::modules::message::tests::{Fixture, inbox, take_log, take_pending, take_unread};
    use super::*;
    use crate::core::Module;

    const CARD: &str = r#"{"v":1,"id":"c1","title":"Deploy?","actions":[{"id":"go","label":"Ship"}]}"#;

    fn rows(f: &mut Fixture, m: &mut Inbox) -> Vec<(String, String)> {
        let mut view = f.cx("", false, |cx| m.open(VIEW, cx)).unwrap();
        assert!(view.is("message", VIEW));
        f.cx("", false, |cx| m.refresh(&mut view, cx));
        view.items.iter().map(|i| (i.id.key().to_string(), i.accessory.clone())).collect()
    }

    #[test]
    fn cards_lists_cards_and_reads_the_unread_posts() {
        let (mut f, mut m) = (Fixture::new(), inbox(""));
        f.run(&mut m, false, &["post", "--id", "n1", "Off the laptop?"]).unwrap();
        f.run(&mut m, false, &["post", "--id", "r1", "--reply-to", "x", "a reply"]).unwrap();
        f.run(&mut m, false, &["card", "post", CARD]).unwrap();
        take_unread();
        let pair = |id: &str, mark: &str| (id.to_string(), mark.to_string());
        // The reply is neither a card nor unread; the unread post shows read here.
        assert_eq!(rows(&mut f, &mut m), [pair("c1", ""), pair("n1", "Unread")]);
        assert_eq!(take_unread(), [0]);
        // Read now: the next opening lists only the card.
        assert_eq!(rows(&mut f, &mut m), [pair("c1", "")]);
        assert!(f.cx("", false, |cx| m.open("nope", cx)).is_none());
        take_log();
        take_pending();
    }
}
