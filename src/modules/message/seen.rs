//! Unread posts and stored dismissals (flick-cb7d, flick-07cd), in the `messages` columns
//! `unread` and `dismissed` (migration 4).
//!
//! - A post nobody asked for (`unprompted`: `message post` without `--reply-to`, not
//!   `--pending` or `--partial`, outside chat threads; KOTA's nudges) is stored unread, and
//!   its card stays `unprompted_timeout_secs` (default 0: until the user closes it). So is a
//!   card without `reply_to`, thread or actions that is not `pending` (flick-8ce3). Replies,
//!   placeholders, chat posts and other cards keep their own timeouts.
//! - Unread clears when the user closes its card (x, a click, Esc) or opens the message list
//!   (view `recent`: the launcher item, `hotkey`; view `cards`: the KOTA menu's Inbox). Opening the list
//!   marks the rows it read `Unread` in that view and closes their corner cards; the `cards`
//!   view and `message read <id>|--all` (`history.rs`) read the same way. A timeout,
//!   `message hide` or a restart leave it unread.
//! - How a card left the corner is stored: 'user' (x, Esc, a click, `card dismiss`, a
//!   `dismiss` action) or 'timeout'. A re-post that shows clears it, as `card show` does.
//!   So a card the user dismissed stays out of the pending count after a restart.
//! - `pending.rs` sends the unread count beside the card count in `Event::CardsPending`.

use std::collections::HashSet;

use rusqlite::params;

use super::store::{Message, Progress, Role};
use super::{Inbox, card};
use crate::core::Cx;
use crate::core::card::{Card, State};
use crate::core::store::Store;

/// How a card left the corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismissal {
    /// The user closed it.
    User,
    /// It timed out: still waiting on the user.
    Timeout,
}

impl Dismissal {
    pub fn column(self) -> &'static str {
        match self {
            Dismissal::User => "user",
            Dismissal::Timeout => "timeout",
        }
    }

    /// Unknown values read as not dismissed.
    pub fn read(s: Option<&str>) -> Option<Self> {
        match s {
            Some("user") => Some(Dismissal::User),
            Some("timeout") => Some(Dismissal::Timeout),
            _ => None,
        }
    }
}

/// A post nobody asked for: it waits to be read. A card counts when `unprompted_card`.
pub fn unprompted(m: &Message) -> bool {
    m.reply_to.is_none()
        && !m.pending
        && m.state == Progress::Done
        && m.thread.is_none()
        && m.role == Role::Peer
        && (m.card.is_none() || card::stored(m).is_some_and(|c| unprompted_card(&c)))
}

/// A card nobody asked for (flick-8ce3): no `reply_to`, no thread, no actions, not
/// `pending`. It is unread as a text post is and stays `unprompted_timeout_secs`; a card with
/// actions waits on the user through the badge's card count instead.
pub fn unprompted_card(c: &Card) -> bool {
    c.reply_to.is_none() && c.thread.is_none() && c.actions.is_empty() && c.state != State::Pending
}

/// Unread and dismissal state on the shared store.
pub trait Seen {
    /// Set how message `id` left the corner; `User` also marks it read.
    fn set_dismissed(&self, id: &str, how: Option<Dismissal>);
    /// Mark every unread message read; returns their ids.
    fn read_all(&self) -> Vec<String>;
    /// Mark message `id` read; true when it was unread.
    fn read_one(&self, id: &str) -> bool;
    fn unread_count(&self) -> u32;
}

impl Seen for Store {
    fn set_dismissed(&self, id: &str, how: Option<Dismissal>) {
        let sql = "UPDATE messages SET dismissed = ?2, unread = unread AND ?2 IS NOT 'user' WHERE id = ?1";
        let _ = self.conn().execute(sql, params![id, how.map(Dismissal::column)]);
    }

