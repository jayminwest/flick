//! The card HUD: a stack of small borderless, non-activating `NSPanel`s in a screen corner,
//! one per card id. Panels never become key and never activate Flick, so they do not take
//! focus from the app in front.
//!
//! - `show` upserts a card by id: a new id goes nearest the corner (with the sound, if
//!   asked); a showing id is redrawn in place, keeping its slot, without a sound. `update`
//!   redraws only a card that shows. `dismiss` / `dismiss_all` remove cards.
//! - `stack` (pure) places them: at most `Placement::max_cards` show, the rest collapse into
//!   a `+N more` pill; the stack stays on the screen it first appeared on until it empties.
//! - Each card hides after its own `Options::timeout_secs` unless the pointer rests on it.
//!   Every card has an x close button. A click elsewhere on a text card opens its link and
//!   dismisses it. Escape (a global key monitor, which needs Accessibility, installed only
//!   while cards show) dismisses every card that is not `Options::sticky`.
//! - What a card shows is a `Content`, drawn by its renderer (`text` today) into the card's
//!   content view. Dismissals by the user or the timeout reach the one `on_dismiss` handler;
//!   `dismiss`/`dismiss_all` do not.

mod stack;
mod text;
mod view;

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSScreen, NSSound};
use objc2_foundation::{NSPoint, NSRect, NSSize};

pub use stack::Corner;
pub use text::TextCard;

use super::{timer, workspace};
use view::{Pill, Views, ns};

const ESCAPE: u16 = 53;
/// Seconds the pointer over a card holds off its timeout, rechecked each time.
const HOVER_RECHECK: f64 = 2.0;

/// What a card shows.
#[derive(Clone, Copy, Debug)]
pub enum Content<'a> {
    /// Header, time, context, body and hint; a click opens the link and dismisses.
    Text(TextCard<'a>),
}

/// Where the stack goes; the latest `show` sets it for every card.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub corner: Corner,
    /// Card width in points, clamped to 240..=900. A card keeps the width it was drawn at.
    pub width: f64,
    /// Cards shown at once (at least 1); older ones collapse into a `+N more` pill.
    pub max_cards: usize,
}

/// How one card behaves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Options {
    /// Seconds before it hides; 0 keeps it until dismissed. Restarts on every show/update.
    pub timeout_secs: f64,
    /// Play a short sound when the card first appears (never on an in-place update).
    pub sound: bool,
    /// Escape does not dismiss it (a card with actions waits for one, the close button or
    /// its timeout).
    pub sticky: bool,
}

/// Why the user's side dismissed a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismissed {
    /// The x button.
    Closed,
    /// A click on a text card (its link, if any, opened).
    Clicked,
    Escape,
    Timeout,
}

/// Where user and timeout dismissals go: set once by `on_dismiss`, called on the main thread
/// after the card is gone and outside any HUD state borrow.
static ON_DISMISS: OnceLock<fn(&str, Dismissed)> = OnceLock::new();

/// Send dismissals by the user or a timeout to `handler` (card id, why). The first handler
/// wins; it must only queue work (post an event), never borrow app state. Tests must not
/// call this.
#[expect(dead_code, reason = "the message module subscribes when cards land (flick-fd93)")]
pub fn on_dismiss(handler: fn(&str, Dismissed)) {
    let _ = ON_DISMISS.set(handler);
}

struct Entry {
    id: String,
    views: Views,
    size: (f64, f64),
    opts: Options,
    /// Which show armed the live timeout; older timeouts do nothing.
    epoch: u64,
    /// Opened by a click on a text card.
    link: Option<String>,
    /// A click anywhere but a button dismisses (text cards).
    click_dismisses: bool,
}

#[derive(Default)]
struct State {
    /// Newest first.
    cards: Vec<Entry>,
    corner: Option<Corner>,
    max_cards: usize,
    /// The visible area of the screen the stack appeared on; `None` while empty.
    area: Option<NSRect>,
    pill: Option<Pill>,
    monitors: Vec<Retained<AnyObject>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
    static EPOCH: Cell<u64> = const { Cell::new(0) };
}

fn next_epoch() -> u64 {
    EPOCH.set(EPOCH.get() + 1);
    EPOCH.get()
}

