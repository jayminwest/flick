//! Screenshots for KOTA (flick-2943): where one goes on mbp-server and the one ssh call that
//! puts it there.
//!
//! `[message] attach_dir` (default `DEFAULT_DIR`) is a path relative to the remote home,
//! of `A-Z a-z 0-9 . _ / -` only, no `.`/`..` component and none starting with `-`, so it
//! is safe inside the remote shell command unquoted. Screenshot `n` (from 1) of request
//! `req` is `<dir>/<req>-<n>.png`. The upload, with the PNG on stdin:
//!
//! ```text
//! /usr/bin/ssh -o BatchMode=yes -o ConnectTimeout=8 <host> <one argv word:>
//!   mkdir -p D && cat > D/<req>-<n>.png && find D -type f -name '*.png' -mtime +7 -delete
//! ```
//!
//! creates the dir, writes the file, and prunes screenshots older than `KEEP_DAYS` in the
//! same connection. It runs before the ask in the same worker job; the ask's `[context]`
//! block names the path (`ask::Item::Screenshot`).

use std::time::Duration;

use super::ask;
use crate::core::card::valid_id;

/// Where screenshots go, relative to the remote home.
pub const DEFAULT_DIR: &str = ".cache/flick/attach";
/// Budget of the upload's ssh child.
pub const BUDGET: Duration = Duration::from_secs(30);
/// The largest PNG sent, in bytes.
pub const PNG_MAX: u64 = 32 * 1024 * 1024;
/// Screenshots older than this many days are deleted on each upload.
pub const KEEP_DAYS: u32 = 7;
/// The longest remote path accepted (`attach_dir`, `kota_ask`).
const PATH_MAX: usize = 200;

/// Whether `s` can go into a remote shell command unquoted as a path: 1 to `PATH_MAX` of
/// `A-Z a-z 0-9 . _ / ~ -`, not starting with `-`.
pub fn remote_word(s: &str) -> bool {
    (1..=PATH_MAX).contains(&s.len())
        && !s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '~' | '-'))
}

/// `attach_dir` checked and without trailing slashes.
pub fn dir(d: &str) -> Result<String, String> {
    let bad = || format!("[message] attach_dir {d:?}: a path under the remote home of A-Z a-z 0-9 . _ / -");
    let d2 = d.trim_end_matches('/');
    let parts_ok = d2.split('/').all(|p| !p.is_empty() && p != "." && p != ".." && !p.starts_with('-'));
    if !remote_word(d2) || d2.contains('~') || !parts_ok {
        return Err(bad());
    }
    Ok(d2.to_string())
}

/// The remote path of screenshot `n` of request `req` under `dir`.
pub fn path(dir: &str, req: &str, n: u32) -> Result<String, String> {
    let dir = self::dir(dir)?;
    if !valid_id(req) {
        return Err(format!("{req:?}: a request id is 1-64 of A-Z a-z 0-9 . _ -"));
    }
    Ok(format!("{dir}/{req}-{n}.png"))
}

/// The upload's argv; the PNG goes on stdin.
pub fn upload_argv(host: &str, dir: &str, req: &str, n: u32) -> Result<Vec<String>, String> {
    let file = path(dir, req, n)?;
    let dir = self::dir(dir)?;
    let mut argv = ask::ssh(host)?;
    argv.push(format!(
        "mkdir -p {dir} && cat > {file} && find {dir} -type f -name '*.png' -mtime +{KEEP_DAYS} -delete"
    ));
    Ok(argv)
}

/// A PNG of `bytes` bytes may be sent: not empty, at most `PNG_MAX`.
pub fn check_size(bytes: u64) -> Result<(), String> {
    match bytes {
        0 => Err("The screenshot is empty".into()),
        n if n > PNG_MAX => Err(format!("The screenshot is over {} MB", PNG_MAX / (1024 * 1024))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
