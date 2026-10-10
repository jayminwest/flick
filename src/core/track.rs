//! Span clock: turns focus, idle and pause inputs into open/close/extend operations on
//! time spans, plus pure report helpers (local-day split, totals). It keeps no storage and
//! reads no clock: callers pass `now` (unix seconds) and persist the `Op`s they get back.
//! Generic over the subject type `S` (anything `Clone + PartialEq`), so window activity
//! (`Subject`) and task timers share the same rules. `civil`: dates, durations and range
//! clipping for the reports of both.

mod civil;

pub use civil::{clip, date, day_start, duration, local_time, parse_date};

/// Shortest span (seconds) that survives a focus change; shorter ones are flicker.
pub const MERGE_SECS: i64 = 2;
/// Longest stored title, in chars.
pub const TITLE_MAX: usize = 256;
/// Longest stored URL, in chars; a longer one is dropped, not cut.
pub const URL_MAX: usize = 2048;

/// What a span is about: an app (bundle id and display name), its window title when
/// titles are on, the browser's front tab URL when URLs are on, and the running task, if
/// any. Build it with `Subject::new` so the title is normalized and equal titles compare
/// equal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    pub app: String,
    pub name: String,
    pub title: Option<String>,
    pub task: Option<i64>,
    pub url: Option<String>,
}

impl Subject {
    pub fn new(app: &str, name: &str, title: Option<&str>, task: Option<i64>) -> Self {
        Subject {
            app: app.to_owned(),
            name: name.to_owned(),
            title: title.and_then(normalize_title),
            task,
            url: None,
        }
    }

    /// This subject with front tab URL `url`, trimmed; blank or longer than `URL_MAX`: none.
    #[must_use]
    pub fn with_url(self, url: Option<&str>) -> Self {
        let url = url.map(str::trim).filter(|u| !u.is_empty() && u.chars().count() <= URL_MAX);
        Subject { url: url.map(str::to_owned), ..self }
    }
}

/// True for leading status glyphs that terminals and agents animate in window titles:
/// spinners (braille, quarter circles), dots, check marks, stars, emoji and joiners.
fn is_status_glyph(c: char) -> bool {
    c.is_whitespace()
        || matches!(c,
            '\u{00B7}' | '\u{2022}' | '\u{2219}' | '\u{200D}' | '\u{FE0F}'
            | '\u{2190}'..='\u{21FF}'     // arrows
            | '\u{2300}'..='\u{23FF}'     // misc technical (⏺ ⏳)
            | '\u{2580}'..='\u{27BF}'     // blocks, shapes (◐ ●), misc symbols, dingbats (✳ ✓)
            | '\u{2800}'..='\u{28FF}'     // braille spinners
            | '\u{2B00}'..='\u{2BFF}'     // misc symbols and arrows
            | '\u{1F300}'..='\u{1FAFF}') // emoji
}

/// A window title fit to store and compare: leading status glyphs and surrounding
/// whitespace removed, capped at `TITLE_MAX` chars. `None` when nothing is left.
pub fn normalize_title(title: &str) -> Option<String> {
    let t = title.trim_start_matches(is_status_glyph).trim_end();
    (!t.is_empty()).then(|| t.chars().take(TITLE_MAX).collect())
}

/// An input to the clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input<S> {
    /// `S` is now in front (app activated, window or title changed, task started).
    Focus(S),
    /// No input for `secs` seconds.
    Idle { secs: u64 },
    /// Input resumed after `Idle`.
    Active,
    /// Stop counting: sleep, screen lock, recording off, quit, task stopped.
    Pause,
    /// Count again after `Pause`, with `S` in front.
    Resume(S),
}

/// What the caller must do to its stored spans, in order. At most one span is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op<S> {
    /// Start a span for `subject` at `at`.
    Open { subject: S, at: i64 },
    /// End the open span at `at`.
    Close { at: i64 },
    /// Move the open span's end to `at` (it stays open).
    Extend { at: i64 },
    /// Delete the open span: it was empty or flicker.
    Discard,
}

/// A stored span, `start..end` in unix seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span<S> {
    pub start: i64,
    pub end: i64,
    pub subject: S,
}

impl<S> Span<S> {
    pub fn secs(&self) -> i64 {
        (self.end - self.start).max(0)
    }
}

/// The span state machine. Feed it every `Input` with the current time.
#[derive(Clone, Debug)]
pub struct Clock<S = Subject> {
    merge_secs: i64,
    current: Option<S>,
    open: Option<(S, i64)>,
    idle: bool,
    paused: bool,
}

