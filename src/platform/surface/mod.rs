//! Surfaces: floating windows that take the keyboard without activating Flick, for
//! conversations and dashboards (chat, private chat, fleet). Generic: a surface knows no
//! module; each one is keyed by a `SurfaceId` and reports to its module's `Handlers`.
//!
//! - `open(id, &Spec, Handlers)` builds a hidden surface once (later calls with the same id do
//!   nothing). `show` puts it on the screen under the pointer and makes it key; `hide`,
//!   `is_visible`, `is_key`, `snapshot` (draws it to a PNG, for checks without Screen
//!   Recording, mx-6b45b0).
//! - Content: a header (`set_header`: title, subtitle, status dot), the transcript
//!   (`set_rows`), an optional notice line (`set_notice`), removable chips (`set_chips`) and
//!   the input (`input`, `set_input`; `Input::Single` or `Input::Multi`).
//! - Transcript: `set_rows` replaces the rows (`Row::Bubble`, `Row::Card`, `Row::Divider`).
//!   Redraws coalesce: the rows show 50 ms after the first call of a burst (or at once on
//!   `show` and `snapshot`). A row whose kind, key and version match one already shown keeps
//!   its views (and a card what the user typed); the rest are built anew. The view stays
//!   scrolled to the bottom only if it was there. Bubbles hold selectable markdown-lite
//!   (`render`: bold, italic, code, headings, bullets; only `http(s)` links open); cards are
//!   drawn by the HUD's card renderer, and their presses reach `Handlers::card_action`.
//! - Copying (`bubble`, flick-0955): a bubble's row is not window background (a drag on it
//!   selects, never moves the window), a click on its fill goes to its text, its text takes
//!   the first click while the window is not key, and its context menu starts with Copy
//!   Message (the whole message, `rows::message_text`, to the pasteboard).
//! - Keys: the edit keys (⌘X/⌘C/⌘V/⌘A/⌘Z/⇧⌘Z) go to the input through `platform::edit`
//!   before anything else. Every other ⌘ key, and Return, Escape, Up and Down in the input,
//!   reach `Handlers::key` as a `Keystroke`; a key it does not take falls back to
//!   `keys::fallback`: Return submits (⇧/⌥ Return adds a line in a multi-line input), Escape
//!   hides and calls `closed`, the rest keep their default.
//! - Focus: the window is a non-activating `NSPanel`. Flick never activates, the app in front
//!   stays active, and once the surface hides that app's window is key again. `show` makes
//!   the input first responder only when it is not already (mx-b73c04), so a redraw never
//!   moves the caret.
//! - Private (`Spec::private`, flick-f218): the window is left out of screenshots, screen
//!   recording and window sharing (`sharingType` none), is not restorable and not in the
//!   Windows menu, and keeps the spec's title as its window title. A banner strip with the
//!   spec's text and an accent outline mark it. The input has spell check, grammar,
//!   autocorrect, completion, inline predictions, Writing Tools, math results, link and data
//!   detection and undo off (`privacy::ALL`) and no context menu; bubbles cannot be selected
//!   (and their links do not open) and have no Copy Message. ⌘C/⌘X copy the input's selection only through
//!   `pasteboard::set_text_concealed` (Concealed + Transient, so clip history skips it); with
//!   nothing selected they reach `Handlers::key`, never the pasteboard. `snapshot` refuses.
//!   What `AppKit` keeps in its views after `set_rows`/`set_input` replace them is outside
//!   Flick's reach.
//! - Threading: main thread only. Handlers run on the main thread with no surface state
//!   borrowed; `submit`, `chip_removed`, `closed` and `card_action` run on a later run loop
//!   turn (deferred with `timer::after`), `key` runs synchronously because its answer decides
//!   the key's fate. Handlers must only queue work or post an event, never borrow app state
//!   (mx-fcbc43). Surface functions may be called from inside a handler.

mod bubble;
mod geometry;
mod input;
mod keys;
mod privacy;
pub(super) mod render;
pub mod rows;
mod style;
mod transcript;
mod window;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBitmapImageFileType, NSButton, NSColor, NSEvent, NSTextView};
use objc2_foundation::NSDictionary;