/// Draw `content` into `e` at `width`.
fn fill(
    mtm: MainThreadMarker,
    e: &mut Entry,
    content: &Content,
    width: f64,
    opts: Options,
    epoch: u64,
) {
    e.views.clear();
    let height = match content {
        Content::Text(card) => {
            e.link = card.link.map(str::to_string);
            e.click_dismisses = true;
            text::render(mtm, &e.views.content, card, width)
        }
    };
    e.size = (width, height);
    e.views.resize(width, height);
    e.opts = opts;
    e.epoch = epoch;
}

/// Show card `id` with `content`, or redraw it in place if it shows. Returns whether it is new.
pub fn show(id: &str, content: &Content, placement: &Placement, opts: &Options) -> bool {
    let mtm = super::mtm();
    let width = placement.width.clamp(240.0, 900.0);
    let epoch = next_epoch();
    let new = STATE.with_borrow_mut(|s| {
        s.corner = Some(placement.corner);
        s.max_cards = placement.max_cards;
        let new = !s.cards.iter().any(|e| e.id == id);
        if new {
            if s.cards.is_empty() {
                s.area = screen_area(mtm);
            }
            let views = view::make(mtm);
            let e = Entry {
                id: id.into(),
                views,
                size: (0.0, 0.0),
                opts: *opts,
                epoch,
                link: None,
                click_dismisses: false,
            };
            s.cards.insert(0, e);
        }
        if let Some(e) = s.cards.iter_mut().find(|e| e.id == id) {
            fill(mtm, e, content, width, *opts, epoch);
        }
        relayout(s, mtm);
        if s.monitors.is_empty() {
            s.monitors = escape_monitors();
        }
        new
    });
    arm(id, epoch, opts.timeout_secs);
    if new
        && opts.sound
        && let Some(sound) = NSSound::soundNamed(&ns("Tink"))
    {
        sound.play();
    }
    new
}

/// Redraw card `id` in place if it shows, at the width it was drawn at; returns whether it
/// does. Never shows a dismissed card.
#[expect(dead_code, reason = "card state updates call it (flick-fd93)")]
pub fn update(id: &str, content: &Content, opts: &Options) -> bool {
    let mtm = super::mtm();
    let epoch = next_epoch();
    let shown = STATE.with_borrow_mut(|s| {
        let Some(e) = s.cards.iter_mut().find(|e| e.id == id) else { return false };
        let width = e.size.0;
        fill(mtm, e, content, width, *opts, epoch);
        relayout(s, mtm);
        true
    });
    if shown {
        arm(id, epoch, opts.timeout_secs);
    }
    shown
}

/// Remove card `id` if it shows; returns whether it did. No `on_dismiss` call.
pub fn dismiss(id: &str) -> bool {
    remove(id).is_some()
}

/// Remove every card. No `on_dismiss` calls.
pub fn dismiss_all() {
    let (cards, monitors) = STATE.with_borrow_mut(|s| {
        let cards = std::mem::take(&mut s.cards);
        (cards, empty(s))
    });
    for e in &cards {
        e.views.place(None);
    }
    unmonitor(&monitors);
}

/// Take card `id` out of the stack and close the gap.
fn remove(id: &str) -> Option<Entry> {
    let mtm = super::mtm();
    let (entry, monitors) = STATE.with_borrow_mut(|s| {
        let i = s.cards.iter().position(|e| e.id == id)?;
        let e = s.cards.remove(i);
        e.views.place(None);
        if s.cards.is_empty() {
            return Some((e, empty(s)));
        }
        relayout(s, mtm);
        Some((e, vec![]))
    })?;
    unmonitor(&monitors);
    Some(entry)
}

/// Reset an emptied stack; returns the key monitors to remove.
fn empty(s: &mut State) -> Vec<Retained<AnyObject>> {
    s.area = None;
    if let Some(p) = &s.pill {
        p.place(None);
    }
    std::mem::take(&mut s.monitors)
}

fn unmonitor(monitors: &[Retained<AnyObject>]) {
    for monitor in monitors {
        // SAFETY: each monitor came from addGlobal/addLocalMonitorForEvents and is removed
        // once: it was taken out of `State`.
        unsafe { NSEvent::removeMonitor(monitor) };
    }
}

