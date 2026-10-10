//! Context attach (flick-65bd): what the next question carries besides its text, each piece a
//! removable chip above the input. Nothing goes without its chip showing.
//!
//! - Summoned from hidden, the window reads the app in front (`platform::context::front`): its
//!   name and bundle id, its focused window's title and its selected text (Accessibility,
//!   never a synthetic ⌘C), one chip each, all on by default. A new summon replaces every
//!   chip; clicking one removes it.
//! - Each of these keys clears the notice line first.
//! - ⌘⇧V adds the clipboard text (replacing an earlier clipboard chip); a concealed or
//!   transient clipboard (password managers) has none.
//! - ⌘⇧S screenshots the display under the pointer with the window out of the way
//!   (`context::shoot_display`). The PNG is read into memory and the temp file deleted at
//!   once; at most `MAX_SHOTS` per question. Without Screen Recording the notice says how to
//!   grant it (and macOS is asked once per run).
//! - On Return the chips go with the question (`message ask` sends none): each screenshot is
//!   uploaded first, `n` from 1 in chip order, to `<attach_dir>/<req>-<n>.png` on the kota
//!   host (`attach::upload_argv`, one ssh with the PNG on stdin, 30 s), in the same worker
//!   job and before kota-ask runs; a failed upload fails the ask and kota-ask never runs.
//!   The `[context]` block (`ask::compose`, capped) names `~/<attach_dir>/<req>-<n>.png`.
//! - A question that did not go out keeps its chips (the last `KEPT` such questions) so ⌘R
//!   sends them again, under the same paths.

use std::sync::Arc;

use super::{ask, attach};
use crate::core::Cx;
use crate::modules::message::Inbox;
use crate::modules::message::run::Exit;
use crate::platform::context::Front;
use crate::platform::surface;

/// Screenshots one question may carry.
pub const MAX_SHOTS: usize = 3;
/// Failed questions whose chips are kept for ⌘R.
pub const KEPT: usize = 4;

/// One chip: a piece of context the next question carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attached {
    /// Sent in the `[context]` block as it is.
    Item(ask::Item),
    /// A screenshot's PNG, held until the question uploads it.
    Shot(Arc<[u8]>),
}

impl Attached {
    fn chip(&self) -> ask::Chip {
        match self {
            Attached::Item(i) => i.chip(),
            Attached::Shot(_) => ask::Item::Screenshot(String::new()).chip(),
        }
    }
}

/// One screenshot upload: its argv (the PNG goes on stdin).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upload {
    pub argv: Vec<String>,
    pub png: Arc<[u8]>,
}

/// The summon-time chips of `front`: app, window title, selection.
pub fn from_front(front: Front) -> Vec<Attached> {
    let app = ask::Item::App { name: front.app, bundle: front.bundle_id };
    let rest = [front.window.map(ask::Item::Window), front.selection.map(ask::Item::Selection)];
    std::iter::once(app).chain(rest.into_iter().flatten()).map(Attached::Item).collect()
}

/// What question `req` sends with `attached`: the `[context]` items, each screenshot named by
/// its remote path, and the uploads that must come first.
pub fn plan(attached: &[Attached], host: &str, dir: &str, req: &str) -> Result<(Vec<ask::Item>, Vec<Upload>), String> {
    let (mut items, mut uploads) = (Vec::new(), Vec::new());
    for a in attached {
        match a {
            Attached::Item(i) => items.push(i.clone()),
            Attached::Shot(png) => {
                let n = u32::try_from(uploads.len() + 1).unwrap_or(u32::MAX);
                uploads.push(Upload { argv: attach::upload_argv(host, dir, req, n)?, png: Arc::clone(png) });
                items.push(ask::Item::Screenshot(attach::path(dir, req, n)?));
            }
        }
    }
    Ok((items, uploads))
}

/// Run every upload with `upload` (on the worker), stopping at the first that fails.
pub fn upload_all(upload: fn(&[String], &[u8]) -> Exit, uploads: &[Upload]) -> Result<(), String> {
    for u in uploads {
        match upload(&u.argv, &u.png) {
            Exit::Sent => {}
            Exit::Rejected(why) | Exit::Failed(why) => return Err(format!("screenshot upload failed: {why}")),
        }
    }
    Ok(())
}