use super::{mtm, timer};
pub use geometry::Size;
use geometry::{Parts, Rect};
use input::ns_rect;
use keys::Fallback;
pub use keys::{Key, Keystroke};
pub use rows::Row;
use rows::{COALESCE_SECS, Owned};
use window::{Views, ns};

/// Which surface: a fixed name per use ("chat", "llm", "private", "fleet").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub &'static str);

/// The input's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// One line; Return always submits.
    Single,
    /// About four lines that scroll; ⇧ or ⌥ Return adds a line.
    Multi,
}

/// How a surface is built; read once by `open`.
#[derive(Clone, Copy, Debug)]
pub struct Spec<'a> {
    /// The header title until the first `set_header`.
    pub title: &'a str,
    /// Frame autosave name: the window's size and place persist under it. Empty: none.
    pub autosave: &'a str,
    /// Content size the first time (no autosaved frame).
    pub size: Size,
    /// Smallest content size the user can resize to (never under `geometry::MIN`).
    pub min_size: Size,
    pub placeholder: &'a str,
    pub input: Input,
    /// Hide (and call `closed`) when another window becomes key.
    pub hide_on_blur: bool,
    /// `Some(banner text)`: a private surface (see the module docs); `None`: a normal one.
    pub private: Option<&'a str>,
}

/// What the header shows.
#[derive(Clone, Copy, Debug)]
pub struct Header<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub status: Status,
}

/// The status dot right of the title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Busy,
    Error,
}

/// A removable chip above the input; clicking it reports `chip_removed` with its index (the
/// module then calls `set_chips` without it).
#[derive(Clone, Copy, Debug)]
pub struct Chip<'a> {
    pub label: &'a str,
    /// An SF Symbol name; empty for none.
    pub symbol: &'a str,
}

/// Where a surface reports. See the module docs for when each runs.
#[derive(Clone, Copy)]
pub struct Handlers {
    /// Return (unless `key` took it) with non-blank input: the text, after the input clears.
    pub submit: fn(SurfaceId, String),
    /// A key; true when the module handled it (then the surface does nothing more).
    pub key: fn(SurfaceId, Keystroke) -> bool,
    /// A chip was clicked: its index in the last `set_chips`.
    pub chip_removed: fn(SurfaceId, usize),
    /// The user hid the surface (Escape, or blur with `hide_on_blur`); not after `hide`.
    pub closed: fn(SurfaceId),
    /// A card row's button: the card id, the action id (or `hud::CANCEL` from a shell
    /// confirm's Cancel) and the values JSON of its inputs (`core::card::action::values_json`).
    /// A press whose values are over the cap never arrives; the card shows the error.
    pub card_action: fn(SurfaceId, &str, &str, String),
}

struct Surface {
    id: SurfaceId,
    handlers: Handlers,
    multi: bool,
    hide_on_blur: bool,
    private: bool,
    v: Views,
    chips: RefCell<Vec<Retained<NSButton>>>,
}

thread_local! {
    static SURFACES: RefCell<HashMap<&'static str, Rc<Surface>>> = RefCell::default();
}

/// The surface `id`, with the map's borrow already released, so handlers may re-enter.
fn get(id: SurfaceId) -> Option<Rc<Surface>> {
    SURFACES.with(|m| m.borrow().get(id.0).cloned())
}

fn find(f: impl Fn(&Surface) -> bool) -> Option<Rc<Surface>> {
    SURFACES.with(|m| m.borrow().values().find(|s| f(s)).cloned())
}

fn by_window(window: &AnyObject) -> Option<Rc<Surface>> {
    find(|s| std::ptr::eq(Retained::as_ptr(&s.v.panel).cast(), window))
}

fn by_text(view: &NSTextView) -> Option<Rc<Surface>> {
    find(|s| s.v.field.is(view))
}

/// Offer `k` to the module, then apply the fallback. True when the key is used up.
fn handle(s: &Rc<Surface>, k: Keystroke) -> bool {
    (s.handlers.key)(s.id, k)
        || match keys::fallback(k, s.multi) {
            Fallback::Submit => {
                submit(s);
                true
            }
            Fallback::Close => {
                close(s);
                true
            }
            Fallback::Default => false,
        }
}

