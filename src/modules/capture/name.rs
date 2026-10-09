//! Screenshot file names: a template with `{date}`, `{time}` and `{kind}`, local time, and a
//! ` (2)` suffix when the name is taken. Pure: the caller passes the clock, the UTC offset and
//! an existence check.

use std::path::{Path, PathBuf};

/// The default `[capture] name`, like macOS's own "Screenshot <date> at <time>.png".
pub const DEFAULT: &str = "Flick {date} at {time}.png";

const DAY: i64 = 86_400;

/// Check a `name` template: no path separators, and nothing left once filled in.
pub fn validate(template: &str) -> Result<(), String> {
    if template.contains('/') {
        return Err(format!("[capture]: name {template:?} must not contain '/'"));
    }
    if template.trim().is_empty() {
        return Err("[capture]: name must not be empty".into());
    }
    Ok(())
}

/// `template` filled in for a `kind` capture taken at Unix time `ts`, ending in `.png`.
/// `{date}` is `YYYY-MM-DD`, `{time}` `HH.MM.SS` (Finder shows `:` as `/`).
pub fn fill(template: &str, kind: &str, ts: i64, utc_offset_secs: i32) -> String {
    let local = ts + i64::from(utc_offset_secs);
    let (y, m, d) = civil_from_days(local.div_euclid(DAY));
    let secs = local.rem_euclid(DAY);
    let date = format!("{y:04}-{m:02}-{d:02}");
    let time = format!("{:02}.{:02}.{:02}", secs / 3600, secs % 3600 / 60, secs % 60);
    let name = template.replace("{date}", &date).replace("{time}", &time).replace("{kind}", kind);
    if name.to_ascii_lowercase().ends_with(".png") { name } else { format!("{name}.png") }
}

/// `dir/name`, or `dir/<stem> (n).png` with the smallest `n >= 2` for which `exists` is false.
pub fn unique(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let stem = &name[..name.len() - ".png".len()];
    (2..u32::MAX)
        .map(|n| dir.join(format!("{stem} ({n}).png")))
        .find(|p| !exists(p))
        .unwrap_or(first)
}

/// `~` and `~/...` under `home`.
pub fn expand(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_path_buf(),
        (Some(rest), Some(home)) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

/// The civil date (y, m, d) of day number `z` since 1970-01-01 (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-09 18:31:01 UTC.
    const TS: i64 = 1_791_570_661;

    #[test]
    fn fill_uses_local_date_and_time() {
        assert_eq!(fill(DEFAULT, "area", TS, 0), "Flick 2026-10-09 at 18.31.01.png");
        // PDT: still the 9th, seven hours earlier.
        assert_eq!(fill(DEFAULT, "area", TS, -25_200), "Flick 2026-10-09 at 11.31.01.png");
        // Ten hours east crosses midnight.
        assert_eq!(fill(DEFAULT, "area", TS, 36_000), "Flick 2026-10-10 at 04.31.01.png");
        assert_eq!(fill("{kind}-{date}", "window", 0, 0), "window-1970-01-01.png");
        assert_eq!(fill("Shot.PNG", "screen", 0, 0), "Shot.PNG");
        // Leap day and a date before 1970.
        assert_eq!(fill("{date}", "a", 951_782_400, 0), "2000-02-29.png");
        assert_eq!(fill("{date} {time}", "a", -1, 0), "1969-12-31 23.59.59.png");
    }

    #[test]
    fn unique_appends_the_first_free_number() {
        let dir = Path::new("/shots");
        let taken = ["/shots/a.png", "/shots/a (2).png"];
        let exists = |p: &Path| taken.iter().any(|t| Path::new(t) == p);
        assert_eq!(unique(dir, "b.png", exists), PathBuf::from("/shots/b.png"));
        assert_eq!(unique(dir, "a.png", exists), PathBuf::from("/shots/a (3).png"));
    }

    #[test]
    fn validate_rejects_separators_and_blanks() {
        assert!(validate(DEFAULT).is_ok());
        assert_eq!(validate("a/b").unwrap_err(), "[capture]: name \"a/b\" must not contain '/'");
        assert_eq!(validate("  ").unwrap_err(), "[capture]: name must not be empty");
    }

    #[test]
    fn expand_resolves_the_home_directory() {
        let home = Path::new("/Users/me");
        assert_eq!(expand("~", Some(home)), PathBuf::from("/Users/me"));
        assert_eq!(expand("~/Pictures/Flick", Some(home)), PathBuf::from("/Users/me/Pictures/Flick"));
        assert_eq!(expand("~other/x", Some(home)), PathBuf::from("~other/x"));
        assert_eq!(expand("/tmp/x", Some(home)), PathBuf::from("/tmp/x"));
        assert_eq!(expand("~/x", None), PathBuf::from("~/x"));
    }
}