impl<S> Default for Clock<S> {
    fn default() -> Self {
        Clock::new(MERGE_SECS)
    }
}

impl<S> Clock<S> {
    /// A clock that drops spans shorter than `merge_secs` on a focus change. It starts
    /// running with nothing in front: the first `Focus` opens a span.
    pub fn new(merge_secs: i64) -> Self {
        Clock { merge_secs, current: None, open: None, idle: false, paused: false }
    }

    /// The open span's subject and start.
    pub fn open(&self) -> Option<(&S, i64)> {
        self.open.as_ref().map(|(s, at)| (s, *at))
    }

    pub fn is_idle(&self) -> bool {
        self.idle
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }
}

impl<S: Clone + PartialEq> Clock<S> {
    /// Adopt a span that is already stored as open (for example after a restart), without
    /// emitting ops. The clock runs afterwards.
    pub fn restore(&mut self, subject: S, start: i64) {
        self.current = Some(subject.clone());
        self.open = Some((subject, start));
        self.idle = false;
        self.paused = false;
    }

    /// Apply `input` at `now` and return the ops to persist.
    pub fn observe(&mut self, input: Input<S>, now: i64) -> Vec<Op<S>> {
        match input {
            Input::Focus(subject) => self.focus(subject, now),
            Input::Idle { secs } => {
                self.idle = true;
                let at = now.saturating_sub(i64::try_from(secs).unwrap_or(i64::MAX));
                self.close(at)
            }
            Input::Active => {
                self.idle = false;
                self.reopen(now)
            }
            Input::Pause => {
                self.paused = true;
                self.close(now)
            }
            Input::Resume(subject) => {
                self.paused = false;
                self.idle = false;
                self.focus(subject, now)
            }
        }
    }

    fn running(&self) -> bool {
        !self.idle && !self.paused
    }

    fn focus(&mut self, subject: S, now: i64) -> Vec<Op<S>> {
        self.current = Some(subject.clone());
        if !self.running() {
            return Vec::new();
        }
        let Some((open, start)) = self.open.take() else {
            return self.reopen(now);
        };
        if open == subject {
            self.open = Some((open, start));
            return vec![Op::Extend { at: now.max(start) }];
        }
        // Flicker: the brief span goes, and its time goes to the new subject.
        if now - start < self.merge_secs {
            self.open = Some((subject.clone(), start));
            return vec![Op::Discard, Op::Open { subject, at: start }];
        }
        self.open = Some((subject.clone(), now));
        vec![Op::Close { at: now }, Op::Open { subject, at: now }]
    }

    /// Open a span for the current subject at `now`, or extend the open one.
    fn reopen(&mut self, now: i64) -> Vec<Op<S>> {
        if !self.running() {
            return Vec::new();
        }
        if let Some((_, start)) = &self.open {
            return vec![Op::Extend { at: now.max(*start) }];
        }
        let Some(subject) = self.current.clone() else { return Vec::new() };
        self.open = Some((subject.clone(), now));
        vec![Op::Open { subject, at: now }]
    }

    /// Close the open span at `at`, never before its start; an empty span is discarded.
    fn close(&mut self, at: i64) -> Vec<Op<S>> {
        match self.open.take() {
            None => Vec::new(),
            Some((_, start)) if at <= start => vec![Op::Discard],
            Some(_) => vec![Op::Close { at }],
        }
    }
}

const DAY: i64 = 86_400;

/// The local day number (days since 1970-01-01 local) of `ts` at `utc_offset_secs`.
pub fn local_day(ts: i64, utc_offset_secs: i32) -> i64 {
    (ts + i64::from(utc_offset_secs)).div_euclid(DAY)
}

/// Split spans at local midnight. Each part comes with its local day number, so a span
/// across midnight counts on both days. Empty spans are dropped.
pub fn split_days<S: Clone>(spans: &[Span<S>], utc_offset_secs: i32) -> Vec<(i64, Span<S>)> {
    let offset = i64::from(utc_offset_secs);
    let mut out = Vec::new();
    for span in spans {
        let mut start = span.start;
        while start < span.end {
            let day = local_day(start, utc_offset_secs);
            let end = span.end.min((day + 1) * DAY - offset);
            out.push((day, Span { start, end, subject: span.subject.clone() }));
            start = end;
        }
    }
    out
}

