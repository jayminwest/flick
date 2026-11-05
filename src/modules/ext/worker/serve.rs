//! The loop of a worker thread: which queued call runs next, and whether it still answers
//! (the rules are in the parent module's docs).

use std::collections::VecDeque;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use super::{Answer, Call, DEBOUNCE, EventKind, Inbox, Job, Last, Request, Shared, Spec, bit};

/// Take the first waiting event out of `waiting`.
fn next_event(waiting: &AtomicU8) -> Option<EventKind> {
    let i = waiting.load(Ordering::SeqCst).trailing_zeros() as usize;
    let kind = *EventKind::ALL.get(i)?;
    waiting.fetch_and(!(1 << i), Ordering::SeqCst);
    Some(kind)
}

/// Keep how call `op`, started at `started`, went.
fn keep(shared: &Shared, op: &'static str, error: Option<&String>, started: Instant) {
    let last = Last { op, error: error.cloned(), took: started.elapsed() };
    *shared.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(last);
}

/// Run `ext`'s calls from `calls`, one at a time, until its `Worker` drops.
pub fn serve(ext: &Spec, job: Job, inbox: &Inbox, shared: &Shared, calls: &Receiver<Call>) {
    let mut backlog = VecDeque::new();
    // The newest query, its number and when its debounce ends. Calls queued
    // around it run while it waits, so it never holds up an action.
    let mut pending: Option<(u64, Instant, Request)> = None;
    loop {
        // Read before the queue is: a user's call queued after this stops an event.
        let mark = shared.sent.load(Ordering::SeqCst);
        backlog.extend(calls.try_iter());
        // A query's number: it runs and answers only while it is the newest.
        let (request, n) = match backlog.pop_front() {
            Some(Call::Query(n, queued, request)) => {
                pending = Some((n, queued + DEBOUNCE, request));
                continue;
            }
            Some(Call::Events) => continue,
            Some(Call::Ext(request)) => (request, None),
            Some(Call::Raw(text, reply)) => {
                let started = Instant::now();
                let result = (job.raw)(ext, &text, &(job.socket)());
                keep(shared, "call", result.as_ref().err(), started);
                let _ = reply.send(result);
                continue;
            }
            None => match pending.take() {
                Some((n, ..)) if shared.newest.load(Ordering::SeqCst) != n => continue,
                // Its debounce ends, or a call comes and runs first.
                Some((n, due, request)) => match calls.recv_timeout(due.saturating_duration_since(Instant::now())) {
                    Ok(call) => {
                        backlog.push_back(call);
                        pending = Some((n, due, request));
                        continue;
                    }
                    // Due, or the `Worker` is gone (the next `recv` ends the thread).
                    Err(_) => (request, Some(n)),
                },
                // Nothing else waits: an event, else the next call.
                None => match next_event(&shared.waiting) {
                    Some(kind) => (Request::Event(kind), None),
                    None => match calls.recv() {
                        Ok(call) => {
                            backlog.push_back(call);
                            continue;
                        }
                        Err(_) => return,
                    },
                },
            },
        };
        let event = matches!(request, Request::Event(_));
        let stale = || match n {
            Some(n) => shared.newest.load(Ordering::SeqCst) != n,
            None => event && shared.sent.load(Ordering::SeqCst) != mark,
        };
        let started = Instant::now();
        let ran = (!stale()).then(|| (job.run)(ext, &request, &(job.socket)(), &stale));
        let Some(result) = ran.filter(|_| !stale()) else {
            // Superseded, or an event a user's call stopped: that event waits again.
            if let Request::Event(kind) = request {
                shared.waiting.fetch_or(bit(kind), Ordering::SeqCst);
            }
            continue;
        };
        keep(shared, request.op().name(), result.as_ref().err(), started);
        let quiet = result.as_ref().is_ok_and(|p| p.warnings.is_empty());
        if quiet && event {
            continue;
        }
        inbox.push(Answer { ext: ext.name.clone(), request, result });
        (job.post)();
    }
}
