//! One named thread per watched app, running its own `CFRunLoop` (`AeroSpace` `MacApp.swift:67-98`,
//! `runLoop.swift`). The app's AX observer is scheduled there and every AX call to the app is
//! made there, so a beachballed app blocks only its own thread. There is no AX messaging
//! timeout: a slow app delays nobody else.
//!
//! The main thread hands a thread jobs through its `Inbox`; a version-0 run loop source wakes
//! the loop to run them. Jobs carry plain data in; AX elements stay on the thread.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::Shared;
use super::frame::Pending;
use super::observe::{self, Watched};
use crate::platform::ax::{CFTypeRef, Cf};

/// Tries to set up the observer of an app that may have only just launched, and the pause
/// between them: a new app answers AX only once it has finished launching.
const SETUP_TRIES: u32 = 20;
const SETUP_PAUSE: Duration = Duration::from_millis(250);
/// Longest run loop turn: a fallback, since `close` signals the source.
const TURN_SECS: f64 = 60.0;

/// Work for an app thread, run with the app's observed state.
pub(super) type Job = Box<dyn FnOnce(&mut Watched) + Send>;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: CFTypeRef;
    fn CFRunLoopGetCurrent() -> CFTypeRef;
    fn CFRunLoopAddSource(rl: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
    fn CFRunLoopRunInMode(mode: CFTypeRef, seconds: f64, return_after_source: bool) -> i32;
    fn CFRunLoopWakeUp(rl: CFTypeRef);
    fn CFRunLoopSourceCreate(
        alloc: CFTypeRef,
        order: isize,
        context: *mut SourceContext,
    ) -> CFTypeRef;
    fn CFRunLoopSourceSignal(source: CFTypeRef);
    fn CFRunLoopSourceInvalidate(source: CFTypeRef);
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
}

/// `CFRunLoopSourceContext` (version 0).
#[repr(C)]
struct SourceContext {
    version: isize,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> CFTypeRef>,
    equal: Option<extern "C" fn(*const c_void, *const c_void) -> bool>,
    hash: Option<extern "C" fn(*const c_void) -> usize>,
    schedule: Option<extern "C" fn(*mut c_void, CFTypeRef, CFTypeRef)>,
    cancel: Option<extern "C" fn(*mut c_void, CFTypeRef, CFTypeRef)>,
    perform: Option<extern "C" fn(*mut c_void)>,
}

/// What the main thread and one app thread share.
#[derive(Default)]
pub(super) struct Inbox {
    state: Mutex<State>,
    frames: Mutex<Pending>,
}

#[derive(Default)]
struct State {
    jobs: Vec<Job>,
    /// The thread's run loop and its job source (retained), 0 until it runs and after it ends.
    run_loop: usize,
    source: usize,
    /// No more jobs: `stop` ran, or the thread ended.
    closed: bool,
}

impl Inbox {
    fn state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Queue `job` and wake the thread. False once the inbox is closed.
    fn send(&self, job: Job) -> bool {
        self.state(|s| {
            if s.closed {
                return false;
            }
            s.jobs.push(job);
            wake(s);
            true
        })
    }

    /// Take no more jobs and wake the thread so it ends.
    fn close(&self) {
        self.state(|s| {
            s.closed = true;
            s.jobs.clear();
            wake(s);
        });
    }

    fn closed(&self) -> bool {
        self.state(|s| s.closed)
    }

    fn take(&self) -> Vec<Job> {
        self.state(|s| std::mem::take(&mut s.jobs))
    }

