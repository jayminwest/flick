//! Caps Lock as F18 at the HID layer (`hidutil property UserKeyMapping`).
//!
//! An event tap cannot stop the Caps Lock toggle: the lock state and LED change below it. So,
//! like Hyperkey and Karabiner, Flick remaps the key before macOS sees it, and the key engine
//! treats F18 (keycode 79) as the hyper source. `UserKeyMapping` is one global list shared with
//! every other tool, so both directions read, merge and write: other mappings survive, and
//! `clear` removes only Flick's entry. The mapping outlives Flick; quit clears it
//! (`app::on_terminate`), and `hidutil property --set '{"UserKeyMapping":[]}'` resets it by hand.

use std::process::Command;

/// HID usage page 7 (keyboard), usage 0x39: Caps Lock.
pub const CAPS_LOCK: u64 = 0x7_0000_0039;
/// HID usage page 7 (keyboard), usage 0x6D: F18 (macOS virtual keycode 79).
pub const F18: u64 = 0x7_0000_006D;

const HIDUTIL: &str = "/usr/bin/hidutil";
const SRC: &str = "HIDKeyboardModifierMappingSrc";
const DST: &str = "HIDKeyboardModifierMappingDst";

/// One `UserKeyMapping` entry: HID usage `src` sends `dst`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub src: u64,
    pub dst: u64,
}

/// Flick's entry.
const OURS: Mapping = Mapping { src: CAPS_LOCK, dst: F18 };

/// Runs `hidutil` with these arguments; `Ok` holds stdout. Tests pass a stub.
type Run<'a> = &'a mut dyn FnMut(&[&str]) -> Result<String, String>;

/// Map Caps Lock to F18, keeping every other mapping. Replaces another tool's Caps Lock entry
/// (two entries for one key are ambiguous). Writes nothing when the mapping is already set.
pub fn set_caps_to_f18() -> Result<(), String> {
    set_with(&mut hidutil)
}

/// Remove Flick's Caps Lock to F18 entry, keeping every other mapping. Writes nothing when it
/// is not set.
pub fn clear_caps_to_f18() -> Result<(), String> {
    clear_with(&mut hidutil)
}

/// Caps Lock currently sends F18 (for `flick keys status`).
pub fn caps_to_f18_is_set() -> Result<bool, String> {
    is_set_with(&mut hidutil)
}

fn set_with(run: Run) -> Result<(), String> {
    let current = get(run)?;
    if current.contains(&OURS) {
        return Ok(());
    }
    put(run, &with_ours(&current))
}

fn clear_with(run: Run) -> Result<(), String> {
    let current = get(run)?;
    if !current.contains(&OURS) {
        return Ok(());
    }
    put(run, &without_ours(&current))
}

fn is_set_with(run: Run) -> Result<bool, String> {
    Ok(get(run)?.contains(&OURS))
}

fn get(run: Run) -> Result<Vec<Mapping>, String> {
    let out = run(&["property", "--get", "UserKeyMapping"])?;
    parse(&out).ok_or_else(|| format!("unrecognized hidutil UserKeyMapping output: {}", out.trim()))
}

fn put(run: Run, mappings: &[Mapping]) -> Result<(), String> {
    run(&["property", "--set", &to_json(mappings)]).map(drop)
}

