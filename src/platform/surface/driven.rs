//! A driven instance's reach into surfaces (`crate::drive`): there `show` puts nothing on
//! the screen, so a surface only counts as shown, as the launcher panel does. Each surface
//! reads as text (`scene`), and keys, input and submit take the user's paths: `key` goes
//! to `Handlers::key` and then the fallback, `submit` is Return's submit.

use std::rc::Rc;

use super::outline::{self, Outline};
use super::{Keystroke, SURFACES, Surface};

/// The surface named `name`, else an error naming the open ones.
fn named(name: &str) -> Result<Rc<Surface>, String> {
    SURFACES.with(|m| {
        let m = m.borrow();
        m.get(name).cloned().ok_or_else(|| {
            let mut names: Vec<&str> = m.keys().copied().collect();
            names.sort_unstable();
            format!("no surface \"{name}\" (open: {})", names.join(", "))
        })
    })
}

/// Every open surface by name, as `outline::surface` writes it. Empty with none.
pub fn scene() -> String {
    let mut all: Vec<Rc<Surface>> = SURFACES.with(|m| m.borrow().values().cloned().collect());
    all.sort_unstable_by_key(|s| s.id.0);
    all.iter().map(|s| text(s)).collect()
}

fn text(s: &Surface) -> String {
    let rows = s.v.rows.owned();
    let chips: Vec<String> = s
        .chips
        .borrow()
        .iter()
        .map(|c| c.title().to_string().trim_end_matches('\u{d7}').trim_end().to_string())
        .collect();
    let (title, subtitle) = (s.v.title.stringValue().to_string(), s.v.subtitle.stringValue());
    let notice = (!s.v.notice.isHidden()).then(|| s.v.notice.stringValue().to_string());
    outline::surface(&Outline {
        name: s.id.0,
        shown: super::is_visible(s.id),
        private: s.private,
        title: &title,
        subtitle: &subtitle.to_string(),
        status: s.status.get(),
        rows: &rows,
        notice: notice.as_deref(),
        chips: &chips,
        input: &s.v.field.text(),
    })
}

/// Offer `key` to surface `name` as a key in its input: true when its module or the
/// fallback took it.
pub fn key(name: &str, key: Keystroke) -> Result<bool, String> {
    Ok(super::handle(&named(name)?, key))
}

/// Replace the input's text of surface `name`.
pub fn input(name: &str, text: &str) -> Result<(), String> {
    named(name).map(|s| s.v.field.set_text(text))
}

/// Submit the input of surface `name` as Return does: it clears and `Handlers::submit`
/// gets the text. A blank input is an error.
pub fn submit(name: &str) -> Result<(), String> {
    let s = named(name)?;
    if s.v.field.text().trim().is_empty() {
        return Err(format!("surface \"{name}\": the input is empty"));
    }
    super::submit(&s);
    Ok(())
}

/// Write surface `name` to a PNG at `path` (`snapshot`).
pub fn shot(name: &str, path: &str) -> Result<(), String> {
    super::snapshot(named(name)?.id, path)
}