    /// `f` on this app's waiting frame sets.
    pub(super) fn frames<R>(&self, f: impl FnOnce(&mut Pending) -> R) -> R {
        f(&mut self.frames.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Signal the job source and wake its run loop, if the thread is running. Called with the
/// lock held, so the thread cannot release them meanwhile.
fn wake(s: &State) {
    if s.source == 0 {
        return;
    }
    // SAFETY: both are retained by the thread until it zeroes them under this lock; signalling
    // and waking are thread-safe.
    unsafe {
        CFRunLoopSourceSignal(s.source as CFTypeRef);
    }
    // SAFETY: as above.
    unsafe { CFRunLoopWakeUp(s.run_loop as CFTypeRef) };
}

/// The main thread's handle on one app thread.
pub(super) struct App {
    inbox: Arc<Inbox>,
}

impl App {
    /// Start app `pid`'s thread. None if the thread could not be created.
    pub(super) fn spawn(pid: i32, bundle_id: Option<String>, shared: Arc<Shared>) -> Option<App> {
        let inbox = Arc::new(Inbox::default());
        let theirs = Arc::clone(&inbox);
        std::thread::Builder::new()
            .name(format!("flick-ax-{pid}"))
            .spawn(move || run(pid, bundle_id.as_deref(), &shared, &theirs))
            .ok()?;
        Some(App { inbox })
    }

    /// Run `job` on the app thread, after the jobs before it. Dropped once the thread ended.
    pub(super) fn send(&self, job: impl FnOnce(&mut Watched) + Send + 'static) {
        self.inbox.send(Box::new(job));
    }

    pub(super) fn inbox(&self) -> Arc<Inbox> {
        Arc::clone(&self.inbox)
    }

    /// Enumerate the app's windows again.
    pub(super) fn rescan(&self) {
        self.send(Watched::rescan);
    }

    /// End the thread once its current job returns; queued jobs are dropped.
    pub(super) fn stop(&self) {
        self.inbox.close();
    }
}

/// The app thread: set up the observer, then run jobs and AX callbacks until `stop`.
fn run(pid: i32, bundle_id: Option<&str>, shared: &Arc<Shared>, inbox: &Arc<Inbox>) {
    let mut tries = 0;
    let watched = loop {
        if inbox.closed() {
            return;
        }
        if let Some(w) = Watched::new(pid, bundle_id.map(str::to_owned), Arc::clone(shared)) {
            break w;
        }
        tries += 1;
        if tries == SETUP_TRIES {
            inbox.close();
            return;
        }
        std::thread::sleep(SETUP_PAUSE);
    };
    observe::install(watched);
    let source = attach(inbox);
    // Jobs sent before the source existed.
    perform(Arc::as_ptr(inbox) as *mut c_void);
    while !inbox.closed() {
        // SAFETY: runs this thread's own run loop; the mode is a framework constant.
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, TURN_SECS, true) };
    }
    detach(inbox, &source);
    observe::uninstall();
}

/// Add the job source to this thread's run loop and publish both in `inbox`.
fn attach(inbox: &Arc<Inbox>) -> Cf {
    let mut context = SourceContext {
        version: 0,
        // The thread keeps `inbox` alive past `detach`, which invalidates the source.
        info: Arc::as_ptr(inbox) as *mut c_void,
        retain: None,
        release: None,
        copy_description: None,
        equal: None,
        hash: None,
        schedule: None,
        cancel: None,
        perform: Some(perform),
    };
    // SAFETY: a valid version-0 context; CF copies it. The result is +1, owned by `Cf`.
    let source = Cf(unsafe { CFRunLoopSourceCreate(ptr::null(), 0, &mut context) });
    // SAFETY: plain C call; the current thread's run loop, not retained.
    let rl = unsafe { CFRunLoopGetCurrent() };
    if source.0.is_null() {
        return source;
    }
    // SAFETY: this thread's run loop, a live source and a framework mode constant.
    unsafe { CFRunLoopAddSource(rl, source.0, kCFRunLoopDefaultMode) };
    inbox.state(|s| {
        // SAFETY: `rl` is live while this thread runs; `detach` releases this +1.
        s.run_loop = unsafe { CFRetain(rl) } as usize;
        // SAFETY: a live source; `detach` releases this +1.
        s.source = unsafe { CFRetain(source.0) } as usize;
    });
    source
}

/// Stop taking jobs and remove the job source, so `perform` never runs again.
fn detach(inbox: &Inbox, source: &Cf) {
    let (rl, held) = inbox.state(|s| {
        s.closed = true;
        s.jobs.clear();
        (std::mem::take(&mut s.run_loop), std::mem::take(&mut s.source))
    });
    drop((Cf(rl as CFTypeRef), Cf(held as CFTypeRef)));
    if !source.0.is_null() {
        // SAFETY: a live source; invalidating removes it from every run loop.
        unsafe { CFRunLoopSourceInvalidate(source.0) };
    }
}

/// Job source callback (this app's thread): run every queued job.
extern "C" fn perform(info: *mut c_void) {
    // SAFETY: `info` is the `Inbox` the thread holds an `Arc` of until after `detach`.
    let inbox = unsafe { &*(info as *const Inbox) };
    for job in inbox.take() {
        if catch_unwind(AssertUnwindSafe(|| observe::with(job))).is_err() {
            eprintln!("flick: winman job panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_inbox_takes_no_jobs() {
        let inbox = Inbox::default();
        assert!(inbox.send(Box::new(|_| {})));
        assert!(inbox.send(Box::new(|_| {})));
        assert_eq!(inbox.take().len(), 2);
        assert!(inbox.take().is_empty());
        assert!(inbox.send(Box::new(|_| {})));
        assert!(!inbox.closed());
        inbox.close();
        assert!(inbox.closed());
        assert!(inbox.take().is_empty());
        assert!(!inbox.send(Box::new(|_| {})));
    }
}
