//! Parser for `adb logcat` text output.
//!
//! Supported layouts (all `threadtime`-based):
//! - `-v threadtime -v uid -v epoch -v usec` (what mobicat requests):
//!   `   1794057757.468383 10210  3727  3727 I Tag: message`
//! - `-v threadtime` (the adb default, e.g. saved files):
//!   `11-07 23:17:38.914   574   721 D Tag: message`
//! - `-v threadtime -v uid`: the same with a uid column, which may be a name (`wifi`).

use std::sync::Arc;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use mobicat_core::{Entry, Interner, Level, Platform, Source};

/// Per-stream parsing context.
pub struct LogcatContext {
    pub device: Arc<str>,
    pub app: Option<Arc<str>>,
    pub interner: Interner,
    /// Used to resolve `threadtime` dates, which have no year or zone.
    pub tz: TimeZone,
    pub now: Timestamp,
}

impl LogcatContext {
    pub fn new(device: Arc<str>, app: Option<Arc<str>>) -> Self {
        Self {
            device,
            app,
            interner: Interner::default(),
            tz: TimeZone::try_system().unwrap_or(TimeZone::UTC),
            now: Timestamp::now(),
        }
    }
}

/// Parse one line. Returns `None` for blank lines, buffer headers
/// (`--------- beginning of main`) and lines that do not look like logcat.
pub fn parse_line(line: &str, ctx: &mut LogcatContext) -> Option<Entry> {
    let line = line.trim_end_matches(['\r', '\n']);
    let rest = line.trim_start();
    if rest.is_empty() || rest.starts_with("---------") {
        return None;
    }

    let (time_ns, rest) = parse_time(rest, ctx)?;

    // Up to four whitespace-separated columns before the level letter:
    // [uid] pid tid L
    let mut cols: [&str; 4] = [""; 4];
    let mut rest = rest;
    let mut level = None;
    for (i, slot) in cols.iter_mut().enumerate() {
        let (tok, after) = next_token(rest)?;
        rest = after;
        if i >= 2
            && tok.len() == 1
            && let Some(l) = Level::from_android(tok.chars().next()?)
        {
            level = Some((l, i));
            break;
        }
        *slot = tok;
    }
    let (level, at) = level?;
    let (pid, tid) = match at {
        2 => (cols[0], cols[1]),
        3 => (cols[1], cols[2]),
        _ => return None,
    };
    let pid: u32 = pid.parse().ok()?;
    let tid: u64 = tid.parse().ok()?;

    // "Tag: message". Tags may contain spaces but not ": ".
    let rest = rest.trim_start();
    let (tag, message) = match rest.find(": ") {
        Some(i) => (&rest[..i], &rest[i + 2..]),
        None => match rest.strip_suffix(':') {
            Some(t) => (t, ""),
            None => (rest, ""),
        },
    };

    Some(Entry {
        platform: Platform::Android,
        device: ctx.device.clone(),
        app: ctx.app.clone(),
        time_ns,
        pid,
        tid,
        level,
        tag: ctx.interner.get(tag.trim_end()),
        subsystem: None,
        category: None,
        library: None,
        source: Source::Log,
        app_only: true,
        crash: false,
        framework: mobicat_core::entry::Framework::Native,
        message: message.into(),
    })
}

fn next_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    Some((&s[..end], &s[end..]))
}

/// Parse the leading timestamp: epoch (`1794057757.468383`) or
/// threadtime (`11-07 23:17:38.914`). Returns nanoseconds and the remainder.
fn parse_time<'a>(s: &'a str, ctx: &LogcatContext) -> Option<(i64, &'a str)> {
    let (first, rest) = next_token(s)?;
    if first.as_bytes().first()?.is_ascii_digit() && !first.contains('-') {
        return Some((parse_epoch(first)?, rest));
    }
    let (clock, rest) = next_token(rest)?;
    Some((parse_threadtime(first, clock, ctx)?, rest))
}

fn parse_epoch(s: &str) -> Option<i64> {
    let (secs, frac) = s.split_once('.').unwrap_or((s, ""));
    let secs: i64 = secs.parse().ok()?;
    if frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let frac_ns: i64 = if frac.is_empty() {
        0
    } else {
        frac.parse::<i64>().ok()? * 10_i64.pow(9 - frac.len() as u32)
    };
    secs.checked_mul(1_000_000_000)?.checked_add(frac_ns)
}

