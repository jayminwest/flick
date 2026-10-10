//! What `message` does to the system: the clock, the HUD, notifications, the pasteboard and
//! opening links. `Env::default()` is the real thing; tests swap in plain functions.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::store;
use crate::platform::hud::{self, Content, Options, Placement};
use crate::platform::{clock, notify, pasteboard, workspace};

pub struct Env {
    pub now: fn() -> i64,
    pub utc_offset: fn(i64) -> i32,
    /// A fresh message id.
    pub new_id: fn() -> String,
    /// Show or redraw the card with this id; true when it is new.
    pub show: fn(&str, &Content, &Placement, &Options) -> bool,
    /// Remove one card (a pending post its reply replaces); true when it showed.
    pub dismiss: fn(&str) -> bool,
    /// Remove every card.
    pub hide: fn(),
    /// (id, title, body) as a system notification.
    pub notify: fn(&str, &str, &str),
    pub copy: fn(&str),
    pub open_url: fn(&str),
}

impl Default for Env {
    fn default() -> Self {
        Env {
            now: store::now,
            utc_offset: clock::utc_offset,
            new_id,
            show: hud::show,
            dismiss: hud::dismiss,
            hide: hud::dismiss_all,
            notify: |id, title, body| {
                if let Err(e) = notify::post(&format!("message:{id}"), title, body) {
                    eprintln!("flick: message: notification: {e}");
                }
            },
            copy: pasteboard::set_text,
            open_url: workspace::open_url,
        }
    }
}

/// `m` and the time in milliseconds, base 36, plus a counter so two posts in one
/// millisecond differ.
fn new_id() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let n = N.fetch_add(1, Ordering::Relaxed) % 36;
    format!("m{}{}", base36(ms), base36(u128::from(n)))
}

fn base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = vec![];
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_short_and_distinct() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(36 * 36 + 35), "10z");
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert!(a.starts_with('m') && crate::modules::message::text::valid_id(&a), "{a}");
    }
}
