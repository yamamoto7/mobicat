//! Non-TUI output: NDJSON (`--json`) and one line per entry (`--plain`).

use std::io::{self, Write};

use serde::Serialize;

use crate::clock::Clock;
use crate::entry::{Entry, Framework, Level, Platform, Source};

/// Bumped on incompatible changes to the JSON shape.
pub const JSON_SCHEMA: u32 = 1;

#[derive(Serialize)]
struct JsonEntry<'a> {
    schema: u32,
    platform: Platform,
    device: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<&'a str>,
    time: String,
    pid: u32,
    tid: u64,
    level: &'static str,
    tag: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    subsystem: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    library: Option<&'a str>,
    source: Source,
    app_only: bool,
    #[serde(skip_serializing_if = "is_false")]
    crash: bool,
    #[serde(skip_serializing_if = "is_native")]
    framework: Framework,
    message: &'a str,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_native(f: &Framework) -> bool {
    *f == Framework::Native
}

fn level_str(l: Level) -> &'static str {
    match l {
        Level::Verbose => "V",
        Level::Debug => "D",
        Level::Info => "I",
        Level::Warn => "W",
        Level::Error => "E",
        Level::Fatal => "F",
    }
}

pub fn write_json(w: &mut impl Write, e: &Entry, clock: &Clock) -> io::Result<()> {
    let j = JsonEntry {
        schema: JSON_SCHEMA,
        platform: e.platform,
        device: &e.device,
        app: e.app.as_deref(),
        time: clock.rfc3339(e.time_ns),
        pid: e.pid,
        tid: e.tid,
        level: level_str(e.level),
        tag: &e.tag,
        subsystem: e.subsystem.as_deref(),
        category: e.category.as_deref(),
        library: e.library.as_deref(),
        source: e.source,
        app_only: e.app_only,
        crash: e.crash,
        framework: e.framework,
        message: &e.message,
    };
    serde_json::to_writer(&mut *w, &j)?;
    w.write_all(b"\n")
}

pub const TAG_WIDTH: usize = 20;

/// Right-align a tag to `TAG_WIDTH` columns, truncating long tags.
pub fn pad_tag(tag: &str) -> String {
    if tag.chars().count() > TAG_WIDTH {
        tag.chars().take(TAG_WIDTH).collect()
    } else {
        format!("{tag:>TAG_WIDTH$}")
    }
}

mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const DIM: &str = "\x1b[2m";
    pub const GREEN: &str = "\x1b[32m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const RED: &str = "\x1b[31m";
    pub const FATAL: &str = "\x1b[1;37;41m";
}

fn level_color(l: Level) -> &'static str {
    match l {
        Level::Verbose => ansi::DIM,
        Level::Debug => ansi::CYAN,
        Level::Info => ansi::GREEN,
        Level::Warn => ansi::YELLOW,
        Level::Error => ansi::RED,
        Level::Fatal => ansi::FATAL,
    }
}

/// `HH:MM:SS.mmm P L tag message`, the same layout as the TUI.
pub fn write_plain(w: &mut impl Write, e: &Entry, clock: &Clock, color: bool) -> io::Result<()> {
    let time = clock.hms_millis(e.time_ns);
    let badge = e.platform.badge();
    let lvl = e.level.letter();
    if e.source == Source::System {
        return if color {
            writeln!(
                w,
                "{}{time} {badge}   ── {} ──{}",
                ansi::DIM,
                e.message,
                ansi::RESET
            )
        } else {
            writeln!(w, "{time} {badge}   ── {} ──", e.message)
        };
    }
    let tag = pad_tag(&e.tag);
    let msg: std::borrow::Cow<str> = if e.crash {
        format!("✖ CRASH  {}", e.message).into()
    } else {
        e.message.as_ref().into()
    };
    if !color {
        return writeln!(w, "{time} {badge} {lvl} {tag} {msg}");
    }
    let pc = match e.platform {
        Platform::Android => ansi::GREEN,
        Platform::Ios => ansi::MAGENTA,
    };
    let lc = level_color(e.level);
    let (mc, mr) = if e.level >= Level::Error {
        (lc, ansi::RESET)
    } else {
        ("", "")
    };
    writeln!(
        w,
        "{dim}{time}{r} {pc}{badge}{r} {lc}{lvl}{r} {tag} {mc}{msg}{mr}",
        dim = ansi::DIM,
        r = ansi::RESET,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use jiff::tz::TimeZone;

    use super::*;

    fn sample() -> Entry {
        Entry {
            platform: Platform::Ios,
            device: Arc::from("SIM-1"),
            app: Some(Arc::from("com.example.app")),
            time_ns: 1_794_057_430_100_001_000,
            pid: 80248,
            tid: 7,
            level: Level::Error,
            tag: Arc::from("network"),
            subsystem: Some(Arc::from("com.example.app")),
            category: Some(Arc::from("network")),
            library: Some(Arc::from("ExampleApp.debug.dylib")),
            source: Source::Log,
            app_only: true,
            crash: false,
            framework: crate::entry::Framework::Native,
            message: "request \"failed\"".into(),
        }
    }

    fn clock() -> Clock {
        Clock::with_tz(TimeZone::fixed(jiff::tz::offset(9)))
    }

    #[test]
    fn json_shape() {
        let mut out = Vec::new();
        write_json(&mut out, &sample(), &clock()).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            r#"{"schema":1,"platform":"ios","device":"SIM-1","app":"com.example.app","time":"2026-11-07T22:17:10.100001+09:00","pid":80248,"tid":7,"level":"E","tag":"network","subsystem":"com.example.app","category":"network","library":"ExampleApp.debug.dylib","source":"log","app_only":true,"message":"request \"failed\""}"#.to_owned() + "\n"
        );
    }

    #[test]
    fn json_marks_crashes_and_frameworks() {
        let mut e = sample();
        e.crash = true;
        e.framework = Framework::ReactNative;
        let mut out = Vec::new();
        write_json(&mut out, &e, &clock()).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#""crash":true,"framework":"react-native""#),
            "{s}"
        );
        let mut out = Vec::new();
        write_plain(&mut out, &e, &clock(), false).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("✖ CRASH  request"));
    }

    #[test]
    fn json_omits_absent_fields() {
        let mut e = sample();
        e.app = None;
        e.subsystem = None;
        e.category = None;
        e.library = None;
        let mut out = Vec::new();
        write_json(&mut out, &e, &clock()).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("subsystem") && !s.contains("\"app\""));
    }

    #[test]
    fn plain_without_color() {
        let mut out = Vec::new();
        write_plain(&mut out, &sample(), &clock(), false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "22:17:10.100 i E              network request \"failed\"\n"
        );
    }

    #[test]
    fn plain_system_line() {
        let e = Entry::system(
            Platform::Android,
            Arc::from("emu"),
            None,
            1_794_057_430_100_001_000,
            Level::Info,
            "process started (pid 3990)",
        );
        let mut out = Vec::new();
        write_plain(&mut out, &e, &clock(), false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "22:17:10.100 A   ── process started (pid 3990) ──\n"
        );
    }

    #[test]
    fn long_tags_are_truncated() {
        assert_eq!(pad_tag("abc").len(), TAG_WIDTH);
        assert_eq!(pad_tag(&"x".repeat(40)).len(), TAG_WIDTH);
    }
}
