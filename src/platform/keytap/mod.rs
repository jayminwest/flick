//! One shared active keyboard event tap (`CGEventTap`) on its own thread.
//!
//! The tap runs on a dedicated thread with its own `CFRunLoop`, so a busy main thread can
//! never make it time out and stall system keyboard input. It sees key down, key up and
//! flags-changed events at the session level and asks a handler for a `Verdict` on each.
//!
//! macOS disables a tap silently after a slow callback, after sleep or on a TCC change. The
//! callback re-enables it at once on a disable notice, and every 5 s the thread checks it,
//! re-enables it, or creates it again (creation fails without Accessibility, so a later grant
//! needs no restart). After each re-arm the thread calls the `on_rearm` hook, where a caller
//! resyncs held keys with `current_flags`.
//!
//! The handler and the hook run on the tap thread. Keep them short: input waits on them.
//! Unlike the rest of `platform`, every function here may be called from any thread.

mod tap;

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, Thread};

/// `kCGHIDEventTap`: posted events enter where hardware events do, so the tap sees them.
const HID_TAP: u32 = 0;
/// `kCGEventSourceStateHIDSystemState`: the physical key state, without synthetic events.
const HID_STATE: i32 = 1;
/// `kCGEventSourceUserData`, and the stamp on events Flick posts here ("flk1").
const FIELD_SOURCE_USER_DATA: u32 = 42;
const INJECTED_MARK: i64 = 0x666c_6b31;
/// `kCGEventFlagMaskCommand`.
pub const FLAG_COMMAND: u64 = 0x0010_0000;
/// `kVK_ANSI_V`.
pub const KEY_V: u16 = 9;
/// UTF-16 units per `CGEventKeyboardSetUnicodeString` event; longer strings are cut by
/// some apps.
const UNICODE_CHUNK: usize = 20;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventKeyboardSetUnicodeString(event: *mut c_void, len: usize, chars: *const u16);
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CGEventSourceFlagsState(state: i32) -> u64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRunLoopStop(rl: *const c_void);
    fn CFRelease(cf: *const c_void);
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

/// What kind of key event the tap saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Down,
    Up,
    FlagsChanged,
}

/// One keyboard event as the handler sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapEvent {
    pub kind: Kind,
    /// macOS virtual keycode (`kVK_*`).
    pub keycode: u16,
    /// `CGEventFlags`, device-dependent bits included.
    pub flags: u64,
    pub autorepeat: bool,
    /// Posted by `post_key`, `post_combo`, `paste` or `type_text`.
    pub injected: bool,
}

/// What the tap does with an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Drop,
    /// Pass with these `CGEventFlags` instead of its own.
    SetFlags(u64),
}

/// The tap's state, for status lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not started.
    Off,
    Running,
    /// Started, but Flick lacks Accessibility; the tap is created once it is granted.
    NoPermission,
    /// Created, but macOS keeps it disabled.
    Disabled,
}

impl Status {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Status::Running,
            2 => Status::NoPermission,
            3 => Status::Disabled,
            _ => Status::Off,
        }
    }
}

/// Decides each event. Runs on the tap thread.
pub type Handler = Box<dyn FnMut(TapEvent) -> Verdict + Send>;
/// Called on the tap thread after the tap was re-enabled or created again.
pub type Rearm = Box<dyn FnMut() + Send>;

/// State shared by `start`/`stop`/`status` and one tap thread.
struct Shared {
    stop: AtomicBool,
    status: AtomicU8,
    /// The tap thread's run loop (retained, as an address), 0 until it has one.
    run_loop: Mutex<usize>,
}

impl Shared {
    fn set_status(&self, status: Status) {
        self.status.store(status as u8, Ordering::SeqCst);
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let rl = *self.run_loop.get_mut().unwrap_or_else(PoisonError::into_inner);
        if rl != 0 {
            // SAFETY: `rl` is the +1 reference the tap thread took with CFRetain.
            unsafe { CFRelease(rl as *const c_void) };
        }
    }
}