/// Clear the input and hand its text to `submit`; blank input does nothing.
fn submit(s: &Surface) {
    let text = s.v.field.text();
    if text.trim().is_empty() {
        return;
    }
    s.v.field.set_text("");
    let (id, h) = (s.id, s.handlers);
    // Moved out once, not cloned: no stray copy of the text is left behind.
    let text = Cell::new(Some(text));
    timer::after(0.0, move || {
        if let Some(text) = text.take() {
            (h.submit)(id, text);
        }
    });
}

/// Hide on the user's behalf and tell `closed`.
fn close(s: &Surface) {
    s.v.panel.orderOut(None);
    let (id, h) = (s.id, s.handlers);
    timer::after(0.0, move || (h.closed)(id));
}

/// Lay out the content for its current size, chips and notice.
fn relayout(s: &Surface) {
    let b = s.v.root.bounds().size;
    let chips = s.chips.borrow();
    let widths: Vec<f64> = chips.iter().map(|c| c.fittingSize().width.ceil()).collect();
    let notice = !s.v.notice.isHidden();
    let parts = Parts { chips: &widths, notice, multi: s.multi, banner: s.v.banner.is_some() };
    let l = geometry::layout(Size { w: b.width, h: b.height }, parts);
    s.v.title.setFrame(ns_rect(l.title));
    s.v.subtitle.setFrame(ns_rect(l.subtitle));
    s.v.dot.setFrame(ns_rect(l.dot));
    s.v.rule.setFrame(ns_rect(l.rule));
    if let (Some((back, label)), Some(b)) = (&s.v.banner, l.banner) {
        back.setFrame(ns_rect(b));
        label.setFrame(ns_rect(Rect::new(b.x, b.y + 4.0, b.w, b.h - 8.0)));
    }
    s.v.rows.set_frame(l.body);
    if let Some(n) = l.notice {
        s.v.notice.setFrame(ns_rect(n));
    }
    for (chip, frame) in chips.iter().zip(&l.chips) {
        chip.setHidden(frame.is_none());
        if let Some(f) = frame {
            chip.setFrame(ns_rect(*f));
        }
    }
    s.v.field.set_frame(l.input);
}

/// Build surface `id` from `spec`, hidden. A second `open` of the same id does nothing.
pub fn open(id: SurfaceId, spec: &Spec, handlers: Handlers) {
    if get(id).is_some() {
        return;
    }
    super::hud::embed::on_press(card_pressed);
    let v = window::build(mtm(), spec, geometry::min_size(spec.min_size));
    let multi = spec.input == Input::Multi;
    let (hide_on_blur, private) = (spec.hide_on_blur, spec.private.is_some());
    let chips = RefCell::default();
    let s = Rc::new(Surface { id, handlers, multi, hide_on_blur, private, v, chips });
    relayout(&s);
    SURFACES.with(|m| m.borrow_mut().insert(id.0, s));
}

/// Move `s` onto the screen under the pointer, keeping it there if it already is.
fn place(s: &Surface) {
    let mouse = NSEvent::mouseLocation();
    let f = s.v.panel.frame();
    let now = Rect::new(f.origin.x, f.origin.y, f.size.width, f.size.height);
    let min = s.v.panel.contentMinSize();
    let min = Size { w: min.width, h: min.height };
    let to = geometry::place(now, min, &window::screens(mtm()), (mouse.x, mouse.y));
    if to != now {
        s.v.panel.setFrame_display(ns_rect(to), true);
    }
}

/// Show `id` on the screen under the pointer and make it key, with the caret in the input.
/// Flick does not activate: the app in front stays active.
pub fn show(id: SurfaceId) {
    let Some(s) = get(id) else { return };
    s.v.rows.flush();
    place(&s);
    s.v.panel.makeKeyAndOrderFront(None);
    let input = s.v.field.responder();
    let focused = s.v.panel.firstResponder().is_some_and(|r| std::ptr::eq(&*r, input));
    if !focused {
        s.v.panel.makeFirstResponder(Some(input));
    }
}

