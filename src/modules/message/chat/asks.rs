//! Chat asks (flick-eedd): the window's Return, `message ask` and ⌘R.
//!
//! 1. The question is checked (`ask::compose`; empty or over 2000 characters is refused
//!    and the window puts the text and its chips back) and the argv built (`ask::argv`).
//!    From the window it carries its chips (`context.rs`): a `[context]` block, and the
//!    screenshot uploads the worker runs first.
//! 2. It is stored at once as the user's message (`role` me, id = the request id, in the
//!    thread), so the window shows it and "Thinking…" under it.
//! 3. A worker runs `/usr/bin/ssh -o BatchMode=yes -o ConnectTimeout=8 <kota_host>
//!    <kota_ask> --id <req> --thread <t>` with the question on stdin (20 s budget,
//!    `run::exec`), after the uploads; a failed upload is the ask's failure and kota-ask
//!    does not run. One ask runs at a time; the rest wait in order, so KOTA gets them as typed.
//! 4. Exit 0: done; KOTA answers with `message post --thread <t> --reply-to <req> --id <x>
//!    [--partial]`. Anything else marks the question failed (`Not sent · ⌘R retries`), puts
//!    the reason on the window's notice line and, for `message ask`, is the verb's error.
//!
//! `message ask [--thread t] <text...>` answers once ssh is done (`core::later`). Without
//! `--thread` it asks in the thread the window shows (or would show). `message chat
//! [--thread t]` shows the window. Both are refused over the network (`NET_DENIED`): a peer
//! must not take this Mac's keyboard or make it ssh.

use std::sync::mpsc::Sender;

use super::ask;
use super::context::{self, Attached, Upload};
use crate::core::card::valid_id;
use crate::core::later::{self, Answer};
use crate::core::Cx;
use crate::modules::message::Inbox;
use crate::modules::message::run::{self, Exit};
use crate::modules::message::store::{Message, Messages, Progress, Role};

const ASK_USAGE: &str = "usage: flick message ask [--thread t] <text...>";
const CHAT_USAGE: &str = "usage: flick message chat [--thread t] [--snapshot <png>]";

/// An ask waiting for the worker.
#[derive(Debug)]
pub struct Ask {
    pub req: String,
    pub argv: Vec<String>,
    /// The composed question, for stdin.
    pub stdin: String,
    /// Where `message ask` waits for the outcome.
    pub answer: Option<Sender<Answer>>,
    /// Screenshots to upload before kota-ask runs.
    pub uploads: Vec<Upload>,
    /// The chips it was asked with, kept for ⌘R if it does not go out.
    pub attached: Vec<Attached>,
}

/// An ask the worker finished.
#[derive(Debug)]
pub struct Asked {
    pub req: String,
    pub exit: Exit,
    pub answer: Option<Sender<Answer>>,
    pub attached: Vec<Attached>,
}

/// `--thread t` first, then the rest.
fn thread_flag<'a>(words: &'a [&'a str]) -> (Option<&'a str>, &'a [&'a str]) {
    match words {
        ["--thread", t, rest @ ..] => (Some(*t), rest),
        rest => (None, rest),
    }
}

