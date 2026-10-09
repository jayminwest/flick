//! Local time: the UTC offset of the system time zone, so core and modules can split days
//! without FFI or a time zone crate.
#![cfg_attr(not(test), expect(dead_code, reason = "wired in flick-a513 step 6 (flick-a30b)"))]

use std::ffi::{c_char, c_int, c_long};

/// `struct tm` from macOS `<time.h>`.
#[repr(C)]
struct Tm {
    sec: c_int,
    min: c_int,
    hour: c_int,
    mday: c_int,
    mon: c_int,
    year: c_int,
    wday: c_int,
    yday: c_int,
    isdst: c_int,
    gmtoff: c_long,
    zone: *mut c_char,
}

unsafe extern "C" {
    // `time_t` is a 64-bit `long` on macOS.
    fn localtime_r(clock: *const i64, result: *mut Tm) -> *mut Tm;
}

/// Seconds east of UTC in the local time zone at Unix time `ts` (DST included): -25200 for
/// PDT, 3600 for CET. 0 when the time cannot be converted.
pub fn utc_offset(ts: i64) -> i32 {
    let mut tm = Tm {
        sec: 0,
        min: 0,
        hour: 0,
        mday: 0,
        mon: 0,
        year: 0,
        wday: 0,
        yday: 0,
        isdst: 0,
        gmtoff: 0,
        zone: std::ptr::null_mut(),
    };
    // SAFETY: both pointers are valid for the call; `localtime_r` is the reentrant variant
    // and writes only to `tm`, returning null on failure.
    let out = unsafe { localtime_r(&ts, &mut tm) };
    if out.is_null() { 0 } else { i32::try_from(tm.gmtoff).unwrap_or(0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offset `date` prints for `ts` (`+0200`), in seconds.
    fn date_offset(ts: i64) -> i32 {
        let out = std::process::Command::new("/bin/date")
            .args(["-r", &ts.to_string(), "+%z"])
            .output()
            .unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let text = text.trim();
        let sign = if text.starts_with('-') { -1 } else { 1 };
        let hours: i32 = text[1..3].parse().unwrap();
        let minutes: i32 = text[3..5].parse().unwrap();
        sign * (hours * 3600 + minutes * 60)
    }

    #[test]
    fn matches_the_system_time_zone_in_winter_and_summer() {
        // 2026-01-15 and 2026-07-15, 12:00 UTC: one of them is in DST where DST applies.
        for ts in [1_768_478_400, 1_784_116_800, 0] {
            assert_eq!(utc_offset(ts), date_offset(ts), "at {ts}");
        }
    }

    #[test]
    fn offsets_are_whole_quarter_hours_within_a_day() {
        let offset = utc_offset(1_768_478_400);
        assert_eq!(offset % 900, 0);
        assert!(offset.abs() <= 14 * 3600);
    }
}
