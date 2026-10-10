//! The card HUD: a stack of small borderless, non-activating `NSPanel`s in a screen corner,
//! one per card id. Panels never activate Flick and become key only when the user clicks a
//! text field in a card, so they do not take focus from the app in front.
//!
//! - `show` upserts a card by id: a new id goes nearest the corner (with the sound, if
//!   asked); a showing id is redrawn in place, keeping its slot, without a sound. `update`
//!   redraws only a card that shows. `dismiss` / `dismiss_all` remove cards.
//! - `stack` (pure) places them: at most `Placement::max_cards` show, the rest collapse into
//!   a `+N more` pill (a click on it pages to the hidden cards); the stack stays on the
//!   screen it first appeared on until it empties.
//! - Each card hides after its own `Options::timeout_secs` unless the pointer rests on it;
//!   the timeout runs only while the user is at the Mac (`timeout.rs`).
//!   Every card has an x close button. A click elsewhere on a text card opens its link and
//!   dismisses it. Escape (a global key monitor, which needs Accessibility, installed only
//!   while cards show) dismisses every card that is not `Options::sticky`.
//! - What a card shows is a `Content` (`show`/`update`, drawn by `text`) or a
//!   `core::card::Card` with the module's `CardUi` (`show_card`/`update_card`, drawn by
//!   `card_view`). Dismissals by the user or the timeout reach the one `on_dismiss` handler;
//!   `dismiss`/`dismiss_all` do not.
//! - A card's action buttons reach the one `on_press` handler with the card id, the action id
//!   and the values JSON of its fields and choices at press time. A card redrawn by an update
//!   keeps what the user typed or picked unless the update changed that input's initial value.
//! - `focus_top` moves the keyboard into the newest card without activating Flick, `unfocus`
//!   gives it back (`focus`, keys in `keys`). Esc on a card that holds the keyboard only
//!   gives it back; a press gives it back too.
//! - `embed` draws a card into another window's view (a `surface` transcript row) with the
//!   same renderer; presses there go to `embed::on_press` instead of `on_press`.

mod card_layout;
mod card_view;
mod clicks;
mod controls;
pub mod embed;
mod escape;
mod focus;
mod keys;
mod stack;
mod text;
mod timeout;
mod view;

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSScreen, NSSound};
use objc2_foundation::{NSPoint, NSRect, NSSize};

pub use card_layout::CANCEL;
pub use card_layout::CardUi;
pub use focus::{focus_top, unfocus};
pub use stack::Corner;
pub use text::TextCard;

use crate::core::card::Card;
use escape::{escape_monitors, unmonitor};
use timeout::arm;
use view::{Pill, Views, ns};

/// What a card shows.
#[derive(Clone, Copy, Debug)]
pub enum Content<'a> {
    /// Header, time, context, body and hint; a click opens the link and dismisses.
    Text(TextCard<'a>),
}

/// What `fill` draws: a `Content`, or a card with its module state.
#[derive(Clone, Copy)]
enum Draw<'a> {
    Content(&'a Content<'a>),
    Card(&'a Card, &'a CardUi<'a>),
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
pub fn on_dismiss(handler: fn(&str, Dismissed)) {
    let _ = ON_DISMISS.set(handler);
}

/// Where action presses go: set once by `on_press`, called on the main thread outside any
/// HUD state borrow, after `AppKit` has finished delivering the click.
static ON_PRESS: OnceLock<fn(&str, &str, String)> = OnceLock::new();

/// Send action presses on cards to `handler` (card id, action id, values JSON as
/// `core::card::action::values_json` builds it). Pressing a `shell` action is just a press:
/// the module answers it by redrawing the card with `CardUi::confirm`, whose Run presses the
/// same action id again and whose Cancel presses `CANCEL`. A press whose values are over the
/// cap never reaches the handler; the card shows the error instead. The first handler wins;
/// it must only queue work (post an event), never borrow app state. Tests must not call this.
pub fn on_press(handler: fn(&str, &str, String)) {
    let _ = ON_PRESS.set(handler);
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
    /// A card's live inputs and buttons; `None` for a `Content` card.
    controls: Option<controls::Controls>,
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
    /// How many cards the last layout showed (the rest are behind the pill).
    shown: usize,
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

/// Draw `draw` into `e` at `width`. A card keeps its inputs' live values (`carry`).
fn fill(mtm: MainThreadMarker, e: &mut Entry, draw: Draw, width: f64, opts: Options, epoch: u64) {
    let before = e.controls.take().map(|c| (c.initial.clone(), c.values()));
    e.views.clear();
    let height = match draw {
        Draw::Content(Content::Text(card)) => {
            e.link = card.link.map(str::to_string);
            e.click_dismisses = true;
            text::render(mtm, &e.views.content, card, width)
        }
        Draw::Card(card, ui) => {
            e.link = None;
            e.click_dismisses = false;
            let before = before.as_ref().map(|(i, l)| (i.as_slice(), l.as_slice()));
            let values = card_layout::carry(before, card.inputs());
            let (height, controls) =
                card_view::render(mtm, &e.views.content, card, ui, &values, width);
            e.controls = Some(controls);
            height
        }
    };
    e.size = (width, height);
    e.views.resize(width, height);
    e.opts = opts;
    e.epoch = epoch;
}

/// Show card `id` with `content`, or redraw it in place if it shows. Returns whether it is new.
pub fn show(id: &str, content: &Content, placement: &Placement, opts: &Options) -> bool {
    show_draw(id, Draw::Content(content), placement, opts)
}

/// Show `card` (its id is the card id) in state `ui`, or redraw it in place if it shows.
/// Returns whether it is new.
pub fn show_card(card: &Card, ui: &CardUi, placement: &Placement, opts: &Options) -> bool {
    show_draw(&card.id, Draw::Card(card, ui), placement, opts)
}

fn show_draw(id: &str, draw: Draw, placement: &Placement, opts: &Options) -> bool {
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
                controls: None,
            };
            s.cards.insert(0, e);
        }
        if let Some(e) = s.cards.iter_mut().find(|e| e.id == id) {
            fill(mtm, e, draw, width, *opts, epoch);
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
#[expect(dead_code, reason = "no caller yet: text cards redraw through show")]
pub fn update(id: &str, content: &Content, opts: &Options) -> bool {
    update_draw(id, Draw::Content(content), opts)
}

/// Redraw `card` in state `ui` in place if it shows, keeping what the user entered; returns
/// whether it does. Never shows a dismissed card.
pub fn update_card(card: &Card, ui: &CardUi, opts: &Options) -> bool {
    update_draw(&card.id, Draw::Card(card, ui), opts)
}

fn update_draw(id: &str, draw: Draw, opts: &Options) -> bool {
    let mtm = super::mtm();
    let epoch = next_epoch();
    let shown = STATE.with_borrow_mut(|s| {
        let Some(e) = s.cards.iter_mut().find(|e| e.id == id) else { return false };
        let width = e.size.0;
        fill(mtm, e, draw, width, *opts, epoch);
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
    s.shown = layout.cards.iter().filter(|f| f.is_some()).count();
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