impl Inbox {
    /// `message chat …` and `message ask …`; `args` starts with the verb.
    pub fn chat_verb(&mut self, args: &[String], cx: &Cx) -> Result<String, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let (verb, rest) = words.split_first().ok_or(CHAT_USAGE)?;
        let (thread, rest) = thread_flag(rest);
        if let Some(t) = thread.filter(|t| !valid_id(t)) {
            return Err(format!("{t}: a thread id is 1-64 of A-Z a-z 0-9 . _ -"));
        }
        let thread = thread.map(str::to_string);
        if *verb == "chat" {
            return match rest {
                [] => {
                    self.summon(thread, cx);
                    Ok(format!("Chat shows thread {}", self.current_thread(cx)))
                }
                ["--snapshot", path] => {
                    if thread.is_some() {
                        self.chat.thread = thread;
                    }
                    self.chat_snapshot(path, cx)
                }
                _ => Err(CHAT_USAGE.into()),
            };
        }
        if rest.is_empty() {
            return Err(ASK_USAGE.into());
        }
        let thread = thread.unwrap_or_else(|| self.current_thread(cx));
        let req = self.ask_with(&thread, &rest.join(" "), vec![], true, cx)?;
        Ok(format!("Asking KOTA ({req}) in thread {thread}"))
    }

    /// Store question `text` with context `attached` in `thread` and queue it for KOTA; `Ok`
    /// is the request id. `wait`: the control request answers once ssh is done
    /// (`core::later`). A refused question stores and sends nothing, and answers at once.
    pub fn ask_with(&mut self, thread: &str, text: &str, attached: Vec<Attached>, wait: bool, cx: &Cx) -> Result<String, String> {
        let req = (self.env.new_id)();
        let s = &self.settings;
        let (items, uploads) = context::plan(&attached, &s.kota_host, &s.attach_dir, &req)?;
        let stdin = ask::compose(text, &items)?;
        let argv = ask::argv(&s.kota_host, &s.kota_ask, &req, thread)?;
        let m = Message {
            id: req.clone(),
            ts: (self.env.now)(),
            body: ask::check(text)?,
            thread: Some(thread.to_string()),
            role: Role::Me,
            ..Message::default()
        };
        cx.store.put_message(&m, self.keep())?;
        self.chat.notice = None;
        let answer = wait.then(later::answer_later);
        self.chat.queue.push_back(Ask { req: req.clone(), argv, stdin, answer, uploads, attached });
        self.pump();
        Ok(req)
    }

    /// ⌘R: send the thread's newest question again if it did not go out.
    pub fn retry(&mut self, cx: &Cx) {
        let Some(thread) = self.chat.thread.clone() else { return };
        let list = cx.store.thread(&thread, self.settings.chat_history);
        let Some(q) = super::model::retry(&list).cloned() else { return };
        match self.resend(q, &thread, cx) {
            Ok(()) => self.pump(),
            Err(e) => self.chat.notice = Some(e),
        }
    }

    /// Mark failed question `q` as sent again and queue it, under its own id and with the
    /// chips it was asked with.
    fn resend(&mut self, q: Message, thread: &str, cx: &Cx) -> Result<(), String> {
        let attached = self.take_unsent(&q.id);
        let s = &self.settings;
        let (items, uploads) = context::plan(&attached, &s.kota_host, &s.attach_dir, &q.id)?;
        let stdin = ask::compose(&q.body, &items)?;
        let argv = ask::argv(&s.kota_host, &s.kota_ask, &q.id, thread)?;
        let m = Message { state: Progress::Done, ..q };
        cx.store.put_message(&m, self.keep())?;
        self.chat.notice = None;
        self.chat.queue.push_back(Ask { req: m.id, argv, stdin, answer: None, uploads, attached });
        Ok(())
    }

    /// Start the next ask if none runs.
    fn pump(&mut self) {
        if self.chat.busy.is_some() {
            return;
        }
        let Some(a) = self.chat.queue.pop_front() else { return };
        self.chat.busy = Some(a.req.clone());
        let job = run::Job {
            card: a.req.clone(),
            action: "ask".into(),
            press: 0,
            argv: a.argv,
            stdin: a.stdin,
            budget: ask::BUDGET,
        };
        let (exec, upload, req) = (self.chat.hooks.ask, self.chat.hooks.upload, a.req);
        let (answer, attached) = (a.answer.clone(), a.attached.clone());
        let failed = Asked { req: req.clone(), exit: Exit::Failed(run::NO_THREAD.into()), answer, attached };
        let (uploads, attached) = (a.uploads, a.attached);
        let work = move || {
            let exit = context::upload_all(upload, &uploads).map_or_else(Exit::Failed, |()| exec(&job));
            Asked { req, exit, answer: a.answer, attached }
        };
        self.chat.done.spawn("flick-chat-ask", work, failed, self.env.changed);
    }

    /// An ask ended: a failure marks the question and shows why; then the next one starts.
    pub(super) fn asked(&mut self, done: Asked, cx: &Cx) {
        self.chat.busy = None;
        let why = match done.exit {
            Exit::Sent => None,
            Exit::Rejected(why) => Some(format!("KOTA rejected: {why}")),
            Exit::Failed(why) => Some(why),
        };
        if let Some(why) = &why
            && let Some(q) = cx.store.message(&done.req)
        {
            // The ask's error stands whether or not the mark is saved.
            let _ = cx.store.put_message(&Message { state: Progress::Failed, ..q }, self.keep());
            self.chat.notice = Some(format!("Not sent: {why}"));
            self.keep_unsent(&done.req, done.attached);
        }
        if let Some(tx) = done.answer {
            let _ = tx.send(match why {
                None => Ok(format!("Asked KOTA ({})", done.req)),
                Some(why) => Err(format!("message ask: {why}")),
            });
        }
        self.pump();
    }
}
