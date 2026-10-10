//! The private chat's session (flick-56dc): the transcript lives in memory only and is
//! wiped (best effort, `transport::wipe_string`) on `clear` and on drop. It is a separate
//! type from the normal chat on purpose: it has no store handle, no `Serialize`, no `Clone`,
//! and a `Debug` that shows counts, never text, so a leak into flick.db, a control reply, an
//! event or a log line does not compile instead of being a review item.
//!
//! - Server gate: `open` takes only a `private = true` server (`Settings::private`). With
//!   none it refuses (`settings::NO_PRIVATE`); it never falls back to a normal server.
//! - Text in: `send` moves the typed text in (no copy) and builds the request body, which
//!   `io::chat` wipes once curl has it. `absorb` moves a stream's pieces in and wipes them;
//!   pieces of any stream but the current one are wiped and dropped. Text grows into fresh
//!   buffers and the outgrown ones are wiped, so a reallocation leaves no stray copy.
//! - Lifecycle (`Wipe`): the private chat (`private_chat.rs`, flick-c325) drops the session
//!   when its window closes, on `Event::Locked` and `Event::Sleep` (`wipe_on`), when config
//!   reloads (`configure` on the running module; the server may no longer be private), and on
//!   quit (an `app::on_terminate` hook in `wire.rs`; those may not borrow app state, so the
//!   session sits in `private_chat::Room`, an `Arc<Mutex<Option<..>>>` the hook reaches
//!   through a `Weak`). `clear` returns the stream still running for the caller to
//!   `io::cancel`.
//!
//! Out of reach (document, don't promise): copies `AppKit` keeps in the window's views, curl's
//! and the kernel's buffers, and pages the system swapped (encrypted on macOS).

use std::fmt;

use super::io::Status;
use super::openai::{self, Chat, Piece, Role, Turn};
use super::settings::{Server, Settings};
use super::transport::wipe_string;
use crate::core::Event;

/// Why the private transcript was wiped. Each one clears the whole session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wipe {
    /// The window closed (`surface::Handlers::closed`, or the module hid it).
    Closed,
    /// `Event::Locked`: the screen locked, or another user's session took over.
    Locked,
    /// `Event::Sleep`: the system is about to sleep.
    Sleep,
    /// Config reloaded.
    Reload,
    /// Flick is quitting. The quit hook (`private_chat::wipe_for_quit`) has no window to tell.
    #[cfg_attr(not(test), expect(dead_code, reason = "quit wipes with no window to show the notice"))]
    Quit,
    /// The user cleared it.
    Cleared,
}

impl Wipe {
    /// A line for the empty window's notice (no text of the chat in it).
    pub fn notice(self) -> &'static str {
        match self {
            Wipe::Closed => "Cleared when the window closed",
            Wipe::Locked => "Cleared when the screen locked",
            Wipe::Sleep => "Cleared when the Mac went to sleep",
            Wipe::Reload => "Cleared when the config reloaded",
            Wipe::Quit => "Cleared when Flick quit",
            Wipe::Cleared => "Cleared",
        }
    }
}

/// The wipe a core event calls for: `Locked` and `Sleep`. `Unlocked`, `Wake` and the rest
/// call for none (the session is already gone by then).
pub fn wipe_on(event: Event) -> Option<Wipe> {
    match event {
        Event::Locked => Some(Wipe::Locked),
        Event::Sleep => Some(Wipe::Sleep),
        _ => None,
    }
}

/// One message of the private transcript. No `Debug`, `Clone` or `Serialize`; its text is
/// wiped when it is dropped.
pub struct Entry {
    pub role: Role,
    pub text: String,
    /// A reasoning model's thinking (assistant entries only).
    pub reasoning: String,
    /// `Running` while the reply streams; user entries are `Done`.
    pub status: Status,
}

impl Drop for Entry {
    fn drop(&mut self) {
        wipe_string(&mut self.text);
        wipe_string(&mut self.reasoning);
    }
}

