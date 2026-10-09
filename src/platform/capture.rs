//! Screenshots through `/usr/sbin/screencapture` (as `hid` drives `hidutil`).
//!
//! screencapture brings the native selection UI: crosshair, Space toggles window mode, Esc
//! cancels, every display, Retina pixels and window shadows. An interactive run that the user
//! cancels exits 0 and writes no file; `run` reports that as `Error::Cancelled`.
//!
//! Capturing other apps' windows needs the Screen Recording permission (`permitted`,
//! `request_permission`). Without it, macOS hands back only the wallpaper and Flick's own
//! windows. `run` blocks until screencapture exits: never call it with `Target::Area` or
//! `Target::Window` on the main thread.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use objc2_app_kit::{NSEvent, NSScreen};

use super::Rect;

const SCREENCAPTURE: &str = "/usr/sbin/screencapture";

/// What to capture.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 5 (flick-e24c)"))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// The user drags a rectangle (Space switches to picking a window).
    Area,
    /// The user picks a window.
    Window,
    /// The whole display under the mouse.
    Screen,
    /// A whole display: 1 is the main display, 2 the next, and so on.
    Display(u32),
    /// A fixed rectangle in Accessibility coordinates (points, top-left origin).
    Rect(Rect),
}

impl Target {
    /// The user picks what to capture; the run can end without a file.
    pub fn interactive(self) -> bool {
        matches!(self, Self::Area | Self::Window)
    }
}

/// One screenshot to take.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 5 (flick-e24c)"))]
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub target: Target,
    /// Where the PNG goes. Must not exist yet.
    pub path: PathBuf,
    /// Include the mouse pointer (non-interactive targets only; screencapture refuses `-C`
    /// with `-i`).
    pub cursor: bool,
    /// Keep the window shadow in window captures.
    pub shadow: bool,
    /// Play the shutter sound.
    pub sound: bool,
}

/// A screenshot on disk, with its size in pixels.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 5 (flick-e24c)"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// Why `run` returned no shot.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 5 (flick-e24c)"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The user pressed Esc in the selection UI.
    Cancelled,
    /// screencapture failed or wrote something that is not a PNG.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("capture cancelled"),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

/// The screencapture arguments for `req`, path last. `Target::Screen` maps to the main display
/// here; `run` first resolves it to the display under the mouse.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 5 (flick-e24c)"))]
pub fn args(req: &Request) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match req.target {
        Target::Area => out.push("-i".into()),
        Target::Window => out.extend(["-i".into(), "-w".into()]),
        Target::Screen => out.push("-m".into()),
        Target::Display(n) => out.extend(["-D".into(), n.to_string()]),
        Target::Rect(r) => {
            out.extend(["-R".into(), format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.w, r.h)]);
        }
    }
    if req.cursor && !req.target.interactive() {
        out.push("-C".into());
    }
    if !req.shadow {
        out.push("-o".into());
    }
    if !req.sound {
        out.push("-x".into());
    }
    out.extend(["-t".into(), "png".into(), req.path.display().to_string()]);
    out
}

