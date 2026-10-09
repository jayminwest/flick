//! Window management through the Accessibility API.
//!
//! Geometry uses the Accessibility coordinate space: origin at the top-left of the
//! primary screen, y grows downward.

use std::path::PathBuf;

pub use crate::platform::Rect;
use crate::platform::{ax, screens, workspace};

impl Rect {
    fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Equal within a few points; apps round their frames.
    fn near(&self, o: Rect) -> bool {
        [self.x - o.x, self.y - o.y, self.w - o.w, self.h - o.h].iter().all(|d| d.abs() <= 4.0)
    }

    fn contains(&self, (px, py): (f64, f64)) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    FirstThird,
    CenterThird,
    LastThird,
    FirstTwoThirds,
    LastTwoThirds,
    Maximize,
    AlmostMaximize,
    Center,
    NextDisplay,
    PreviousDisplay,
    Minimize,
    Hide,
}

impl WindowAction {
    pub const ALL: [WindowAction; 20] = [
        Self::LeftHalf,
        Self::RightHalf,
        Self::TopHalf,
        Self::BottomHalf,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::FirstThird,
        Self::CenterThird,
        Self::LastThird,
        Self::FirstTwoThirds,
        Self::LastTwoThirds,
        Self::Maximize,
        Self::AlmostMaximize,
        Self::Center,
        Self::NextDisplay,
        Self::PreviousDisplay,
        Self::Minimize,
        Self::Hide,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::LeftHalf => "Left Half",
            Self::RightHalf => "Right Half",
            Self::TopHalf => "Top Half",
            Self::BottomHalf => "Bottom Half",
            Self::TopLeft => "Top Left Quarter",
            Self::TopRight => "Top Right Quarter",
            Self::BottomLeft => "Bottom Left Quarter",
            Self::BottomRight => "Bottom Right Quarter",
            Self::FirstThird => "First Third",
            Self::CenterThird => "Center Third",
            Self::LastThird => "Last Third",
            Self::FirstTwoThirds => "First Two Thirds",
            Self::LastTwoThirds => "Last Two Thirds",
            Self::Maximize => "Maximize",
            Self::AlmostMaximize => "Almost Maximize",
            Self::Center => "Center",
            Self::NextDisplay => "Next Display",
            Self::PreviousDisplay => "Previous Display",
            Self::Minimize => "Minimize",
            Self::Hide => "Hide",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::LeftHalf => "rectangle.lefthalf.inset.filled",
            Self::RightHalf => "rectangle.righthalf.inset.filled",
            Self::TopHalf => "rectangle.tophalf.inset.filled",
            Self::BottomHalf => "rectangle.bottomhalf.inset.filled",
            Self::TopLeft => "rectangle.inset.topleft.filled",
            Self::TopRight => "rectangle.inset.topright.filled",
            Self::BottomLeft => "rectangle.inset.bottomleft.filled",
            Self::BottomRight => "rectangle.inset.bottomright.filled",
            Self::FirstThird | Self::FirstTwoThirds => "rectangle.leadingthird.inset.filled",
            Self::CenterThird | Self::Center => "rectangle.center.inset.filled",
            Self::LastThird | Self::LastTwoThirds => "rectangle.trailingthird.inset.filled",
            Self::Maximize => "rectangle.inset.filled",
            Self::AlmostMaximize => "rectangle.dashed",
            Self::NextDisplay | Self::PreviousDisplay => "display.2",
            Self::Minimize => "minus.square",
            Self::Hide => "eye.slash",
        }
    }

    /// Config name: the title in kebab case, e.g. "left-half".
    pub fn slug(self) -> String {
        self.title().to_lowercase().replace(' ', "-")
    }

    pub fn from_slug(slug: &str) -> Option<WindowAction> {
        Self::ALL.into_iter().find(|a| a.slug() == slug)
    }

    /// Target frame within the screen's usable `area`, given the window's `current` frame.
    /// Display moves, minimize, and hide are handled by `apply`, so here they keep the frame.
    pub fn target(self, a: Rect, current: Rect) -> Rect {
        let (hw, hh, tw) = (a.w / 2.0, a.h / 2.0, a.w / 3.0);
        let r = |x: f64, y: f64, w: f64, h: f64| Rect { x: a.x + x, y: a.y + y, w, h };
        match self {
            Self::LeftHalf => r(0.0, 0.0, hw, a.h),
            Self::RightHalf => r(hw, 0.0, hw, a.h),
            Self::TopHalf => r(0.0, 0.0, a.w, hh),
            Self::BottomHalf => r(0.0, hh, a.w, hh),
            Self::TopLeft => r(0.0, 0.0, hw, hh),
            Self::TopRight => r(hw, 0.0, hw, hh),
            Self::BottomLeft => r(0.0, hh, hw, hh),
            Self::BottomRight => r(hw, hh, hw, hh),
            Self::FirstThird => r(0.0, 0.0, tw, a.h),
            Self::CenterThird => r(tw, 0.0, tw, a.h),
            Self::LastThird => r(2.0 * tw, 0.0, tw, a.h),
            Self::FirstTwoThirds => r(0.0, 0.0, 2.0 * tw, a.h),
            Self::LastTwoThirds => r(tw, 0.0, 2.0 * tw, a.h),
            Self::Maximize => a,
            Self::AlmostMaximize => r(a.w * 0.05, a.h * 0.05, a.w * 0.9, a.h * 0.9),
            Self::Center => {
                let (w, h) = (current.w.min(a.w), current.h.min(a.h));
                r((a.w - w) / 2.0, (a.h - h) / 2.0, w, h)
            }
            Self::NextDisplay | Self::PreviousDisplay | Self::Minimize | Self::Hide => current,
        }
    }

    /// Like `target`, but repeating a half cycles its size through 1/2, 2/3, 1/3 (as Rectangle does).
    #[expect(
        clippy::items_after_statements,
        clippy::unwrap_used,
        reason = "SIZES[0] is checked to frame above, so every size frames"
    )]
    pub fn cycled(self, a: Rect, current: Rect) -> Rect {
        let frame = |f: f64| {
            let r = match self {
                Self::LeftHalf => Rect { w: a.w * f, ..a },
                Self::RightHalf => Rect { x: a.x + a.w * (1.0 - f), w: a.w * f, ..a },
                Self::TopHalf => Rect { h: a.h * f, ..a },
                Self::BottomHalf => Rect { y: a.y + a.h * (1.0 - f), h: a.h * f, ..a },
                _ => return None,
            };
            Some(Rect { x: r.x.round(), y: r.y.round(), w: r.w.round(), h: r.h.round() })
        };
        const SIZES: [f64; 3] = [1.0 / 2.0, 2.0 / 3.0, 1.0 / 3.0];
        let Some(_) = frame(SIZES[0]) else { return self.target(a, current) };
        let at = SIZES.iter().position(|&f| frame(f).is_some_and(|r| r.near(current)));
        frame(SIZES[at.map_or(0, |i| (i + 1) % SIZES.len())]).unwrap()
    }
}