/// Append `piece` to `to`. When `to` must grow, the text moves into a new buffer of at least
/// twice the size and the old one is wiped, so no reallocation leaves an unwiped copy.
fn append(to: &mut String, piece: &str) {
    if to.capacity() - to.len() < piece.len() {
        let want = (to.len() + piece.len()).max(to.capacity() * 2).max(64);
        let mut grown = String::with_capacity(want);
        grown.push_str(to);
        wipe_string(to);
        *to = grown;
    }
    to.push_str(piece);
}

/// A private chat: one private server, a model, the transcript in memory.
pub struct PrivateSession {
    server: Server,
    model: String,
    entries: Vec<Entry>,
    /// The stream filling the last entry, while it runs.
    streaming: Option<u64>,
}

impl fmt::Debug for PrivateSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateSession")
            .field("server", &self.server.name)
            .field("model", &self.model)
            .field("entries", &self.entries.len())
            .field("streaming", &self.streaming)
            .finish_non_exhaustive()
    }
}

impl Drop for PrivateSession {
    fn drop(&mut self) {
        self.clear();
    }
}

impl PrivateSession {
    /// A session on private server `name` (`None`: the first private one). Never a normal
    /// server; with no private server, `settings::NO_PRIVATE`.
    pub fn open(settings: &Settings, name: Option<&str>) -> Result<PrivateSession, String> {
        let server = settings.private(name)?.clone();
        Ok(PrivateSession { server, model: String::new(), entries: vec![], streaming: None })
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Use `model` for the next request (a model id from the server's list).
    pub fn set_model(&mut self, model: &str) {
        model.clone_into(&mut self.model);
    }

    /// The transcript, oldest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The stream the reply arrives on, while it runs.
    pub fn streaming(&self) -> Option<u64> {
        self.streaming
    }

    /// Add the user's `text` (moved in, never copied) and build the request body for the
    /// whole chat, for `io::chat` (which wipes it once sent); then call `started`. Refused
    /// (and `text` wiped) while a reply streams, with no model, or for blank text.
    pub fn send(&mut self, mut text: String, system: &str, max_tokens: u32) -> Result<Vec<u8>, String> {
        let refused = if self.streaming.is_some() {
            Some("llm: wait for the reply, or stop it")
        } else if self.model.is_empty() {
            Some("llm: pick a model first")
        } else if text.trim().is_empty() {
            Some("llm: nothing to send")
        } else {
            None
        };
        if let Some(why) = refused {
            wipe_string(&mut text);
            return Err(why.into());
        }
        self.entries.push(Entry { role: Role::User, text, reasoning: String::new(), status: Status::Done });
        let turns: Vec<Turn> = self
            .entries
            .iter()
            .filter(|e| !e.text.is_empty())
            .map(|e| Turn { role: e.role, content: &e.text })
            .collect();
        Ok(openai::chat_body(&Chat { model: &self.model, system, turns: &turns, max_tokens }))
    }

    /// The reply to the last `send` streams on `id` (`io::chat`'s id).
    pub fn started(&mut self, id: u64) {
        let reply = Entry { role: Role::Assistant, text: String::new(), reasoning: String::new(), status: Status::Running };
        self.entries.push(reply);
        self.streaming = Some(id);
    }

    /// Add what stream `id` sent since the last call (`Shared::take`'s pieces and status),
    /// then wipe the pieces. Pieces of any other stream (one `clear` cut off) are only wiped.
    /// True when the transcript changed.
    pub fn absorb(&mut self, id: u64, mut pieces: Vec<Piece>, status: Status) -> bool {
        let current = self.streaming == Some(id);
        if let Some(reply) = self.entries.last_mut().filter(|_| current) {
            for piece in &pieces {
                match piece {
                    Piece::Text(t) => append(&mut reply.text, t),
                    Piece::Reasoning(r) => append(&mut reply.reasoning, r),
                    _ => {}
                }
            }
            if status != Status::Running {
                reply.status = status;
                self.streaming = None;
            }
        }
        openai::wipe_pieces(&mut pieces);
        current
    }

    /// Wipe the whole transcript. Returns the stream still running, for the caller to stop
    /// (`io::cancel`); its later pieces land nowhere.
    pub fn clear(&mut self) -> Option<u64> {
        self.entries.clear();
        self.streaming.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    fn settings(text: &str) -> Settings {
        parse(text).unwrap().section("llm").unwrap().unwrap().get::<Settings>().unwrap().check().unwrap()
    }

    const BOTH: &str = "[[llm.servers]]\nname = \"mlx\"\nurl = \"http://mlx\"\n\
        [[llm.servers]]\nname = \"vault\"\nurl = \"http://vault:11235\"\nprivate = true\n";

    fn session() -> PrivateSession {
        let mut s = PrivateSession::open(&settings(BOTH), None).unwrap();
        s.set_model("qwen");
        s
    }

    fn text(t: &str) -> Piece {
        Piece::Text(t.into())
    }

    /// Compiles only while `T` implements neither trait (the `static_assertions`
    /// `assert_not_impl_any` trick: a second impl makes the call ambiguous).
    trait Lacks<A> {
        fn check() {}
    }
    impl<T> Lacks<()> for T {}
    struct Serializes;
    impl<T: serde::Serialize> Lacks<Serializes> for T {}
    struct Clones;
    impl<T: Clone> Lacks<Clones> for T {}

    #[test]
    fn neither_the_session_nor_an_entry_can_be_serialized_or_cloned() {
        <PrivateSession as Lacks<_>>::check();
        <Entry as Lacks<_>>::check();
    }

    #[test]
    fn only_a_private_server_opens_a_session_with_no_fallback() {
        let s = session();
        assert_eq!((s.server().name.as_str(), s.server().private), ("vault", true));
        let all = settings(BOTH);
        assert_eq!(PrivateSession::open(&all, Some("mlx")).unwrap_err(), "llm: mlx is not private; the private chat never uses it");
        let normal = settings("[[llm.servers]]\nname = \"mlx\"\nurl = \"http://mlx\"\n");
        assert_eq!(PrivateSession::open(&normal, None).unwrap_err(), crate::modules::llm::settings::NO_PRIVATE);
    }

    #[test]
    fn debug_shows_counts_never_text() {
        let mut s = session();
        let body = s.send("my canary 7f3a".into(), "", 0).unwrap();
        s.started(1);
        s.absorb(1, vec![text("reply canary 9b2c")], Status::Running);
        let shown = format!("{s:?}");
        assert_eq!(shown, "PrivateSession { server: \"vault\", model: \"qwen\", entries: 2, streaming: Some(1), .. }");
        assert!(!shown.contains("canary"));
        assert!(String::from_utf8(body).unwrap().contains("my canary 7f3a"));
    }

    #[test]
    fn a_send_carries_the_whole_chat_and_a_reply_streams_into_the_last_entry() {
        let mut s = session();
        let body: serde_json::Value = serde_json::from_slice(&s.send("hi".into(), "be brief", 64).unwrap()).unwrap();
        assert_eq!(body["model"], "qwen");
        assert_eq!(body["max_tokens"], 64);
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        s.started(7);
        assert_eq!(s.streaming(), Some(7));
        assert!(s.absorb(7, vec![Piece::Reasoning("hm".into()), text("Hel"), Piece::Usage(openai::Usage::default())], Status::Running));
        assert!(s.absorb(7, vec![text("lo"), Piece::Done], Status::Done));
        assert_eq!(s.streaming(), None);
        let reply = &s.entries()[1];
        assert_eq!((reply.role, reply.text.as_str(), reply.reasoning.as_str()), (Role::Assistant, "Hello", "hm"));
        assert_eq!(reply.status, Status::Done);
        let next: serde_json::Value = serde_json::from_slice(&s.send("again".into(), "", 0).unwrap()).unwrap();
        let roles: Vec<&str> = next["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        assert_eq!(s.model(), "qwen");
    }

    #[test]
    fn a_failed_reply_keeps_its_status_and_an_empty_reply_is_left_out_of_the_next_request() {
        let mut s = session();
        s.send("q".into(), "", 0).unwrap();
        s.started(1);
        assert!(s.absorb(1, vec![], Status::Failed("boom".into())));
        assert_eq!(s.entries()[1].status, Status::Failed("boom".into()));
        let next: serde_json::Value = serde_json::from_slice(&s.send("q2".into(), "", 0).unwrap()).unwrap();
        assert_eq!(next["messages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn sends_are_refused_while_streaming_without_a_model_or_when_blank() {
        let mut s = session();
        assert_eq!(s.send("  \n".into(), "", 0).unwrap_err(), "llm: nothing to send");
        s.send("one".into(), "", 0).unwrap();
        s.started(1);
        assert_eq!(s.send("two".into(), "", 0).unwrap_err(), "llm: wait for the reply, or stop it");
        assert_eq!(s.entries().len(), 2);
        let mut bare = PrivateSession::open(&settings(BOTH), Some("vault")).unwrap();
        assert_eq!(bare.send("x".into(), "", 0).unwrap_err(), "llm: pick a model first");
        assert!(bare.entries().is_empty());
    }

    #[test]
    fn clear_wipes_everything_and_hands_back_the_running_stream_whose_late_pieces_land_nowhere() {
        let mut s = session();
        s.send("secret".into(), "", 0).unwrap();
        s.started(3);
        s.absorb(3, vec![text("par")], Status::Running);
        assert_eq!(s.clear(), Some(3));
        assert!(s.entries().is_empty());
        assert_eq!(s.streaming(), None);
        assert!(!s.absorb(3, vec![text("tial")], Status::Done));
        assert!(!s.absorb(9, vec![text("other")], Status::Running));
        assert!(s.entries().is_empty());
        assert_eq!(s.clear(), None);
        // The session stays usable after a clear.
        s.send("again".into(), "", 0).unwrap();
        assert_eq!(s.entries().len(), 1);
    }

    #[test]
    fn locked_and_sleep_wipe_and_nothing_else_does() {
        assert_eq!(wipe_on(Event::Locked), Some(Wipe::Locked));
        assert_eq!(wipe_on(Event::Sleep), Some(Wipe::Sleep));
        for e in [Event::Unlocked, Event::Wake, Event::Started, Event::Idle { secs: 60 }, Event::ModuleChanged { module: "llm" }] {
            assert_eq!(wipe_on(e), None, "{e:?}");
        }
        let all = [Wipe::Closed, Wipe::Locked, Wipe::Sleep, Wipe::Reload, Wipe::Quit, Wipe::Cleared];
        let notices: Vec<&str> = all.iter().map(|w| w.notice()).collect();
        assert!(notices.iter().all(|n| n.starts_with("Cleared")));
        for (i, n) in notices.iter().enumerate() {
            assert!(!notices[..i].contains(n), "{n} twice");
        }
    }

    #[test]
    fn append_moves_into_a_bigger_buffer_and_keeps_the_text() {
        let mut s = String::new();
        append(&mut s, "ab");
        assert_eq!((s.as_str(), s.capacity()), ("ab", 64));
        let long = "x".repeat(100);
        append(&mut s, &long);
        assert_eq!(s.len(), 102);
        assert_eq!(s.capacity(), 128);
        let cap = s.capacity();
        append(&mut s, "y");
        assert_eq!(s.capacity(), cap, "room left: no new buffer");
        assert!(s.starts_with("abx") && s.ends_with("xy"));
    }

    #[test]
    fn dropping_a_session_or_an_entry_wipes_quietly() {
        let mut s = session();
        s.send("gone".into(), "", 0).unwrap();
        drop(s);
        let e = Entry { role: Role::User, text: "t".into(), reasoning: "r".into(), status: Status::Done };
        drop(e);
    }
}
