//! Parsing and answers for `flick capture` verbs that need no module state.

use std::path::PathBuf;

use crate::core::Cx;
use crate::platform::Rect;
use crate::platform::capture::Target;

use super::store::Row;
use super::{name, view};

/// `--out <absolute path>`, `--no-copy` and `--annotate`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Opts {
    pub out: Option<PathBuf>,
    pub no_copy: bool,
    /// Open the editor on the shot (area and window only).
    pub annotate: bool,
}

/// `ls [--limit n]` and `last`.
pub fn list(limit: u32, cx: &Cx) -> Result<String, String> {
    let rows = view::live_shots(cx.store, limit);
    if cx.json {
        return serde_json::to_string(&rows).map_err(|e| format!("capture: {e}"));
    }
    let line = |r: &Row| format!("{}\t{}\t{}x{}", r.id, r.path, r.width, r.height);
    Ok(rows.iter().map(line).collect::<Vec<_>>().join("\n"))
}

/// `last`: the newest capture.
pub fn last(cx: &Cx) -> Result<String, String> {
    let row = view::live_shots(cx.store, 1).into_iter().next().ok_or("capture: no captures yet")?;
    if cx.json {
        return serde_json::to_string(&row).map_err(|e| format!("capture: {e}"));
    }
    Ok(row.path)
}

/// The store's name for what `target` captures.
pub fn kind(target: Target) -> &'static str {
    match target {
        Target::Area => "area",
        Target::Window => "window",
        Target::Screen => "screen",
        Target::Display(_) => "display",
        Target::Rect(_) => "rect",
    }
}

/// `--out <absolute path>`, `--no-copy` and `--annotate`.
pub fn options(args: &[String]) -> Result<Opts, String> {
    let mut opts = Opts::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-copy" => opts.no_copy = true,
            "--annotate" => opts.annotate = true,
            "--out" => {
                let path = args.next().ok_or("capture: --out needs a path")?;
                let path = name::expand(path, dirs::home_dir().as_deref());
                if !path.is_absolute() {
                    return Err("capture: --out needs an absolute path".into());
                }
                opts.out = Some(path);
            }
            other => return Err(format!("capture: unknown option {other:?}")),
        }
    }
    Ok(opts)
}

/// `x,y,w,h` in points, top-left origin; width and height above zero.
pub fn parse_rect(text: &str) -> Result<Rect, String> {
    let bad = || format!("capture: bad rect {text:?} (want x,y,w,h)");
    let nums: Vec<f64> = text.split(',').map(|n| n.trim().parse().map_err(|_| bad())).collect::<Result<_, _>>()?;
    match nums[..] {
        [x, y, w, h] if w > 0.0 && h > 0.0 => Ok(Rect { x, y, w, h }),
        _ => Err(bad()),
    }
}

/// `--limit n` (default 20).
pub fn parse_limit(args: &[String]) -> Result<u32, String> {
    match args {
        [] => Ok(20),
        [flag, n] if flag == "--limit" => n.parse().map_err(|_| format!("capture: bad limit {n:?}")),
        _ => Err("capture: usage: capture ls [--limit n]".into()),
    }
}
