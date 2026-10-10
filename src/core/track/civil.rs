//! Civil dates and short durations for span reports: local midnights, `YYYY-MM-DD` dates
//! and times, "2h 05m", and spans cut to a range. Shared by `activity` and `task`, which may
//! not import each other (flick-dc95). Day numbers are days since 1970-01-01 (proleptic
//! Gregorian), local days as `local_day` gives them.

use super::{DAY, Span};

/// Unix time of local midnight starting local day `day`.
pub fn day_start(day: i64, utc_offset_secs: i32) -> i64 {
    day * DAY - i64::from(utc_offset_secs)
}

/// Days since 1970-01-01 of civil date `y-m-d` (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date (y, m, d) of day number `z`.
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

/// "YYYY-MM-DD" of day number `z`.
pub fn date(z: i64) -> String {
    let (y, m, d) = civil_from_days(z);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The day number of date "YYYY-MM-DD". `None` for anything else, or a month outside 1-12
/// or a day outside 1-31.
pub fn parse_date(s: &str) -> Option<i64> {
    let mut parts = s.splitn(3, '-').map(str::parse::<i64>);
    let (Some(Ok(y)), Some(Ok(m)), Some(Ok(d))) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

/// "YYYY-MM-DD HH:MM" in local time.
pub fn local_time(ts: i64, utc_offset_secs: i32) -> String {
    let local = ts + i64::from(utc_offset_secs);
    let secs = local.rem_euclid(DAY);
    format!("{} {:02}:{:02}", date(local.div_euclid(DAY)), secs / 3600, secs % 3600 / 60)
}

/// "2h 05m", "12m", "45s".
pub fn duration(secs: i64) -> String {
    match secs {
        ..60 => format!("{}s", secs.max(0)),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

/// `spans` cut to `from..to`, empty parts dropped.
pub fn clip<S>(spans: Vec<Span<S>>, from: i64, to: i64) -> Vec<Span<S>> {
    spans
        .into_iter()
        .map(|s| Span { start: s.start.max(from), end: s.end.min(to), subject: s.subject })
        .filter(|s| s.end > s.start)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        for day in [-800_000, -1, 0, 59, 60, 11_017, 20_000, 2_000_000] {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day);
            // A negative year has no "YYYY-MM-DD" form to parse back.
            if y >= 0 {
                assert_eq!(parse_date(&date(day)), Some(day), "{day}");
            }
        }
        assert_eq!(date(-1), "1969-12-31");
        assert_eq!(day_start(2, 3600), 2 * DAY - 3600);
    }

    #[test]
    fn parse_date_takes_only_plausible_dates() {
        assert_eq!(parse_date("1970-01-03"), Some(2));
        for bad in ["", "yesterday", "2026-13-01", "2026-01-00", "2026-01-32", "2026-01", "a-b-c"] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn local_times_shift_by_the_offset() {
        assert_eq!(local_time(0, 0), "1970-01-01 00:00");
        assert_eq!(local_time(0, -3600), "1969-12-31 23:00");
        assert_eq!(local_time(3 * 3600 + 25 * 60, 7200), "1970-01-01 05:25");
    }

    #[test]
    fn durations_read_short() {
        assert_eq!(duration(-5), "0s");
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(12 * 60 + 5), "12m");
        assert_eq!(duration(3599), "59m");
        assert_eq!(duration(2 * 3600 + 5 * 60), "2h 05m");
    }

    #[test]
    fn clip_cuts_to_the_range() {
        let span = |start, end, subject| Span { start, end, subject };
        let cut = clip(vec![span(0, 100, 1), span(150, 300, 2), span(400, 500, 3)], 50, 200);
        assert_eq!(cut, [span(50, 100, 1), span(150, 200, 2)]);
    }
}
