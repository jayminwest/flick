//! What the chat window shows, pure (flick-6a0d): a thread's messages and the reply in flight
//! as bubbles, its title, and the header's subtitle and status. `chat.rs` maps the bubbles
//! onto `platform::surface` rows.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::store::{End, Msg, Who};
use crate::platform::surface::rows::{BubbleState, Side};
use crate::platform::surface::{self, Status};

/// Characters of a thread title.
pub const TITLE_MAX: usize = 48;
/// The title of a thread with nothing sent yet.
pub const UNTITLED: &str = "New chat";
/// A reply with no text yet: the model is thinking (a reasoning model) or loading.
pub const THINKING: &str = "Thinking…";
pub const WAITING: &str = "…";

/// A reply still arriving.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub model: String,
    pub text: String,
    /// Reasoning arrived; it is not shown or kept, only noted.
    pub thinking: bool,
}

/// One bubble, owned; `rows` lends it to the surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bubble {
    pub key: String,
    pub version: u64,
    pub side: Side,
    pub header: String,
    pub md: String,
    pub state: BubbleState,
}

fn bubble(key: String, side: Side, header: String, md: String, state: BubbleState) -> Bubble {
    let mut h = DefaultHasher::new();
    (&header, &md, format!("{side:?}{state:?}")).hash(&mut h);
    Bubble { key, version: h.finish(), side, header, md, state }
}

/// The bubbles of `msgs`, then a prompt waiting for the model list (`waiting`), then the
/// reply in flight (`live`).
pub fn transcript(msgs: &[Msg], waiting: Option<&str>, live: Option<&Live>) -> Vec<Bubble> {
    let mut out: Vec<Bubble> = msgs.iter().enumerate().map(|(i, m)| message(i, m)).collect();
    if let Some(w) = waiting {
        out.push(bubble("waiting".into(), Side::Mine, String::new(), w.into(), BubbleState::Pending));
    }
    if let Some(l) = live {
        let (md, state) = match (l.text.is_empty(), l.thinking) {
            (false, _) => (l.text.clone(), BubbleState::Streaming),
            (true, true) => (THINKING.into(), BubbleState::Pending),
            (true, false) => (WAITING.into(), BubbleState::Pending),
        };
        out.push(bubble("live".into(), Side::Theirs, l.model.clone(), md, state));
    }
    out
}

fn message(i: usize, m: &Msg) -> Bubble {
    let key = format!("m{i}");
    if m.who == Who::User {
        return bubble(key, Side::Mine, String::new(), m.body.clone(), BubbleState::Done);
    }
    match &m.end {
        End::Done => bubble(key, Side::Theirs, m.model.clone(), m.body.clone(), BubbleState::Done),
        End::Stopped => bubble(key, Side::Theirs, format!("{} · stopped", m.model), m.body.clone(), BubbleState::Done),
        End::Failed(e) => {
            let md = if m.body.is_empty() { e.clone() } else { format!("{}\n\n{e}", m.body) };
            bubble(key, Side::Theirs, format!("{} · failed", m.model), md, BubbleState::Failed)
        }
    }
}

/// The surface rows of `bubbles`.
pub fn rows(bubbles: &[Bubble]) -> Vec<surface::Row<'_>> {
    bubbles
        .iter()
        .map(|b| surface::Row::Bubble { key: &b.key, version: b.version, side: b.side, header: &b.header, time: "", md: &b.md, state: b.state })
        .collect()
}

/// A thread's title: the first line of its first prompt, clipped.
pub fn title(first: &str) -> String {
    let line = first.trim().lines().next().unwrap_or_default().trim();
    if line.is_empty() {
        return UNTITLED.into();
    }
    match line.char_indices().nth(TITLE_MAX) {
        Some((i, _)) => format!("{}…", line[..i].trim_end()),
        None => line.to_string(),
    }
}

/// Where the chat is, for the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// Waiting for the server's model list before the first send.
    Loading,
    Streaming,
}

/// The header's subtitle: server and model, and the keys that apply now.
pub fn subtitle(server: &str, model: &str, phase: Phase) -> String {
    let model = if model.is_empty() { "first listed model" } else { model };
    match phase {
        Phase::Idle => format!("{server} · {model}  ·  ⌘N new chat  ·  Esc hides"),
        Phase::Loading => format!("{server} · loading models…"),
        Phase::Streaming => format!("{server} · {model}  ·  ⌘. stops"),
    }
}