/// The running tap thread, if any.
struct Running {
    shared: Arc<Shared>,
    thread: Thread,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// Start the tap thread with `handler`, replacing a running one. The tap is created at once
/// if Flick has Accessibility, else within 5 s of the grant.
pub fn start(handler: Handler, on_rearm: Rearm) {
    stop();
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        status: AtomicU8::new(Status::NoPermission as u8),
        run_loop: Mutex::new(0),
    });
    let ours = Arc::clone(&shared);
    let spawned = thread::Builder::new()
        .name("flick-keytap".into())
        .spawn(move || tap::run(&ours, handler, on_rearm));
    match spawned {
        Ok(handle) => {
            let thread = handle.thread().clone();
            *RUNNING.lock().unwrap_or_else(PoisonError::into_inner) =
                Some(Running { shared, thread });
        }
        Err(e) => eprintln!("flick: keytap thread: {e}"),
    }
}

/// Stop the tap thread. Returns at once; from now on every event passes, and the thread
/// removes the tap and exits on its next wake.
pub fn stop() {
    let Some(running) = RUNNING.lock().unwrap_or_else(PoisonError::into_inner).take() else {
        return;
    };
    running.shared.stop.store(true, Ordering::SeqCst);
    let rl = *running.shared.run_loop.lock().unwrap_or_else(PoisonError::into_inner);
    if rl != 0 {
        // SAFETY: `rl` is a retained, live run loop; CFRunLoopStop may be called from any
        // thread.
        unsafe { CFRunLoopStop(rl as *const c_void) };
    }
    running.thread.unpark();
}

pub fn status() -> Status {
    RUNNING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map_or(Status::Off, |r| Status::from_u8(r.shared.status.load(Ordering::SeqCst)))
}

/// Press and release `keycode` with no modifiers. The tap sees both events with
/// `injected: true`.
pub fn post_key(keycode: u16) {
    post_marked(keycode, 0, &[]);
}

/// Press and release `keycode` with exactly `flags` (`CGEventFlags`), whatever modifiers are
/// physically held, marked as injected so the keys engine passes it untouched.
pub fn post_combo(keycode: u16, flags: u64) {
    post_marked(keycode, flags, &[]);
}

/// Cmd+V in the frontmost app with explicit flags (a held shift does not make it
/// cmd+shift+V), marked as injected. Unlike `ax::send_paste`, the keys engine ignores it.
pub fn paste() {
    post_combo(KEY_V, FLAG_COMMAND);
}

/// Type `text` into the frontmost app as unicode key events (no clipboard), in chunks of
/// at most 20 UTF-16 units that never split a surrogate pair. Marked as injected, no flags.
pub fn type_text(text: &str) {
    for chunk in utf16_chunks(text, UNICODE_CHUNK) {
        post_marked(0, 0, &chunk);
    }
}