/// Map `current` from screen area `from` to the same relative spot on `to`.
fn move_between(current: Rect, from: Rect, to: Rect) -> Rect {
    let w = current.w.min(to.w);
    let h = current.h.min(to.h);
    let fx = if from.w > current.w { (current.x - from.x) / (from.w - current.w) } else { 0.0 };
    let fy = if from.h > current.h { (current.y - from.y) / (from.h - current.h) } else { 0.0 };
    Rect {
        x: to.x + fx.clamp(0.0, 1.0) * (to.w - w),
        y: to.y + fy.clamp(0.0, 1.0) * (to.h - h),
        w,
        h,
    }
}

pub use crate::platform::ax::{ensure_trusted, is_trusted, send_paste};

/// The frame `action` gives a window at `current`, on the screen (of usable `areas`) that
/// holds its center, else the first. Display moves wrap around.
pub fn frame_for(
    action: WindowAction,
    current: Rect,
    areas: &[Rect],
) -> Result<Rect, &'static str> {
    let index = areas.iter().position(|a| a.contains(current.center())).unwrap_or(0);
    let area = *areas.get(index).ok_or("No screens")?;
    let n = areas.len();
    Ok(match action {
        WindowAction::NextDisplay => move_between(current, area, areas[(index + 1) % n]),
        WindowAction::PreviousDisplay => move_between(current, area, areas[(index + n - 1) % n]),
        _ => action.cycled(area, current),
    })
}

pub fn apply(action: WindowAction) -> Result<(), &'static str> {
    // Hide acts on the app, like cmd+H: instant, and needs no Accessibility permission.
    if action == WindowAction::Hide {
        return workspace::hide_frontmost();
    }
    if !ensure_trusted() {
        return Err("Flick needs Accessibility permission");
    }
    let win = ax::focused_window().ok_or("No focused window")?;
    if action == WindowAction::Minimize {
        win.minimize();
        return Ok(());
    }
    let current = win.frame().ok_or("Can't read window frame")?;
    let target = frame_for(action, current, &screens::visible_areas())?;
    win.set_frame(target);
    Ok(())
}

// --- Window switcher ---

/// A window to switch to. `element` is None for an app whose windows are on another
/// desktop: Accessibility only lists windows on the current one.
pub struct AppWindow {
    pub pid: i32,
    pub title: String,
    pub app: String,
    pub bundle: Option<PathBuf>,
    pub minimized: bool,
    element: Option<ax::AxWindow>,
}

impl AppWindow {
    pub fn on_other_desktop(&self) -> bool {
        self.element.is_none()
    }
}

/// Standard windows of every regular app, apps in `recent` order (most recent first).
/// The frontmost window goes last, so the first entry is the previous window.
pub fn list_windows(recent: &[i32], frontmost: Option<i32>) -> Vec<AppWindow> {
    if !ensure_trusted() {
        return vec![];
    }
    let me = std::process::id() as i32;
    let elsewhere = crate::platform::spaces::window_pids(false);
    let mut out = Vec::new();
    for app in workspace::regular_apps(me) {
        let (pid, name, bundle) = (app.pid, app.name, app.bundle);
        let before = out.len();
        for win in ax::standard_windows(pid) {
            let title = win.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| name.clone());
            out.push(AppWindow {
                pid,
                title,
                app: name.clone(),
                bundle: bundle.clone(),
                minimized: win.minimized,
                element: Some(win),
            });
        }
        if out.len() == before && elsewhere.contains(&pid) && !app.hidden {
            out.push(AppWindow {
                pid,
                title: name.clone(),
                app: name,
                bundle,
                minimized: false,
                element: None,
            });
        }
    }

    let rank = |pid: i32| recent.iter().position(|&p| p == pid).unwrap_or(usize::MAX);
    out.sort_by_key(|w| rank(w.pid)); // stable: keeps each app's front-to-back order
    if let Some(i) = out.iter().position(|w| Some(w.pid) == frontmost) {
        let current = out.remove(i);
        out.push(current);
    }
    out
}

/// Raise `w` and activate its app (switching desktops if needed).
pub fn focus(w: &AppWindow) {
    ax::focus(w.pid, w.element.as_ref(), w.minimized);
}
