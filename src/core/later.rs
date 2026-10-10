//! Verbs that answer later. A module's `command` runs on the main thread and must return
//! at once, but some verbs only know their answer after work on a thread (`kota ask`
//! waits for ssh). Such a verb calls `answer_later` and hands the sender to its thread;
//! the control socket then waits for what the thread sends instead of the verb's
//! immediate text, while the main thread stays free (the thread may itself send requests
//! to this Flick).
//!
//! The control layer calls `take` on the main thread right after the verb returns (every
//! caller, so a slot never leaks into the next request), then `settle` on its own
//! thread. Callers that do not wait (`control::local`) drop it.

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

/// A verb's answer: `Ok` text, or the error for the caller.
pub type Answer = Result<String, String>;

/// How long the socket waits for a later answer.
pub const MAX_WAIT: Duration = Duration::from_secs(60);

thread_local! {
    static SLOT: RefCell<Option<Receiver<Answer>>> = const { RefCell::new(None) };
}

/// From a verb on the main thread: the request's answer becomes what the returned sender
/// sends. If it is dropped without sending, the verb's own answer stands.
pub fn answer_later() -> Sender<Answer> {
    let (tx, rx) = channel();
    SLOT.with(|s| *s.borrow_mut() = Some(rx));
    tx
}

/// The control layer, right after a verb returned: its later answer, if it asked for one.
pub fn take() -> Option<Receiver<Answer>> {
    SLOT.with(|s| s.borrow_mut().take())
}

/// The final answer of a verb that answered `now`: an error stands; else what `later`
/// sends within `max` (a dropped sender keeps `now`; no answer in time is an error).
pub fn settle(now: Answer, later: Option<Receiver<Answer>>, max: Duration) -> Answer {
    let (Ok(text), Some(rx)) = (&now, later) else { return now };
    match rx.recv_timeout(max) {
        Ok(answer) => answer,
        Err(RecvTimeoutError::Disconnected) => Ok(text.clone()),
        Err(RecvTimeoutError::Timeout) => Err(format!("no answer after {} s", max.as_secs())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn a_verb_without_a_later_answer_answers_now() {
        assert!(take().is_none());
        assert_eq!(settle(Ok("now".into()), None, MAX_WAIT), Ok("now".into()));
        assert_eq!(settle(Err("no".into()), None, MAX_WAIT), Err("no".into()));
    }

    #[test]
    fn the_later_answer_replaces_the_immediate_one() {
        let tx = answer_later();
        let rx = take();
        assert!(take().is_none(), "taken once");
        thread::spawn(move || tx.send(Err("ssh failed".into())));
        assert_eq!(settle(Ok("k1".into()), rx, MAX_WAIT), Err("ssh failed".into()));
    }

    #[test]
    fn an_error_or_a_dropped_sender_keeps_the_immediate_answer() {
        let tx = answer_later();
        assert_eq!(settle(Err("bad".into()), take(), MAX_WAIT), Err("bad".into()));
        drop(tx);
        let tx = answer_later();
        drop(tx);
        assert_eq!(settle(Ok("k1".into()), take(), MAX_WAIT), Ok("k1".into()));
    }

    #[test]
    fn no_answer_in_time_is_an_error() {
        let _tx = answer_later();
        assert_eq!(
            settle(Ok("k1".into()), take(), Duration::from_millis(10)),
            Err("no answer after 0 s".into())
        );
    }
}