/// The header's dot: busy while a reply or a model list is on its way, red when the last
/// reply failed.
pub fn status(msgs: &[Msg], phase: Phase) -> Status {
    if phase != Phase::Idle {
        return Status::Busy;
    }
    match msgs.last() {
        Some(Msg { end: End::Failed(_), .. }) => Status::Error,
        _ => Status::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(who: Who, body: &str, end: End) -> Msg {
        Msg { who, model: "qwen".into(), body: body.into(), end, ts: 1 }
    }

    fn line(b: &Bubble) -> String {
        format!("{} {:?}|{:?}|{}|{}", b.key, b.state, b.side, b.header, b.md)
    }

    #[test]
    fn messages_become_bubbles_with_their_end() {
        let msgs = [
            m(Who::User, "hi", End::Done),
            m(Who::Assistant, "yo", End::Done),
            m(Who::Assistant, "par", End::Stopped),
            m(Who::Assistant, "half", End::Failed("boom".into())),
            m(Who::Assistant, "", End::Failed("down".into())),
        ];
        let got: Vec<String> = transcript(&msgs, None, None).iter().map(line).collect();
        assert_eq!(
            got,
            [
                "m0 Done|Mine||hi",
                "m1 Done|Theirs|qwen|yo",
                "m2 Done|Theirs|qwen · stopped|par",
                "m3 Failed|Theirs|qwen · failed|half\n\nboom",
                "m4 Failed|Theirs|qwen · failed|down",
            ]
        );
    }

    #[test]
    fn a_waiting_prompt_and_the_live_reply_come_last() {
        let live = |text: &str, thinking| Live { model: "g".into(), text: text.into(), thinking };
        let one = |w, l: &Live| transcript(&[], w, Some(l)).iter().map(line).collect::<Vec<_>>().join(" / ");
        assert_eq!(one(Some("q"), &live("", false)), "waiting Pending|Mine||q / live Pending|Theirs|g|…");
        assert_eq!(one(None, &live("", true)), "live Pending|Theirs|g|Thinking…");
        assert_eq!(one(None, &live("ab", true)), "live Streaming|Theirs|g|ab");
    }

    #[test]
    fn versions_change_with_what_shows() {
        let v = |l: &Live| transcript(&[], None, Some(l))[0].version;
        let a = Live { model: "g".into(), text: "a".into(), thinking: false };
        assert_eq!(v(&a), v(&a.clone()));
        assert_ne!(v(&a), v(&Live { text: "ab".into(), ..a.clone() }));
        let done = transcript(&[m(Who::Assistant, "a", End::Done)], None, None);
        let failed = transcript(&[m(Who::Assistant, "a", End::Failed(String::new()))], None, None);
        assert_ne!(done[0].version, failed[0].version);
    }

    #[test]
    fn rows_lend_every_field() {
        let b = transcript(&[m(Who::User, "hi", End::Done)], None, None);
        let r = rows(&b);
        assert!(matches!(r[0], surface::Row::Bubble { key: "m0", side: Side::Mine, header: "", time: "", md: "hi", state: BubbleState::Done, .. }));
    }

    #[test]
    fn titles_are_the_first_line_clipped() {
        assert_eq!(title("  hello\nworld"), "hello");
        assert_eq!(title(" \n "), UNTITLED);
        let long = "é".repeat(60);
        assert_eq!(title(&long), format!("{}…", "é".repeat(48)));
        assert_eq!(title(&"a ".repeat(30)).chars().count(), 48);
    }

    #[test]
    fn the_header_follows_the_phase() {
        assert_eq!(subtitle("mlx", "q", Phase::Idle), "mlx · q  ·  ⌘N new chat  ·  Esc hides");
        assert_eq!(subtitle("mlx", "", Phase::Loading), "mlx · loading models…");
        assert_eq!(subtitle("mlx", "", Phase::Streaming), "mlx · first listed model  ·  ⌘. stops");
        let failed = [m(Who::Assistant, "", End::Failed("x".into()))];
        assert_eq!(status(&failed, Phase::Idle), Status::Error);
        assert_eq!(status(&failed, Phase::Loading), Status::Busy);
        assert_eq!(status(&[], Phase::Streaming), Status::Busy);
        assert_eq!(status(&[m(Who::User, "q", End::Done)], Phase::Idle), Status::Idle);
    }
}