impl Inbox {
    /// Summoned from hidden: the chips of the app in front, replacing all others.
    pub(super) fn attach_front(&mut self) {
        self.chat.attached = (self.chat.hooks.context)().map(from_front).unwrap_or_default();
    }

    /// ⌘⇧V: the clipboard text as a chip, replacing an earlier one.
    pub(super) fn attach_clipboard(&mut self) {
        self.chat.notice = None;
        let text = (self.chat.hooks.clipboard)().filter(|t| !t.trim().is_empty());
        let Some(text) = text else {
            self.chat.notice = Some("The clipboard holds no text".into());
            return;
        };
        let item = Attached::Item(ask::Item::Clipboard(text));
        let at = self.chat.attached.iter().position(|a| matches!(a, Attached::Item(ask::Item::Clipboard(_))));
        match at {
            Some(i) => self.chat.attached[i] = item,
            None => self.chat.attached.push(item),
        }
        self.chips();
    }

    /// ⌘⇧S: take a screenshot unless the question has its fill; it arrives as `Note::Shot`.
    pub(super) fn attach_screenshot(&mut self) {
        self.chat.notice = None;
        if self.chat.attached.iter().filter(|a| matches!(a, Attached::Shot(_))).count() >= MAX_SHOTS {
            self.chat.notice = Some(format!("At most {MAX_SHOTS} screenshots per question"));
            return;
        }
        (self.chat.hooks.shoot)();
    }

    /// A screenshot arrived (or why there is none).
    pub(super) fn shot(&mut self, png: Result<Arc<[u8]>, String>) {
        let checked = png.and_then(|p| attach::check_size(p.len() as u64).map(|()| p));
        match checked {
            Ok(png) => {
                self.chat.attached.push(Attached::Shot(png));
                self.chips();
            }
            Err(why) => self.chat.notice = Some(why),
        }
    }

    /// A chip was clicked: drop it.
    pub(super) fn unchip(&mut self, index: usize) {
        if index < self.chat.attached.len() {
            self.chat.attached.remove(index);
            self.chips();
        }
    }

    /// Keep the chips of question `req`, which did not go out, for ⌘R.
    pub(super) fn keep_unsent(&mut self, req: &str, attached: Vec<Attached>) {
        if attached.is_empty() {
            return;
        }
        self.chat.unsent.retain(|(r, _)| r != req);
        if self.chat.unsent.len() == KEPT {
            self.chat.unsent.pop_front();
        }
        self.chat.unsent.push_back((req.to_string(), attached));
    }

    /// The chips question `req` was asked with, if it did not go out.
    pub(super) fn take_unsent(&mut self, req: &str) -> Vec<Attached> {
        let at = self.chat.unsent.iter().position(|(r, _)| r == req);
        at.and_then(|i| self.chat.unsent.remove(i)).map(|(_, a)| a).unwrap_or_default()
    }

    /// Draw the chips.
    pub(super) fn chips(&self) {
        let chips: Vec<ask::Chip> = self.chat.attached.iter().map(Attached::chip).collect();
        let chips: Vec<surface::Chip> = chips.iter().map(|c| surface::Chip { label: &c.label, symbol: c.symbol }).collect();
        (self.chat.hooks.chips)(&chips);
    }

    /// Return in the window: ask with the chips, which are used up; a refused question keeps
    /// them and its text.
    pub(super) fn submit(&mut self, text: &str, cx: &Cx) {
        let thread = self.current_thread(cx);
        let attached = std::mem::take(&mut self.chat.attached);
        match self.ask_with(&thread, text, attached.clone(), false, cx) {
            Ok(_) => self.chips(),
            Err(e) => {
                self.chat.attached = attached;
                (self.chat.hooks.set_input)(text);
                self.chat.notice = Some(e);
            }
        }
    }
}

#[cfg(test)]
mod tests;