/// Hide `id` without calling `closed`.
pub fn hide(id: SurfaceId) {
    if let Some(s) = get(id) {
        s.v.panel.orderOut(None);
    }
}

pub fn is_visible(id: SurfaceId) -> bool {
    get(id).is_some_and(|s| s.v.panel.isVisible())
}

/// Whether `id` has the keyboard.
pub fn is_key(id: SurfaceId) -> bool {
    get(id).is_some_and(|s| s.v.panel.isKeyWindow())
}

/// Draw `id`'s content into a PNG at `path`, shown or not. A private surface refuses: its
/// content never goes to disk.
pub fn snapshot(id: SurfaceId, path: &str) -> Result<(), String> {
    let s = get(id).ok_or("no such surface")?;
    if s.private {
        return Err("private surface".into());
    }
    s.v.rows.flush();
    let bounds = s.v.root.bounds();
    let rep =
        s.v.root.bitmapImageRepForCachingDisplayInRect(bounds).ok_or("can't create bitmap")?;
    s.v.root.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    // SAFETY: an empty properties dictionary is valid for PNG encoding.
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or("can't encode PNG")?;
    png.writeToFile_atomically(&ns(path), true).then_some(()).ok_or(format!("can't write {path}"))
}

pub fn set_header(id: SurfaceId, header: &Header) {
    let Some(s) = get(id) else { return };
    s.v.title.setStringValue(&ns(header.title));
    s.v.subtitle.setStringValue(&ns(header.subtitle));
    let color = match header.status {
        Status::Idle => NSColor::tertiaryLabelColor(),
        Status::Busy => NSColor::systemOrangeColor(),
        Status::Error => NSColor::systemRedColor(),
    };
    s.v.dot.setFillColor(&color);
}

/// Replace the chips; each shows its symbol, its label and a remove mark.
pub fn set_chips(id: SurfaceId, chips: &[Chip]) {
    let Some(s) = get(id) else { return };
    let mtm = mtm();
    let made: Vec<Retained<NSButton>> = chips
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let b = window::chip_button(mtm, c, i);
            s.v.root.addSubview(&b);
            b
        })
        .collect();
    for old in s.chips.replace(made) {
        old.removeFromSuperview();
    }
    relayout(&s);
}

/// Replace the transcript with `rows`, top to bottom (see the module docs).
pub fn set_rows(id: SurfaceId, rows: &[Row]) {
    let Some(s) = get(id) else { return };
    if s.v.rows.set(rows.iter().map(Owned::new).collect()) {
        timer::after(COALESCE_SECS, move || {
            if let Some(s) = get(id) {
                s.v.rows.flush();
            }
        });
    }
}

/// A press on an embedded card (`hud::embed::on_press`): find its surface, then hand the
/// press to `card_action`, or show the error if its values are over the cap.
fn card_pressed(host: usize, tag: isize) {
    let Some(s) = find(|s| s.v.rows.has_card(host)) else { return };
    match s.v.rows.press(host, tag) {
        Some((card, action, Ok(values))) => (s.handlers.card_action)(s.id, &card, &action, values),
        Some((_, _, Err(why))) => s.v.rows.show_error(host, &why),
        None => {}
    }
}

/// The input's text as typed so far.
pub fn input(id: SurfaceId) -> String {
    get(id).map(|s| s.v.field.text()).unwrap_or_default()
}

/// Replace the input's text (an unchanged text keeps the caret).
pub fn set_input(id: SurfaceId, text: &str) {
    if let Some(s) = get(id) {
        s.v.field.set_text(text);
    }
}

/// A one-line notice above the chips (a permission hint, a failure); `None` removes it.
pub fn set_notice(id: SurfaceId, notice: Option<&str>) {
    let Some(s) = get(id) else { return };
    s.v.notice.setStringValue(&ns(notice.unwrap_or("")));
    s.v.notice.setHidden(notice.is_none());
    relayout(&s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_surfaces_are_hidden_and_empty() {
        let id = SurfaceId("test-none");
        assert!(!is_visible(id) && !is_key(id));
        assert_eq!(input(id), "");
        assert_eq!(snapshot(id, "/nonexistent.png"), Err("no such surface".into()));
    }
}
