//! What `help` does to the system: open a URL, copy text. Tests swap in plain functions.

use crate::platform::{pasteboard, workspace};

pub struct Hooks {
    pub open: fn(&str),
    pub copy: fn(&str),
}

impl Default for Hooks {
    fn default() -> Self {
        Hooks { open: workspace::open_url, copy: pasteboard::set_text }
    }
}
