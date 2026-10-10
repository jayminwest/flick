//! Front tab URLs (`urls = true`). When a supported browser (`platform::browser::supports`)
//! comes to the front, or its window or title changes, the module asks for its front tab URL.
//! The read is an Apple Event that can wait on the browser or on the user's Automation
//! prompt, so one worker thread runs it (`ask`) and posts `ModuleChanged`; the module then
//! takes the answer on the main thread (`take_url`). Meanwhile the span carries the last URL
//! read from that browser; an answer with another URL splits the span like a title change,
//! and within `merge_secs` replaces it. Private windows and denied browsers give no URL.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::core::store::Store;
use crate::core::Event;
use crate::platform::browser::{self, TabUrl};
use crate::platform::events;

use super::Activity;

/// Answers the worker left for the main thread.
pub type Inbox = Arc<Mutex<Vec<Answer>>>;

/// A read of browser `bundle`'s front tab, for the app `pid` named `name`.
pub struct Ask {
    pub seq: u64,
    pub pid: i32,
    pub bundle: String,
    pub name: String,
    pub inbox: Inbox,
}

/// The result of `Ask` number `seq`.
pub struct Answer {
    pub seq: u64,
    pub pid: i32,
    pub name: String,
    pub url: TabUrl,
}

/// The module's URL state.
#[derive(Default)]
pub struct Urls {
    /// Number of the last ask; older answers are stale.
    seq: u64,
    inbox: Inbox,
    /// The last URL read, and the pid it came from.
    last: Option<(i32, String)>,
    /// An answer is being applied: `tab_url` must not ask again.
    answering: bool,
    /// Browsers (display names) that refused Automation on their last read.
    denied: Vec<String>,
}

impl Urls {
    pub fn denied(&self) -> &[String] {
        &self.denied
    }
}

impl Activity {
    /// The URL for a span of browser `app` (bundle id) `pid` named `name`: the last one read
    /// from `pid`. Also asks for a fresh read, unless an answer is being applied. `None` with
    /// `urls = false` and for apps that are not supported browsers: those are never asked.
    pub(super) fn tab_url(&mut self, pid: i32, app: &str, name: &str) -> Option<String> {
        if !self.config.urls || !browser::supports(app) {
            return None;
        }
        if !self.urls.answering {
            self.urls.seq += 1;
            let inbox = Arc::clone(&self.urls.inbox);
            let (bundle, name) = (app.to_owned(), name.to_owned());
            (self.env.ask_url)(Ask { seq: self.urls.seq, pid, bundle, name, inbox });
        }
        self.urls.last.as_ref().filter(|(p, _)| *p == pid).map(|(_, u)| u.clone())
    }

    /// Take the answer to the last ask, if it came: remember its URL, and refocus its browser
    /// if it is still in front. Older answers are dropped.
    pub(super) fn take_url(&mut self, store: &Store, now: i64) {
        let answers = std::mem::take(&mut *self.urls.inbox.lock().unwrap_or_else(PoisonError::into_inner));
        let Some(a) = answers.into_iter().rfind(|a| a.seq == self.urls.seq) else {
            return self.touch(store, now);
        };
        self.urls.denied.retain(|n| *n != a.name);
        self.urls.last = match a.url {
            TabUrl::Url(url) => Some((a.pid, url)),
            TabUrl::Denied => {
                self.urls.denied.push(a.name);
                None
            }
            TabUrl::None | TabUrl::Failed => None,
        };
        if self.front != Some(a.pid) {
            return self.touch(store, now);
        }
        self.urls.answering = true;
        let input = self.focus(a.pid);
        self.urls.answering = false;
        match input {
            Some(input) => self.apply(input, store, now),
            None => self.touch(store, now),
        }
    }
}

/// The real `Env::ask_url`: queue `ask` for the one worker thread, which reads with
/// `browser::front_tab_url` and posts `ModuleChanged`.
pub fn ask(ask: Ask) {
    static WORKER: OnceLock<Sender<Ask>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| spawn(Box::new(browser::front_tab_url), Box::new(notify)));
    let _ = worker.send(ask);
}

fn notify() {
    events::post(Event::ModuleChanged { module: "activity" });
}

/// Start a worker that answers asks with `read` and calls `notify` after each answer. Asks
/// queued behind a running read are skipped for the newest.
fn spawn(read: Box<dyn Fn(&str) -> TabUrl + Send>, notify: Box<dyn Fn() + Send>) -> Sender<Ask> {
    let (tx, rx) = mpsc::channel::<Ask>();
    let _ = std::thread::Builder::new().name("activity-urls".into()).spawn(move || {
        while let Ok(mut ask) = rx.recv() {
            while let Ok(newer) = rx.try_recv() {
                ask = newer;
            }
            let url = read(&ask.bundle);
            let answer = Answer { seq: ask.seq, pid: ask.pid, name: ask.name, url };
            ask.inbox.lock().unwrap_or_else(PoisonError::into_inner).push(answer);
            notify();
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const WAIT: Duration = Duration::from_secs(15);

    fn send(tx: &Sender<Ask>, seq: u64, inbox: &Inbox) {
        let (bundle, name) = ("b.dev".to_owned(), "B".to_owned());
        tx.send(Ask { seq, pid: 4, bundle, name, inbox: Arc::clone(inbox) }).unwrap();
    }

    /// Deterministic: the reader holds ask 1 until asks 2 and 3 are queued behind it, so
    /// the worker must skip 2 and answer 3. No timing window decides what runs.
    #[test]
    fn the_worker_answers_the_newest_ask_with_its_reader() {
        let (entered_tx, entered) = mpsc::channel::<String>();
        let (release, release_rx) = mpsc::channel::<()>();
        let (notified_tx, notified) = mpsc::channel::<()>();
        let read = move |bundle: &str| {
            entered_tx.send(bundle.to_owned()).unwrap();
            release_rx.recv_timeout(WAIT).unwrap();
            TabUrl::Url(format!("https://{bundle}/"))
        };
        let tx = spawn(Box::new(read), Box::new(move || notified_tx.send(()).unwrap()));
        let inbox = Inbox::default();
        send(&tx, 1, &inbox);
        assert_eq!(entered.recv_timeout(WAIT).unwrap(), "b.dev", "the worker reads ask 1");
        send(&tx, 2, &inbox);
        send(&tx, 3, &inbox);
        release.send(()).unwrap();
        notified.recv_timeout(WAIT).unwrap();
        assert_eq!(entered.recv_timeout(WAIT).unwrap(), "b.dev", "the worker reads again");
        release.send(()).unwrap();
        notified.recv_timeout(WAIT).unwrap();
        let answers = inbox.lock().unwrap();
        let seqs: Vec<u64> = answers.iter().map(|a| a.seq).collect();
        assert_eq!(seqs, [1, 3], "ask 2, queued behind 1, is skipped for 3");
        let last = answers.last().unwrap();
        assert_eq!((last.pid, last.name.as_str(), &last.url), (4, "B", &TabUrl::Url("https://b.dev/".into())));
    }
}