/// `MM-DD` + `hh:mm:ss.mmm` in the local zone. The year is assumed to be the
/// current one, or the previous one if that would put the entry in the future.
fn parse_threadtime(date: &str, clock: &str, ctx: &LogcatContext) -> Option<i64> {
    let (mm, dd) = date.split_once('-')?;
    let (month, day): (i8, i8) = (mm.parse().ok()?, dd.parse().ok()?);
    let (hms, frac) = clock.split_once('.').unwrap_or((clock, "0"));
    let mut parts = hms.split(':');
    let h: i8 = parts.next()?.parse().ok()?;
    let m: i8 = parts.next()?.parse().ok()?;
    let sec: i8 = parts.next()?.parse().ok()?;
    let frac_ns: i32 = {
        let digits = frac.get(..frac.len().min(9))?;
        let v: i32 = digits.parse().ok()?;
        v * 10_i32.pow(9 - digits.len() as u32)
    };
    let year = ctx.now.to_zoned(ctx.tz.clone()).year();
    for y in [year, year - 1] {
        let dt = Date::new(y, month, day).ok()?.at(h, m, sec, frac_ns);
        let ts = dt.to_zoned(ctx.tz.clone()).ok()?.timestamp();
        if ts <= ctx.now + jiff::SignedDuration::from_hours(24) {
            return i64::try_from(ts.as_nanosecond()).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> LogcatContext {
        let mut c = LogcatContext::new(
            Arc::from("emulator-5554"),
            Some(Arc::from("com.example.app")),
        );
        c.tz = TimeZone::fixed(jiff::tz::offset(9));
        c.now = "2026-11-08T00:00:00Z".parse().unwrap();
        c
    }

    #[test]
    fn epoch_uid_layout() {
        let e = parse_line(
            "         1794057757.484206 10210  3727  3727 E om.example.app: Not starting debugger",
            &mut ctx(),
        )
        .unwrap();
        assert_eq!(e.time_ns, 1_794_057_757_484_206_000);
        assert_eq!((e.pid, e.tid), (3727, 3727));
        assert_eq!(e.level, Level::Error);
        assert_eq!(&*e.tag, "om.example.app");
        assert_eq!(&*e.message, "Not starting debugger");
    }

    #[test]
    fn threadtime_layout_without_and_with_uid() {
        let mut c = ctx();
        let a = parse_line(
            "11-07 23:17:38.914   574   721 D InetDiagMessage: Destroyed 0 sockets",
            &mut c,
        )
        .unwrap();
        assert_eq!((a.pid, a.tid, a.level), (574, 721, Level::Debug));
        assert_eq!(&*a.tag, "InetDiagMessage");
        // 2026-11-07 23:17:38.914 +09:00
        assert_eq!(a.time_ns, 1_794_061_058_914_000_000);

        let b = parse_line(
            "11-07 23:17:45.296  wifi   802   802 I wpa_supplicant: wlan0: CTRL-EVENT",
            &mut c,
        )
        .unwrap();
        assert_eq!((b.pid, b.tid, b.level), (802, 802, Level::Info));
        assert_eq!(&*b.tag, "wpa_supplicant");
        assert_eq!(&*b.message, "wlan0: CTRL-EVENT");
    }

    #[test]
    fn threadtime_in_the_future_means_last_year() {
        let mut c = ctx();
        let e = parse_line("12-31 23:00:00.000   1   1 I T: m", &mut c).unwrap();
        let ts = Timestamp::from_nanosecond(i128::from(e.time_ns)).unwrap();
        assert_eq!(ts.to_zoned(c.tz.clone()).year(), 2025);
    }

    #[test]
    fn skips_headers_and_garbage() {
        let mut c = ctx();
        assert!(parse_line("--------- beginning of main", &mut c).is_none());
        assert!(parse_line("", &mut c).is_none());
        assert!(parse_line("adb: device offline", &mut c).is_none());
        assert!(parse_line("1794057757.1 x y Z", &mut c).is_none());
    }

    #[test]
    fn tag_with_spaces_and_empty_message() {
        let mut c = ctx();
        let e = parse_line(
            "1794057757.000001 10210 1 2 W Some Tag: hello: world",
            &mut c,
        )
        .unwrap();
        assert_eq!(&*e.tag, "Some Tag");
        assert_eq!(&*e.message, "hello: world");
        let e = parse_line("1794057757.000001 10210 1 2 W Lonely:", &mut c).unwrap();
        assert_eq!(&*e.tag, "Lonely");
        assert_eq!(&*e.message, "");
    }

    #[test]
    fn recorded_fixture() {
        let text = include_str!("../../../testdata/android/logcat-epoch-uid.txt");
        let mut c = ctx();
        let entries: Vec<_> = text.lines().filter_map(|l| parse_line(l, &mut c)).collect();
        let data_lines = text.lines().filter(|l| !l.starts_with("---------")).count();
        assert_eq!(entries.len(), data_lines, "every data line parses");
        let pids: std::collections::BTreeSet<_> = entries.iter().map(|e| e.pid).collect();
        assert_eq!(pids.into_iter().collect::<Vec<_>>(), vec![3727, 3990]);
        assert!(entries.iter().any(|e| e.level == Level::Verbose));
        assert!(entries.windows(2).all(|w| w[0].time_ns <= w[1].time_ns));
    }

    #[test]
    fn recorded_threadtime_fixtures() {
        let mut c = ctx();
        for text in [
            include_str!("../../../testdata/android/logcat-threadtime.txt"),
            include_str!("../../../testdata/android/logcat-threadtime-uid.txt"),
        ] {
            for l in text
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with("---------"))
            {
                assert!(parse_line(l, &mut c).is_some(), "failed: {l}");
            }
        }
    }
}