/// The real runner: `/usr/bin/hidutil <args>`.
fn hidutil(args: &[&str]) -> Result<String, String> {
    let out = Command::new(HIDUTIL).args(args).output().map_err(|e| format!("{HIDUTIL}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{HIDUTIL} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// `current` without any Caps Lock entry, plus Flick's.
fn with_ours(current: &[Mapping]) -> Vec<Mapping> {
    let mut out: Vec<Mapping> = current.iter().copied().filter(|m| m.src != CAPS_LOCK).collect();
    out.push(OURS);
    out
}

/// `current` without Flick's entry (another tool's Caps Lock mapping stays).
fn without_ours(current: &[Mapping]) -> Vec<Mapping> {
    current.iter().copied().filter(|m| *m != OURS).collect()
}

/// The `--set` argument. Values are decimal: plain JSON, no hex extension needed.
fn to_json(mappings: &[Mapping]) -> String {
    let entries: Vec<String> =
        mappings.iter().map(|m| format!(r#"{{"{SRC}":{},"{DST}":{}}}"#, m.src, m.dst)).collect();
    format!(r#"{{"UserKeyMapping":[{}]}}"#, entries.join(","))
}

/// Parse `hidutil property --get UserKeyMapping`: `(null)` when never set, else an
/// old-style plist array of dictionaries with decimal numbers:
///
/// ```text
/// (
///         {
///         HIDKeyboardModifierMappingDst = 30064771181;
///         HIDKeyboardModifierMappingSrc = 30064771129;
///     }
/// )
/// ```
///
/// On a Mac with several HID services and no global mapping, `--get` prints a per-service
/// table (`RegistryID  Key  Value` header, one row per service); all `(null)` means unset.
///
/// `None` for anything else, so a format change never writes back a list that lost entries.
fn parse(out: &str) -> Option<Vec<Mapping>> {
    let out = out.trim();
    if out.is_empty() || out == "(null)" || all_null_table(out) {
        return Some(Vec::new());
    }
    let body = out.strip_prefix('(')?.strip_suffix(')')?.trim();
    let mut mappings = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let open = rest.strip_prefix('{')?;
        let close = open.find('}')?;
        mappings.push(parse_entry(&open[..close])?);
        rest = open[close + 1..].trim_start().trim_start_matches(',').trim_start();
    }
    Some(mappings)
}

/// The per-service table with every value `(null)`.
fn all_null_table(out: &str) -> bool {
    let mut lines = out.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split_whitespace().collect();
    header == ["RegistryID", "Key", "Value"]
        && lines.all(|l| {
            matches!(l.split_whitespace().collect::<Vec<_>>()[..], [_, "UserKeyMapping", "(null)"])
        })
}

/// `Key = value;` pairs; exactly Src and Dst.
fn parse_entry(entry: &str) -> Option<Mapping> {
    let (mut src, mut dst) = (None, None);
    for field in entry.split(';').map(str::trim).filter(|f| !f.is_empty()) {
        let (key, value) = field.split_once('=')?;
        let slot = match key.trim() {
            SRC => &mut src,
            DST => &mut dst,
            _ => return None,
        };
        *slot = Some(number(value.trim().trim_matches('"'))?);
    }
    Some(Mapping { src: src?, dst: dst? })
}

fn number(s: &str) -> Option<u64> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OTHER: Mapping = Mapping { src: 0x7_0000_0064, dst: 0x7_0000_0035 };

    /// `hidutil --get` text for these mappings, as macOS prints it (Dst before Src).
    fn listing(mappings: &[Mapping]) -> String {
        if mappings.is_empty() {
            return "(null)\n".into();
        }
        let entries: Vec<String> = mappings
            .iter()
            .map(|m| {
                format!(
                    "        {{\n        {DST} = {};\n        {SRC} = {};\n    }}",
                    m.dst, m.src
                )
            })
            .collect();
        format!("(\n{}\n)\n", entries.join(",\n"))
    }

    /// A stub `hidutil`: answers `--get` from `state`, applies `--set` to it, logs every call.
    struct Stub {
        state: String,
        calls: Vec<Vec<String>>,
    }

    impl Stub {
        fn new(mappings: &[Mapping]) -> Self {
            Self { state: listing(mappings), calls: Vec::new() }
        }

        fn run(&mut self, args: &[&str]) -> Result<String, String> {
            self.calls.push(args.iter().map(|a| (*a).to_owned()).collect());
            match args {
                ["property", "--get", "UserKeyMapping"] => Ok(self.state.clone()),
                ["property", "--set", json] => {
                    self.state = listing(&from_json(json));
                    Ok(String::new())
                }
                _ => Err(format!("unexpected args {args:?}")),
            }
        }

        fn sets(&self) -> usize {
            self.calls.iter().filter(|c| c[1] == "--set").count()
        }

        fn mappings(&self) -> Vec<Mapping> {
            parse(&self.state).unwrap()
        }
    }

    fn from_json(json: &str) -> Vec<Mapping> {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v["UserKeyMapping"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| Mapping { src: e[SRC].as_u64().unwrap(), dst: e[DST].as_u64().unwrap() })
            .collect()
    }

    #[test]
    fn usages_are_caps_lock_and_f18() {
        assert_eq!(CAPS_LOCK, 30_064_771_129);
        assert_eq!(F18, 30_064_771_181);
    }

    #[test]
    fn parses_null_empty_and_real_listings() {
        assert_eq!(parse("(null)\n"), Some(vec![]));
        assert_eq!(parse(""), Some(vec![]));
        assert_eq!(parse("(\n)\n"), Some(vec![]));
        assert_eq!(parse(&listing(&[OTHER, OURS])), Some(vec![OTHER, OURS]));
        let hex = "({HIDKeyboardModifierMappingSrc = 0x700000039; HIDKeyboardModifierMappingDst = 0X70000006D;})";
        assert_eq!(parse(hex), Some(vec![OURS]));
        let table = "RegistryID  Key                   Value\n10000075d   UserKeyMapping   (null)\n100000a4e   UserKeyMapping   (null)\n";
        assert_eq!(parse(table), Some(vec![]));
    }

    #[test]
    fn unrecognized_output_is_none() {
        for bad in [
            "garbage",
            "(",
            "( { HIDKeyboardModifierMappingSrc = 1; } )",
            "( { HIDKeyboardModifierMappingSrc = 1; HIDKeyboardModifierMappingDst = x; } )",
            "( { HIDKeyboardModifierMappingSrc = 1; HIDKeyboardModifierMappingDst = 2; Extra = 3; } )",
            "( { HIDKeyboardModifierMappingSrc 1; } )",
            "( { HIDKeyboardModifierMappingSrc = 1; HIDKeyboardModifierMappingDst = 2; ",
            "( x )",
            "RegistryID  Key  Value\n10000075d   UserKeyMapping   (\n",
            "RegistryID  Key\n10000075d   UserKeyMapping   (null)\n",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn json_round_trips_in_decimal() {
        let json = to_json(&[OTHER, OURS]);
        assert!(json.contains("30064771129") && !json.contains("0x"), "{json}");
        assert_eq!(from_json(&json), vec![OTHER, OURS]);
        assert_eq!(to_json(&[]), r#"{"UserKeyMapping":[]}"#);
    }

    #[test]
    fn merge_keeps_others_and_replaces_a_foreign_caps_lock_entry() {
        let foreign = Mapping { src: CAPS_LOCK, dst: 0x7_0000_0029 };
        assert_eq!(with_ours(&[]), vec![OURS]);
        assert_eq!(with_ours(&[OTHER, foreign]), vec![OTHER, OURS]);
        assert_eq!(without_ours(&[OTHER, OURS]), vec![OTHER]);
        assert_eq!(without_ours(&[foreign]), vec![foreign]);
    }

    #[test]
    fn set_merges_into_existing_mappings() {
        let mut stub = Stub::new(&[OTHER]);
        set_with(&mut |a| stub.run(a)).unwrap();
        assert_eq!(stub.mappings(), vec![OTHER, OURS]);
        assert_eq!(stub.calls[0], ["property", "--get", "UserKeyMapping"]);
        assert!(is_set_with(&mut |a| stub.run(a)).unwrap());
    }

    #[test]
    fn set_twice_writes_once() {
        let mut stub = Stub::new(&[]);
        set_with(&mut |a| stub.run(a)).unwrap();
        set_with(&mut |a| stub.run(a)).unwrap();
        assert_eq!(stub.sets(), 1);
        assert_eq!(stub.mappings(), vec![OURS]);
    }

    #[test]
    fn clear_removes_only_ours() {
        let mut stub = Stub::new(&[OTHER, OURS]);
        clear_with(&mut |a| stub.run(a)).unwrap();
        assert_eq!(stub.mappings(), vec![OTHER]);
        assert!(!is_set_with(&mut |a| stub.run(a)).unwrap());

        let mut empty = Stub::new(&[OURS]);
        clear_with(&mut |a| empty.run(a)).unwrap();
        assert_eq!(empty.calls[1], ["property", "--set", r#"{"UserKeyMapping":[]}"#]);
    }

    #[test]
    fn clear_without_ours_writes_nothing() {
        let mut stub = Stub::new(&[OTHER]);
        clear_with(&mut |a| stub.run(a)).unwrap();
        assert_eq!(stub.sets(), 0);
        assert_eq!(stub.mappings(), vec![OTHER]);
    }

    #[test]
    fn unparseable_or_failing_get_never_writes() {
        let mut stub = Stub::new(&[]);
        stub.state = "something new".into();
        assert!(set_with(&mut |a| stub.run(a)).unwrap_err().contains("something new"));
        assert!(clear_with(&mut |a| stub.run(a)).is_err());
        assert_eq!(stub.sets(), 0);

        let mut fail = |_: &[&str]| Err::<String, String>("no hidutil".into());
        assert_eq!(set_with(&mut fail), Err("no hidutil".into()));
        assert_eq!(is_set_with(&mut fail), Err("no hidutil".into()));
    }

    #[test]
    fn the_live_mapping_parses() {
        // Read-only: `--get` on this machine, whatever other tools have set.
        caps_to_f18_is_set().unwrap();
    }

    #[test]
    fn the_real_runner_reports_failures() {
        // Read-only and invalid: never touches the live mapping.
        let err = hidutil(&["property", "--no-such-flag"]).unwrap_err();
        assert!(err.starts_with(HIDUTIL), "{err}");
    }
}
