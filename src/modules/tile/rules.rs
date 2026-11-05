//! Which windows tile manages, and how: app pins (bundle id to workspace), float rules, and
//! the dialog and popup heuristics. Pure Rust over facts winman reads from each window.
//!
//! The dialog table and heuristics port `AeroSpace`'s `isDialogHeuristic` and
//! `isWindowHeuristic` (Sources/AppBundle/model/AxUiElementWindowType.swift and
//! KnownBundleId.swift, commit bc6be84), under its license:
//!
//! MIT License
//!
//! Copyright (c) 2023 Nikita Bobko
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all
//! copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! SOFTWARE.

use super::workspace::Ws;

/// The `AXSubrole` of a regular window.
pub const STANDARD: &str = "AXStandardWindow";

/// Apps whose windows always float: Flick's own panels and System Settings.
const FLOAT: [&str; 2] = ["com.jayminwest.flick", "com.apple.systempreferences"];

/// Apps whose regular windows have no enabled fullscreen button, so that test would float
/// them all: GIMP and Activity Monitor show none, Chrome re-enables it late on a dragged-out
/// tab, terminals, qutebrowser, Emacs and VS Code can hide their title bars, and Steam.
const NO_FULLSCREEN_BUTTON: [&str; 12] = [
    "org.gimp.gimp-2.10",
    "com.google.Chrome",
    "com.apple.ActivityMonitor",
    "org.alacritty",
    "net.kovidgoyal.kitty",
    "com.github.wez.wezterm",
    "org.qutebrowser.qutebrowser",
    "com.googlecode.iterm2",
    "org.gnu.Emacs",
    "com.microsoft.VSCode",
    "com.vscodium",
    "com.valvesoftware.steam.helper",
];

/// Apps whose regular windows report a subrole other than `STANDARD`: qutebrowser says
/// `AXDialog` when its decorations are off.
const NONSTANDARD_SUBROLE: [&str; 1] = ["org.qutebrowser.qutebrowser"];

/// The state of a window's fullscreen (green) button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Missing,
    Disabled,
    Enabled,
}

/// What the rules read from one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts<'a> {
    /// The app's bundle id; empty when it has none.
    pub bundle: &'a str,
    /// `AXSubrole`, e.g. `STANDARD` or `AXDialog`.
    pub subrole: &'a str,
    pub fullscreen: Button,
    /// It has at least one of the close, minimize, zoom and fullscreen buttons.
    pub buttons: bool,
    /// It is focused or its app's main window.
    pub focused: bool,
    /// Its window level is the normal one (popups sit above it).
    pub normal_level: bool,
}

/// How tile treats a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A tile in its workspace's tree.
    Tile,
    /// On a workspace, but outside the tree: tile hides and shows it, never lays it out.
    Float,
    /// Not a real window (a menu, tooltip or indicator): tile leaves it alone.
    Popup,
}

/// App pins and float rules, from the built-ins and config.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pins: Vec<(String, Ws)>,
    float: Vec<String>,
}

impl Rules {
    /// `pins`: bundle id to the workspace its new windows open on. `float`: bundle ids
    /// whose windows float, on top of the built-in `FLOAT`.
    pub fn new(pins: Vec<(String, Ws)>, float: Vec<String>) -> Rules {
        Rules { pins, float }
    }

    /// The workspace `bundle`'s new windows open on, when it is pinned.
    pub fn pin(&self, bundle: &str) -> Option<Ws> {
        self.pins.iter().find(|(b, _)| b == bundle).map(|(_, ws)| *ws)
    }

    pub fn kind(&self, f: &Facts) -> Kind {
        if is_popup(f) {
            Kind::Popup
        } else if self.floats(f.bundle) || is_dialog(f) {
            Kind::Float
        } else {
            Kind::Tile
        }
    }

    /// Every window of `bundle` floats.
    fn floats(&self, bundle: &str) -> bool {
        FLOAT.contains(&bundle) || self.float.iter().any(|b| b == bundle)
    }
}

/// No buttons, no focus and above the normal level: a context menu, tooltip or the
/// screen-share indicator.
fn is_popup(f: &Facts) -> bool {
    !f.buttons && !f.focused && !f.normal_level
}

/// A dialog floats: any subrole but `STANDARD` (file pickers, find bars, the login screen),
/// or no enabled fullscreen button (windows not designed to be big: settings, About this
/// Mac, copy progress), except in the apps known to report these wrongly.
fn is_dialog(f: &Facts) -> bool {
    let subrole = f.subrole != STANDARD && !NONSTANDARD_SUBROLE.contains(&f.bundle);
    let small = f.fullscreen != Button::Enabled && !NO_FULLSCREEN_BUTTON.contains(&f.bundle);
    subrole || small
}

#[cfg(test)]
mod tests;
