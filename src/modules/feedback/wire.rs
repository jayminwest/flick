//! What `feedback` reads from and does to the system: the clock, the front app and the
//! pasteboard. `Env::default()` is the real thing; tests swap in plain functions.

use std::path::PathBuf;

use crate::core::store;
use crate::platform::{clock, pasteboard, workspace};

/// The front app: (bundle id, name).
pub type App = (Option<String>, String);

pub struct Env {
    pub now: fn() -> i64,
    pub utc_offset: fn(i64) -> i32,
    /// The app in front, unless it is Flick.
    pub frontmost: fn() -> Option<App>,
    pub copy: fn(&str),
    /// The checkout this binary was built from (`FLICK_BUILD_SOURCE`, else the crate).
    pub source: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// `FLICK_BUILD_SHA` and `FLICK_BUILD_DIRTY` as one label (`entry::build`).
    pub build: String,
}

impl Default for Env {
    fn default() -> Self {
        let source = option_env!("FLICK_BUILD_SOURCE").filter(|s| !s.is_empty());
        Env {
            now: store::now,
            utc_offset: clock::utc_offset,
            frontmost,
            copy: pasteboard::set_text,
            source: Some(PathBuf::from(source.unwrap_or(env!("CARGO_MANIFEST_DIR")))),
            home: dirs::home_dir(),
            build: super::entry::build(
                option_env!("FLICK_BUILD_SHA"),
                option_env!("FLICK_BUILD_DIRTY"),
            ),
        }
    }
}

fn frontmost() -> Option<App> {
    let pid = workspace::frontmost_pid()?;
    if u32::try_from(pid).ok() == Some(std::process::id()) {
        return None;
    }
    workspace::app_identity(pid)
}