/// Total seconds per key, largest first; equal totals sort by key.
pub fn sum_by<'a, S: 'a, K: Ord>(
    spans: impl IntoIterator<Item = &'a Span<S>>,
    key: impl Fn(&Span<S>) -> K,
) -> Vec<(K, i64)> {
    let mut totals = std::collections::BTreeMap::new();
    for span in spans {
        *totals.entry(key(span)).or_insert(0) += span.secs();
    }
    let mut out: Vec<(K, i64)> = totals.into_iter().collect();
    out.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str) -> Subject {
        Subject::new(id, id, None, None)
    }

    fn open(s: &Subject, at: i64) -> Op<Subject> {
        Op::Open { subject: s.clone(), at }
    }

    fn span(start: i64, end: i64, s: &str) -> Span<&str> {
        Span { start, end, subject: s }
    }

    #[test]
    fn normalize_strips_status_glyphs_and_caps_length() {
        assert_eq!(
            normalize_title("◐ System reboot readiness check").as_deref(),
            Some("System reboot readiness check")
        );
        assert_eq!(normalize_title("⠋ ✳ build  ").as_deref(), Some("build"));
        assert_eq!(normalize_title("🔴\u{FE0F} rec · notes").as_deref(), Some("rec · notes"));
        assert_eq!(normalize_title("-zsh").as_deref(), Some("-zsh"));
        assert_eq!(normalize_title("/Users/me").as_deref(), Some("/Users/me"));
        assert_eq!(normalize_title(" ◓ ⏺ "), None);
        assert_eq!(normalize_title(""), None);
        let long = "é".repeat(300);
        assert_eq!(normalize_title(&long).map(|t| t.chars().count()), Some(TITLE_MAX));
    }

    #[test]
    fn subject_new_normalizes_so_spinner_frames_compare_equal() {
        let a = Subject::new("com.wez", "WezTerm", Some("◐ job"), Some(7));
        let b = Subject::new("com.wez", "WezTerm", Some("◓ job"), Some(7));
        assert_eq!(a, b);
        assert_eq!(a.title.as_deref(), Some("job"));
        assert_eq!(Subject::new("x", "X", Some("⠋"), None).title, None);
    }

    #[test]
    fn with_url_trims_and_drops_blank_or_long_urls() {
        let s = Subject::new("b", "B", None, None);
        assert_eq!(s.url, None);
        let u = s.clone().with_url(Some(" https://a.dev/x \n"));
        assert_eq!(u.url.as_deref(), Some("https://a.dev/x"));
        assert_ne!(u, s, "a URL change is a subject change");
        assert_eq!(s.clone().with_url(Some("  ")).url, None);
        assert_eq!(s.clone().with_url(None).url, None);
        let long = format!("https://a.dev/{}", "x".repeat(URL_MAX));
        assert_eq!(s.with_url(Some(&long)).url, None);
    }

    #[test]
    fn focus_opens_then_same_subject_extends() {
        let mut c = Clock::default();
        let (a, b) = (app("a"), app("b"));
        assert_eq!(c.observe(Input::Focus(a.clone()), 100), [open(&a, 100)]);
        assert_eq!(c.open(), Some((&a, 100)));
        assert_eq!(c.observe(Input::Focus(a.clone()), 150), [Op::Extend { at: 150 }]);
        assert_eq!(c.observe(Input::Focus(b.clone()), 200), [Op::Close { at: 200 }, open(&b, 200)]);
        assert_eq!(c.open(), Some((&b, 200)));
    }

    #[test]
    fn flicker_shorter_than_merge_is_discarded_and_time_moves_on() {
        let mut c = Clock::default();
        c.observe(Input::Focus(app("a")), 0);
        c.observe(Input::Focus(app("b")), 100);
        let next = app("d");
        assert_eq!(c.observe(Input::Focus(next.clone()), 101), [Op::Discard, open(&next, 100)]);
        assert_eq!(c.open(), Some((&next, 100)));
        // At exactly merge_secs the span is kept.
        let last = app("e");
        assert_eq!(
            c.observe(Input::Focus(last.clone()), 102),
            [Op::Close { at: 102 }, open(&last, 102)]
        );
    }

    #[test]
    fn idle_backdates_close_never_before_start_and_active_reopens() {
        let mut c = Clock::default();
        let a = app("a");
        c.observe(Input::Focus(a.clone()), 1000);
        assert_eq!(c.observe(Input::Idle { secs: 60 }, 1100), [Op::Close { at: 1040 }]);
        assert!(c.is_idle());
        assert_eq!(c.open(), None);
        assert_eq!(c.observe(Input::Idle { secs: 120 }, 1160), []);
        // Focus while idle is remembered, not opened.
        let b = app("b");
        assert_eq!(c.observe(Input::Focus(b.clone()), 1170), []);
        assert_eq!(c.observe(Input::Active, 1200), [open(&b, 1200)]);
        assert!(!c.is_idle());
        // Idle reaching back past the start leaves nothing: the span is discarded.
        assert_eq!(c.observe(Input::Idle { secs: 60 }, 1230), [Op::Discard]);
        c.observe(Input::Active, 1300);
        assert_eq!(c.observe(Input::Idle { secs: u64::MAX }, 1400), [Op::Discard]);
    }

    #[test]
    fn active_without_idle_extends_and_with_nothing_in_front_does_nothing() {
        let mut c = Clock::default();
        assert_eq!(c.observe(Input::Active, 5), []);
        let a = app("a");
        c.observe(Input::Focus(a), 10);
        assert_eq!(c.observe(Input::Active, 20), [Op::Extend { at: 20 }]);
        // A clock running backwards never moves an end before the start.
        assert_eq!(c.observe(Input::Active, 3), [Op::Extend { at: 10 }]);
        assert_eq!(c.observe(Input::Focus(app("a")), 4), [Op::Extend { at: 10 }]);
    }

    #[test]
    fn pause_resume_round_trips() {
        let mut c = Clock::default();
        let (a, b) = (app("a"), app("b"));
        c.observe(Input::Focus(a.clone()), 0);
        assert_eq!(c.observe(Input::Pause, 50), [Op::Close { at: 50 }]);
        assert!(c.is_paused());
        assert_eq!(c.observe(Input::Pause, 60), []);
        assert_eq!(c.observe(Input::Focus(b), 70), []);
        assert_eq!(c.observe(Input::Active, 80), []);
        assert_eq!(c.observe(Input::Resume(a.clone()), 90), [open(&a, 90)]);
        assert!(!c.is_paused());
        // Resume while idle clears idle too; Resume while running acts as Focus.
        c.observe(Input::Idle { secs: 0 }, 100);
        assert_eq!(c.observe(Input::Resume(a.clone()), 110), [open(&a, 110)]);
        assert_eq!(c.observe(Input::Resume(a), 120), [Op::Extend { at: 120 }]);
        assert_eq!(c.observe(Input::Pause, 130), [Op::Close { at: 130 }]);
        // Pausing an empty span discards it.
        c.observe(Input::Resume(app("a")), 140);
        assert_eq!(c.observe(Input::Pause, 140), [Op::Discard]);
    }

    #[test]
    fn restore_adopts_an_open_span_and_runs() {
        let mut c: Clock<i64> = Clock::new(0);
        c.observe(Input::Pause, 0);
        c.restore(7, 100);
        assert!(!c.is_paused() && !c.is_idle());
        assert_eq!(c.open(), Some((&7, 100)));
        assert_eq!(
            c.observe(Input::Focus(8), 101),
            [Op::Close { at: 101 }, Op::Open { subject: 8, at: 101 }]
        );
    }

    #[test]
    fn split_days_cuts_at_local_midnight() {
        // 2h either side of local midnight at UTC+2 (local midnight = 22:00 UTC).
        let midnight = 20 * DAY - 2 * 3600;
        let out = split_days(&[span(midnight - 3600, midnight + 7200, "a")], 7200);
        assert_eq!(
            out,
            [
                (19, span(midnight - 3600, midnight, "a")),
                (20, span(midnight, midnight + 7200, "a"))
            ]
        );
        // Multi-day, negative offset, empty span dropped.
        let out = split_days(&[span(0, 2 * DAY, "b"), span(5, 5, "c")], -3600);
        assert_eq!(
            out.iter().map(|(d, s)| (*d, s.secs())).collect::<Vec<_>>(),
            [(-1, 3600), (0, DAY), (1, DAY - 3600)]
        );
        assert_eq!(local_day(-1, 0), -1);
    }

    #[test]
    fn sum_by_totals_largest_first_then_by_key() {
        let spans = [
            span(0, 10, "a"),
            span(10, 40, "b"),
            span(40, 50, "a"),
            span(50, 70, "c"),
            span(9, 1, "d"),
        ];
        assert_eq!(sum_by(&spans, |s| s.subject), [("b", 30), ("a", 20), ("c", 20), ("d", 0)]);
        assert!(sum_by(&[] as &[Span<&str>], |s| s.subject).is_empty());
    }
}