    fn read_all(&self) -> Vec<String> {
        let conn = self.conn();
        let ids = conn
            .prepare("SELECT id FROM messages WHERE unread")
            .and_then(|mut s| s.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        let _ = conn.execute("UPDATE messages SET unread = 0 WHERE unread", []);
        ids
    }

    fn read_one(&self, id: &str) -> bool {
        self.conn().execute("UPDATE messages SET unread = 0 WHERE id = ?1 AND unread", [id]).is_ok_and(|n| n > 0)
    }

    fn unread_count(&self) -> u32 {
        self.conn().query_row("SELECT COUNT(*) FROM messages WHERE unread", [], |r| r.get(0)).unwrap_or(0)
    }
}

impl Inbox {
    /// Seconds `m`'s card stays.
    pub(super) fn timeout(&self, m: &Message) -> u64 {
        if unprompted(m) { self.settings.unprompted_timeout_secs } else { self.settings.timeout_secs }
    }

    /// The message list opens: read every unread post, remember which for its rows, and
    /// close their corner cards.
    pub(super) fn open_recent(&mut self, cx: &Cx) {
        self.fresh = cx.store.read_all().into_iter().collect::<HashSet<_>>();
        for id in &self.fresh {
            (self.env.dismiss)(id);
        }
        self.announce(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::message::store::{Keep, MIGRATIONS, Messages};

    const KEEP: Keep = Keep { history: 9, per_thread: 9, threads: 9 };

    #[test]
    fn migration_4_keeps_old_rows_read_and_shown() {
        let s = Store::in_memory();
        s.migrate("message", &MIGRATIONS[..3]).unwrap();
        s.conn().execute_batch("INSERT INTO messages (id, ts, body) VALUES ('old', 1, 'hi')").unwrap();
        s.migrate("message", MIGRATIONS).unwrap();
        let old = s.message("old").unwrap();
        assert_eq!((old.unread, old.dismissed), (false, None));
        assert_eq!(s.unread_count(), 0);
    }

    #[test]
    fn unread_and_dismissals_round_trip() {
        let s = Store::in_memory();
        s.migrate("message", MIGRATIONS).unwrap();
        let m = |id: &str| Message { id: id.into(), unread: true, ..Message::default() };
        for id in ["a", "b", "c"] {
            s.put_message(&m(id), KEEP).unwrap();
        }
        assert_eq!(s.unread_count(), 3);
        let json = serde_json::to_string(&s.message("a")).unwrap();
        assert!(json.ends_with(r#""pending":false,"unread":true}"#), "{json}");
        // A timeout keeps it unread; the user's dismissal reads it.
        s.set_dismissed("a", Some(Dismissal::Timeout));
        assert_eq!(s.message("a").map(|m| (m.unread, m.dismissed)), Some((true, Some(Dismissal::Timeout))));
        s.set_dismissed("a", Some(Dismissal::User));
        assert_eq!(s.message("a").map(|m| (m.unread, m.dismissed)), Some((false, Some(Dismissal::User))));
        s.set_dismissed("a", None);
        assert_eq!(s.message("a").map(|m| (m.unread, m.dismissed)), Some((false, None)));
        let mut read = s.read_all();
        read.sort();
        assert_eq!(read, ["b", "c"]);
        assert_eq!((s.unread_count(), s.read_all().len()), (0, 0));
        // Unknown values written by a later version read as not dismissed.
        s.conn().execute_batch("UPDATE messages SET dismissed = 'x' WHERE id = 'b'").unwrap();
        assert_eq!(s.message("b").unwrap().dismissed, None);
    }

    #[test]
    fn only_a_post_nobody_asked_for_is_unprompted() {
        let post = Message { id: "p".into(), ..Message::default() };
        assert!(unprompted(&post));
        let card = |json: &str| Message { card: Some(json.into()), ..post.clone() };
        assert!(unprompted(&card(r#"{"v":1,"id":"p","title":"FYI","state":"done"}"#)));
        for other in [
            card(r#"{"v":1,"id":"p","title":"Go?","actions":[{"id":"go","label":"Go"}]}"#),
            card(r#"{"v":1,"id":"p","title":"Busy","state":"pending"}"#),
        ] {
            assert!(!unprompted(&other), "{other:?}");
        }
        for other in [
            Message { reply_to: Some("k1".into()), ..post.clone() },
            Message { pending: true, ..post.clone() },
            Message { state: Progress::Partial, ..post.clone() },
            Message { thread: Some("t".into()), ..post.clone() },
            Message { card: Some("{}".into()), ..post.clone() },
            Message { role: Role::Me, ..post.clone() },
        ] {
            assert!(!unprompted(&other), "{other:?}");
        }
    }
}