/// Dismiss card `id` for the user's side and tell the handler.
fn close(id: &str, why: Dismissed) {
    if remove(id).is_some()
        && let Some(handler) = ON_DISMISS.get()
    {
        handler(id, why);
    }
}

/// Place every card and the pill.
fn relayout(s: &mut State, mtm: MainThreadMarker) {
    let (Some(area), Some(corner)) = (s.area, s.corner) else { return };
    let area = stack::Rect::new(area.origin.x, area.origin.y, area.size.width, area.size.height);
    let sizes: Vec<(f64, f64)> = s.cards.iter().map(|e| e.size).collect();
    let layout = stack::layout(area, corner, &sizes, s.max_cards);
    for (e, frame) in s.cards.iter().zip(&layout.cards) {
        e.views.place(frame.map(ns_rect));
    }
    match layout.pill {
        Some((frame, hidden)) => {
            let pill = s.pill.get_or_insert_with(|| Pill::new(mtm));
            pill.place(Some((ns_rect(frame), &stack::more(hidden))));
        }
        None => {
            if let Some(p) = &s.pill {
                p.place(None);
            }
        }
    }
}

fn ns_rect(r: stack::Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}

/// The visible area of the screen under the pointer, else the main screen's.
fn screen_area(mtm: MainThreadMarker) -> Option<NSRect> {
    let mouse = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let under = screens.iter().find(|s| contains(s.frame(), mouse));
    under.or_else(|| NSScreen::mainScreen(mtm)).map(|s| s.visibleFrame())
}

fn contains(f: NSRect, p: NSPoint) -> bool {
    p.x >= f.origin.x
        && p.x < f.origin.x + f.size.width
        && p.y >= f.origin.y
        && p.y < f.origin.y + f.size.height
}

fn arm(id: &str, epoch: u64, secs: f64) {
    if secs > 0.0 {
        let id = id.to_string();
        timer::after(secs, move || expire(&id, epoch));
    }
}

/// A timeout of card `id`: dismiss it, unless a later show or update re-armed it or the
/// pointer rests on it.
fn expire(id: &str, epoch: u64) {
    let live = STATE.with_borrow(|s| {
        let e = s.cards.iter().find(|e| e.id == id && e.epoch == epoch)?;
        Some(e.views.panel.isVisible() && contains(e.views.panel.frame(), NSEvent::mouseLocation()))
    });
    match live {
        Some(true) => {
            let id = id.to_string();
            timer::after(HOVER_RECHECK, move || expire(&id, epoch));
        }
        Some(false) => close(id, Dismissed::Timeout),
        None => {}
    }
}

/// A click on the card in panel `window`: on its close button, or elsewhere.
fn clicked(window: usize, on_close: bool) {
    let hit = STATE.with_borrow(|s| {
        let e = s.cards.iter().find(|e| e.views.key() == window)?;
        Some((e.id.clone(), e.link.clone(), e.click_dismisses))
    });
    match hit {
        Some((id, _, _)) if on_close => close(&id, Dismissed::Closed),
        Some((id, link, true)) => {
            close(&id, Dismissed::Clicked);
            if let Some(link) = link {
                workspace::open_url(&link);
            }
        }
        _ => {}
    }
}

/// Escape: dismiss every card that is not sticky.
fn escape() {
    let ids: Vec<String> = STATE
        .with_borrow(|s| s.cards.iter().filter(|e| !e.opts.sticky).map(|e| e.id.clone()).collect());
    for id in ids {
        close(&id, Dismissed::Escape);
    }
}

/// Escape anywhere: a global monitor for other apps' key events (needs Accessibility;
/// without it, only clicks and timeouts dismiss) and a local one for Flick's own.
fn escape_monitors() -> Vec<Retained<AnyObject>> {
    let global = RcBlock::new(|event: NonNull<NSEvent>| {
        // SAFETY: AppKit passes a valid event for the duration of the call.
        if unsafe { event.as_ref() }.keyCode() == ESCAPE {
            timer::after(0.0, escape);
        }
    });
    let local = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: as above.
        if unsafe { event.as_ref() }.keyCode() == ESCAPE {
            timer::after(0.0, escape);
        }
        event.as_ptr()
    });
    let global =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &global);
    // SAFETY: the block returns the event it was given, unchanged: a valid pointer.
    let local = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &local)
    };
    global.into_iter().chain(local).collect()
}