/// `text` as UTF-16, cut into pieces of at most `max` units (at least 2), never between
/// the two halves of a surrogate pair.
fn utf16_chunks(text: &str, max: usize) -> Vec<Vec<u16>> {
    let max = max.max(2);
    let mut chunks = vec![];
    let mut cur: Vec<u16> = vec![];
    let mut buf = [0u16; 2];
    for c in text.chars() {
        let units = c.encode_utf16(&mut buf);
        if cur.len() + units.len() > max {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.extend_from_slice(units);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

/// Post a down and an up of `keycode` with exactly `flags`, carrying `unicode` if not
/// empty, stamped with `INJECTED_MARK`.
fn post_marked(keycode: u16, flags: u64, unicode: &[u16]) {
    for down in [true, false] {
        // SAFETY: a null source is allowed; the result is a +1 reference or null.
        let event = unsafe { CGEventCreateKeyboardEvent(ptr::null(), keycode, down) };
        if event.is_null() {
            return;
        }
        // SAFETY: `event` is a live, non-null CGEvent.
        unsafe { CGEventSetFlags(event, flags) };
        if !unicode.is_empty() {
            // SAFETY: as above; `unicode` is a live slice of `len` UTF-16 units, copied by
            // the call.
            unsafe { CGEventKeyboardSetUnicodeString(event, unicode.len(), unicode.as_ptr()) };
        }
        // SAFETY: as above; the field id is a valid `CGEventField`.
        unsafe { CGEventSetIntegerValueField(event, FIELD_SOURCE_USER_DATA, INJECTED_MARK) };
        // SAFETY: as above; posting does not consume the reference.
        unsafe { CGEventPost(HID_TAP, event) };
        // SAFETY: releases the +1 reference from CGEventCreateKeyboardEvent, used no more.
        unsafe { CFRelease(event) };
    }
}

/// The modifier flags of the physical keyboard now (`CGEventFlags`, device bits included).
pub fn current_flags() -> u64 {
    // SAFETY: plain C call with a valid source state; it only reads key state.
    unsafe { CGEventSourceFlagsState(HID_STATE) }
}

/// Whether some app has secure input on (a password field, Terminal's Secure Keyboard
/// Entry). Taps then see no key down/up events; flags-changed events still arrive.
pub fn secure_input() -> bool {
    // SAFETY: plain C call without arguments.
    unsafe { IsSecureEventInputEnabled() }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn status_round_trips_through_u8() {
        for s in [Status::Off, Status::Running, Status::NoPermission, Status::Disabled] {
            assert_eq!(Status::from_u8(s as u8), s);
        }
        assert_eq!(Status::from_u8(9), Status::Off);
    }

    #[test]
    fn read_only_queries_answer() {
        // CGEventFlags fit in 32 bits; anything above means a wrong call.
        assert_eq!(current_flags() >> 32, 0);
        let _ = secure_input();
    }

    #[test]
    fn unicode_chunks_keep_surrogate_pairs_whole() {
        assert!(utf16_chunks("", 20).is_empty());
        let ascii: String = "a".repeat(45);
        let chunks = utf16_chunks(&ascii, 20);
        assert_eq!(chunks.iter().map(Vec::len).collect::<Vec<_>>(), [20, 20, 5]);
        // "x" then 10 emoji (2 units each): a 20-unit chunk can't end mid-pair.
        let text = format!("x{}", "\u{1F600}".repeat(10));
        let chunks = utf16_chunks(&text, 20);
        assert_eq!(chunks.iter().map(Vec::len).collect::<Vec<_>>(), [19, 2]);
        let joined: Vec<u16> = chunks.concat();
        assert_eq!(String::from_utf16(&joined).ok(), Some(text));
        // A max below 2 still fits a pair.
        assert_eq!(
            utf16_chunks("\u{1F600}", 1),
            vec!["\u{1F600}".encode_utf16().collect::<Vec<_>>()]
        );
    }

    #[test]
    fn verdicts_differ() {
        assert_ne!(Verdict::Drop, Verdict::Pass);
        assert_ne!(Verdict::SetFlags(0), Verdict::Pass);
    }

    /// Installs a real tap that passes every event unchanged and removes it within a second.
    /// Needs Accessibility for the host app of the test binary; run with `--ignored`.
    #[test]
    #[ignore = "installs a real (pass-through) event tap"]
    fn starts_and_stops_a_pass_through_tap() {
        start(Box::new(|e| Verdict::SetFlags(e.flags)), Box::new(|| {}));
        thread::sleep(Duration::from_millis(300));
        let running = status();
        eprintln!("keytap status: {running:?}");
        assert!(matches!(running, Status::Running | Status::NoPermission), "{running:?}");
        stop();
        assert_eq!(status(), Status::Off);
        // Not called: they would type into the focused app.
        let _: fn(u16) = post_key;
        let _: fn() = paste;
        let _: fn(u16, u64) = post_combo;
        let _: fn(&str) = type_text;
    }
}