/// Take the screenshot and wait for it. Blocking: interactive targets wait for the user.
/// Refuses an existing `req.path`, so a cancelled run is never mistaken for an old file.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 5 (flick-e24c)"))]
pub fn run(req: &Request) -> Result<Shot, Error> {
    if req.path.exists() {
        return Err(Error::Failed(format!("{} already exists", req.path.display())));
    }
    let mut req = req.clone();
    if req.target == Target::Screen {
        req.target = Target::Display(display_under_mouse());
    }
    let out = Command::new(SCREENCAPTURE)
        .args(args(&req))
        .output()
        .map_err(|e| Error::Failed(format!("{SCREENCAPTURE}: {e}")))?;
    if !out.status.success() {
        return Err(Error::Failed(format!(
            "{SCREENCAPTURE}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    finish(&req, &req.path)
}

/// Turn the file screencapture left at `path` (or its absence) into the result of `req`.
fn finish(req: &Request, path: &Path) -> Result<Shot, Error> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && req.target.interactive() => {
            return Err(Error::Cancelled);
        }
        Err(e) => return Err(Error::Failed(format!("{}: {e}", path.display()))),
    };
    let (width, height) = png_size(&bytes)
        .ok_or_else(|| Error::Failed(format!("{} is not a PNG", path.display())))?;
    Ok(Shot { path: path.to_path_buf(), width, height })
}

/// Width and height in pixels from a PNG's IHDR chunk, which the format requires first.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in flick-abc0 step 5 (flick-e24c)"))]
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    let rest = bytes.strip_prefix(SIGNATURE)?;
    let ihdr = rest.get(4..16)?.strip_prefix(b"IHDR")?;
    let be = |b: &[u8]| -> Option<u32> { Some(u32::from_be_bytes(b.try_into().ok()?)) };
    Some((be(ihdr.get(..4)?)?, be(ihdr.get(4..8)?)?))
}

/// The screencapture display number (1-based, main first) whose frame holds `(x, y)`; 1 when
/// none does. `frames` and the point share one coordinate space.
fn display_at(x: f64, y: f64, frames: &[Rect]) -> u32 {
    let hit = frames.iter().position(|f| x >= f.x && x <= f.x + f.w && y >= f.y && y <= f.y + f.h);
    hit.and_then(|i| u32::try_from(i + 1).ok()).unwrap_or(1)
}

/// The display the mouse is on. `NSScreen::screens` lists the main display first, matching
/// screencapture's `-D` numbering.
fn display_under_mouse() -> u32 {
    let p = NSEvent::mouseLocation();
    let frames: Vec<Rect> = NSScreen::screens(super::mtm())
        .iter()
        .map(|s| {
            let f = s.frame();
            Rect { x: f.origin.x, y: f.origin.y, w: f.size.width, h: f.size.height }
        })
        .collect();
    display_at(p.x, p.y, &frames)
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Flick has the Screen Recording permission. Never prompts.
#[expect(dead_code, reason = "wired in flick-abc0 step 5 (flick-e24c)")]
pub fn permitted() -> bool {
    // SAFETY: plain C call without arguments; it only reads the TCC state.
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// Ask for the Screen Recording permission: the first call shows the system prompt, later
/// calls only answer. A grant takes effect after Flick restarts.
#[expect(dead_code, reason = "wired in flick-abc0 step 5 (flick-e24c)")]
pub fn request_permission() -> bool {
    // SAFETY: plain C call without arguments; macOS shows its own prompt.
    unsafe { CGRequestScreenCaptureAccess() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(target: Target) -> Request {
        Request { target, path: "/tmp/s.png".into(), cursor: false, shadow: true, sound: true }
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 6, 0, 0, 0]);
        b
    }

    #[test]
    fn args_for_every_target() {
        let rect = Rect { x: 10.0, y: 20.4, w: 400.0, h: 300.6 };
        let cases: [(Target, &[&str]); 5] = [
            (Target::Area, &["-i"]),
            (Target::Window, &["-i", "-w"]),
            (Target::Screen, &["-m"]),
            (Target::Display(2), &["-D", "2"]),
            (Target::Rect(rect), &["-R", "10,20,400,301"]),
        ];
        for (target, head) in cases {
            let mut want: Vec<&str> = head.to_vec();
            want.extend(["-t", "png", "/tmp/s.png"]);
            assert_eq!(args(&req(target)), want, "{target:?}");
        }
    }

    #[test]
    fn args_for_every_flag() {
        let all_off = Request { cursor: true, shadow: false, sound: false, ..req(Target::Screen) };
        assert_eq!(args(&all_off), ["-m", "-C", "-o", "-x", "-t", "png", "/tmp/s.png"]);
        // screencapture rejects -C in interactive mode.
        let area = Request { cursor: true, ..req(Target::Area) };
        assert_eq!(args(&area), ["-i", "-t", "png", "/tmp/s.png"]);
        let window = Request { shadow: false, ..req(Target::Window) };
        assert_eq!(args(&window), ["-i", "-w", "-o", "-t", "png", "/tmp/s.png"]);
    }

    #[test]
    fn png_size_reads_ihdr() {
        assert_eq!(png_size(&png(2880, 1800)), Some((2880, 1800)));
        assert_eq!(png_size(&png(1, 1)[..24]), Some((1, 1)));
    }

    #[test]
    fn png_size_rejects_truncated_and_foreign_bytes() {
        let full = png(640, 480);
        for len in [0, 7, 8, 15, 16, 23] {
            assert_eq!(png_size(&full[..len]), None, "len {len}");
        }
        assert_eq!(png_size(b"GIF89a\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"), None);
        let mut not_ihdr = full.clone();
        not_ihdr[12..16].copy_from_slice(b"IDAT");
        assert_eq!(png_size(&not_ihdr), None);
    }

    #[test]
    fn display_at_picks_the_screen_under_the_point() {
        let frames = [
            Rect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 },
            Rect { x: 1440.0, y: -200.0, w: 2560.0, h: 1440.0 },
        ];
        assert_eq!(display_at(100.0, 100.0, &frames), 1);
        assert_eq!(display_at(2000.0, 1000.0, &frames), 2);
        assert_eq!(display_at(-50.0, 0.0, &frames), 1);
        assert_eq!(display_at(0.0, 0.0, &[]), 1);
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flick-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn finish_maps_files_to_shots_and_errors() {
        let good = temp("good.png");
        std::fs::write(&good, png(800, 600)).unwrap();
        let shot = finish(&req(Target::Area), &good).unwrap();
        assert_eq!(shot, Shot { path: good.clone(), width: 800, height: 600 });

        let bad = temp("bad.png");
        std::fs::write(&bad, b"nope").unwrap();
        assert!(matches!(finish(&req(Target::Screen), &bad), Err(Error::Failed(_))));

        let missing = temp("missing.png");
        assert_eq!(finish(&req(Target::Area), &missing), Err(Error::Cancelled));
        assert_eq!(finish(&req(Target::Window), &missing), Err(Error::Cancelled));
        let err = finish(&req(Target::Display(1)), &missing).unwrap_err();
        assert!(matches!(&err, Error::Failed(why) if why.contains("missing.png")), "{err}");
        assert_eq!(Error::Cancelled.to_string(), "capture cancelled");

        std::fs::remove_file(good).unwrap();
        std::fs::remove_file(bad).unwrap();
    }

    #[test]
    fn run_refuses_an_existing_path() {
        let path = temp("exists.png");
        std::fs::write(&path, png(1, 1)).unwrap();
        let err = run(&Request { path: path.clone(), ..req(Target::Area) }).unwrap_err();
        assert!(err.to_string().ends_with("already exists"), "{err}");
        std::fs::remove_file(path).unwrap();
    }

    /// Manual smoke: needs the Screen Recording permission for the test binary's host
    /// (Terminal). `cargo test -- --ignored run_captures_the_main_display`.
    #[test]
    #[ignore = "takes a real screenshot; needs Screen Recording"]
    fn run_captures_the_main_display() {
        let path = temp("display1.png");
        let shot = run(&Request { path: path.clone(), sound: false, ..req(Target::Display(1)) });
        let shot = shot.unwrap();
        assert!(shot.width > 0 && shot.height > 0);
        std::fs::remove_file(path).unwrap();
    }
}
