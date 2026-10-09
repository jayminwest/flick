//! The feedback file: JSON Lines, one `Entry` or `Resolution` object per line, append only.
//! Each save is one `write` of one line to a file opened with `O_APPEND`, so a crash or a
//! second writer never damages earlier lines; `read` skips a line it cannot parse. Resolving
//! appends a `Resolution` naming the entry's `ts`; builds without it skip that line (it has
//! no `text`). Pure but for the file I/O.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const DAY: i64 = 86_400;

/// One line of the file. Flat on purpose: `jq -r .text feedback.jsonl` reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Local time with its UTC offset (RFC 3339): `2026-10-09T11:31:01-07:00`.
    pub ts: String,
    pub text: String,
    /// The commit this Flick was built from: a full sha, `-dirty` for a build with
    /// uncommitted changes, or `dev` for a plain `cargo build`.
    pub build: String,
    /// The app in front when the launcher opened; absent from the CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// That app's bundle identifier, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    /// The root search text when "Add Feedback…" ran, when not empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Set by `read` from a later `Resolution` line; never written with the entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
}

/// How an entry was resolved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolved {
    /// When, like `Entry::ts`.
    pub ts: String,
    /// E.g. the issue that tracks it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A line marking every entry whose `ts` is `resolves` as resolved.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub resolves: String,
    #[serde(flatten)]
    pub resolved: Resolved,
}

/// One parsed line.
#[derive(Deserialize)]
#[serde(untagged)]
enum Line {
    Resolution(Resolution),
    Entry(Entry),
}

/// Append `entry` (an `Entry` or a `Resolution`) as one line, creating the file and its
/// folder first.
pub fn append(path: &Path, entry: &impl Serialize) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("Can't write {}: {e}", path.display());
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path).map_err(fail)?;
    file.write_all(line.as_bytes()).map_err(fail)
}

/// The last `limit` entries, newest first, with `resolved` set; resolved ones only when
/// `all`. A missing file has none; bad lines are skipped.
pub fn read(path: &Path, limit: usize, all: bool) -> Result<Vec<Entry>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("Can't read {}: {e}", path.display())),
    };
    let (mut entries, mut resolved) = (vec![], HashMap::new());
    for line in text.lines().filter_map(|l| serde_json::from_str::<Line>(l).ok()) {
        match line {
            Line::Entry(e) => entries.push(e),
            Line::Resolution(r) => {
                resolved.insert(r.resolves, r.resolved);
            }
        }
    }
    let entries = entries.into_iter().rev().map(|e| Entry { resolved: resolved.get(&e.ts).cloned(), ..e });
    Ok(entries.filter(|e| all || e.resolved.is_none()).take(limit).collect())
}

/// Unix time `ts` as RFC 3339 local time, `utc_offset_secs` east of UTC.
pub fn rfc3339(ts: i64, utc_offset_secs: i32) -> String {
    let local = ts + i64::from(utc_offset_secs);
    let (y, m, d) = civil_from_days(local.div_euclid(DAY));
    let secs = local.rem_euclid(DAY);
    let (sign, off) = if utc_offset_secs < 0 { ('-', -utc_offset_secs) } else { ('+', utc_offset_secs) };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
        off / 3600,
        off % 3600 / 60
    )
}

/// The build label for `sha` (`FLICK_BUILD_SHA`) and `dirty` (`FLICK_BUILD_DIRTY`).
pub fn build(sha: Option<&str>, dirty: Option<&str>) -> String {
    let sha = sha.map(str::trim).filter(|s| (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()));
    match sha {
        Some(sha) if dirty.is_some_and(|d| d.trim() == "1") => format!("{}-dirty", sha.to_lowercase()),
        Some(sha) => sha.to_lowercase(),
        None => "dev".into(),
    }
}

/// The file: `file` when set (`~` expanded), else `feedback.jsonl` in checkout `source`.
/// The checkout must exist: a prebuilt app knows the path of a machine it never ran on.
pub fn resolve(file: &str, source: Option<&Path>, home: Option<&Path>) -> Result<PathBuf, String> {
    let file = file.trim();
    if !file.is_empty() {
        return Ok(expand(file, home));
    }
    match source {
        Some(dir) if dir.is_dir() => Ok(dir.join("feedback.jsonl")),
        _ => Err("No feedback file: this Flick does not know its checkout. Set [feedback] file".into()),
    }
}

/// `~` and `~/...` under `home`.
fn expand(path: &str, home: Option<&Path>) -> PathBuf {
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

    fn entry(text: &str) -> Entry {
        Entry {
            ts: "t".into(),
            text: text.into(),
            build: "dev".into(),
            app: None,
            bundle_id: None,
            query: None,
            resolved: None,
        }
    }

    #[test]
    fn times_are_local_rfc3339_with_the_offset() {
        assert_eq!(rfc3339(TS, 0), "2026-10-09T18:31:01+00:00");
        assert_eq!(rfc3339(TS, -25_200), "2026-10-09T11:31:01-07:00");
        assert_eq!(rfc3339(TS, 19_800), "2026-10-10T00:01:01+05:30");
        assert_eq!(rfc3339(0, -3600), "1969-12-31T23:00:00-01:00");
    }

    #[test]
    fn build_is_the_sha_or_dev() {
        let sha = "0123456789ABCDEF0123456789abcdef01234567";
        assert_eq!(build(Some(sha), Some("0")), sha.to_lowercase());
        assert_eq!(build(Some(sha), Some("1")), format!("{}-dirty", sha.to_lowercase()));
        assert_eq!(build(Some("unknown"), Some("1")), "dev");
        assert_eq!(build(None, None), "dev");
    }

    #[test]
    fn the_file_is_the_setting_or_the_checkouts() {
        let home = Path::new("/Users/me");
        assert_eq!(resolve(" ~/fb.jsonl ", None, Some(home)).unwrap(), home.join("fb.jsonl"));
        assert_eq!(resolve("~", None, Some(home)).unwrap(), home);
        assert_eq!(resolve("/x/f.jsonl", None, None).unwrap(), PathBuf::from("/x/f.jsonl"));
        assert_eq!(resolve("~other/f", None, Some(home)).unwrap(), PathBuf::from("~other/f"));
        let dir = std::env::temp_dir();
        assert_eq!(resolve("", Some(&dir), None).unwrap(), dir.join("feedback.jsonl"));
        let gone = dir.join("flick-feedback-no-such-checkout");
        assert!(resolve("", Some(&gone), None).unwrap_err().contains("[feedback] file"));
        assert!(resolve("", None, None).is_err());
    }

    #[test]
    fn entries_append_as_lines_and_read_back_newest_first() {
        let dir = std::env::temp_dir().join(format!("flick-feedback-entry-{}", std::process::id()));
        let path = dir.join("sub/feedback.jsonl");
        assert_eq!(read(&path, 5, false).unwrap(), []);
        let mut first = entry("one\nline two");
        first.app = Some("Safari".into());
        append(&path, &first).unwrap();
        std::fs::write(&path, std::fs::read_to_string(&path).unwrap() + "not json\n").unwrap();
        append(&path, &entry("two")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert!(text.starts_with(r#"{"ts":"t","text":"one\nline two","build":"dev","app":"Safari"}"#), "{text}");
        assert_eq!(read(&path, 5, false).unwrap(), [entry("two"), first]);
        assert_eq!(read(&path, 1, false).unwrap(), [entry("two")]);
        assert!(read(&dir, 1, false).unwrap_err().starts_with("Can't read"));
        assert!(append(&dir, &entry("x")).unwrap_err().starts_with("Can't write"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolutions_mark_entries_by_ts_and_hide_them_unless_all() {
        let dir = std::env::temp_dir().join(format!("flick-feedback-resolve-{}", std::process::id()));
        let path = dir.join("feedback.jsonl");
        let (a, b) = (Entry { ts: "a".into(), ..entry("one") }, Entry { ts: "b".into(), ..entry("two") });
        append(&path, &a).unwrap();
        append(&path, &b).unwrap();
        let done = Resolved { ts: "c".into(), note: Some("flick-1234".into()) };
        append(&path, &Resolution { resolves: "a".into(), resolved: done.clone() }).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("{\"resolves\":\"a\",\"ts\":\"c\",\"note\":\"flick-1234\"}\n"), "{text}");
        assert_eq!(read(&path, 5, false).unwrap(), std::slice::from_ref(&b));
        let resolved_a = Entry { resolved: Some(done), ..a };
        assert_eq!(read(&path, 5, true).unwrap(), [b, resolved_a]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
