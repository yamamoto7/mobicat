use std::sync::Arc;

use serde::Serialize;

/// Log level shared by both platforms.
///
/// Android: V D I W E F(A). iOS: Debug→D, Info/Default→I, Error→E, Fault→F.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl Level {
    pub const ALL: [Level; 6] = [
        Level::Verbose,
        Level::Debug,
        Level::Info,
        Level::Warn,
        Level::Error,
        Level::Fatal,
    ];

    pub fn letter(self) -> char {
        match self {
            Level::Verbose => 'V',
            Level::Debug => 'D',
            Level::Info => 'I',
            Level::Warn => 'W',
            Level::Error => 'E',
            Level::Fatal => 'F',
        }
    }

    /// Parse an Android priority letter. `A` (assert) maps to `Fatal`.
    pub fn from_android(c: char) -> Option<Level> {
        Some(match c {
            'V' => Level::Verbose,
            'D' => Level::Debug,
            'I' => Level::Info,
            'W' => Level::Warn,
            'E' => Level::Error,
            'F' | 'A' => Level::Fatal,
            _ => return None,
        })
    }

    /// Parse a level name as used in queries and config (`warn`, `e`, ...).
    pub fn parse(s: &str) -> Option<Level> {
        Some(match s.to_ascii_lowercase().as_str() {
            "v" | "verbose" => Level::Verbose,
            "d" | "debug" => Level::Debug,
            "i" | "info" | "default" => Level::Info,
            "w" | "warn" | "warning" => Level::Warn,
            "e" | "error" => Level::Error,
            "f" | "fatal" | "a" | "assert" | "fault" => Level::Fatal,
            _ => return None,
        })
    }

    /// Map an OSLog `messageType`. iOS has no warning level.
    pub fn from_os_log(message_type: &str) -> Level {
        match message_type {
            "Debug" => Level::Debug,
            "Error" => Level::Error,
            "Fault" => Level::Fatal,
            // "Info", "Default", and anything unknown.
            _ => Level::Info,
        }
    }

    /// The next more severe level, saturating at `Fatal`.
    pub fn up(self) -> Level {
        let i = self as usize;
        Self::ALL[(i + 1).min(Self::ALL.len() - 1)]
    }

    /// The next less severe level, saturating at `Verbose`.
    pub fn down(self) -> Level {
        let i = self as usize;
        Self::ALL[i.saturating_sub(1)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Android,
    Ios,
}

impl Platform {
    pub fn badge(self) -> char {
        match self {
            Platform::Android => 'A',
            Platform::Ios => 'i',
        }
    }
}

/// Where an entry came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The platform log (logcat, OSLog).
    Log,
    /// The app's stdout (iOS `--launch`).
    Stdout,
    /// The app's stderr (iOS `--launch`).
    Stderr,
    /// Produced by mobicat itself: process restarts, disconnects, warnings.
    System,
}

/// The framework an entry was logged through, when it can be told.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Framework {
    #[default]
    Native,
    ReactNative,
    Flutter,
    Kmp,
    Capacitor,
}

impl Framework {
    pub fn name(self) -> &'static str {
        match self {
            Framework::Native => "native",
            Framework::ReactNative => "react-native",
            Framework::Flutter => "flutter",
            Framework::Kmp => "kmp",
            Framework::Capacitor => "capacitor",
        }
    }

    pub fn parse(s: &str) -> Option<Framework> {
        Some(match s {
            "native" => Framework::Native,
            "react-native" | "rn" | "reactnative" => Framework::ReactNative,
            "flutter" => Framework::Flutter,
            "kmp" | "kotlin" | "kermit" => Framework::Kmp,
            "capacitor" => Framework::Capacitor,
            _ => return None,
        })
    }
}

/// One log entry, normalized across platforms (docs/requirements.md §3).
#[derive(Clone, Debug)]
pub struct Entry {
    pub platform: Platform,
    /// Android serial or simulator UDID.
    pub device: Arc<str>,
    /// Package name / bundle ID, when known.
    pub app: Option<Arc<str>>,
    /// Unix time in nanoseconds.
    pub time_ns: i64,
    pub pid: u32,
    pub tid: u64,
    pub level: Level,
    /// Android tag; iOS category, else the last subsystem component, else the library.
    pub tag: Arc<str>,
    /// iOS only.
    pub subsystem: Option<Arc<str>>,
    /// iOS only.
    pub category: Option<Arc<str>>,
    /// iOS only: basename of the image that emitted the entry.
    pub library: Option<Arc<str>>,
    pub source: Source,
    /// Whether the entry is from the app's own code (docs/requirements.md §4).
    pub app_only: bool,
    /// Part of a crash (docs/requirements.md §9).
    pub crash: bool,
    pub framework: Framework,
    pub message: Box<str>,
}

impl Entry {
    /// A note from mobicat itself, shown inline (process started, disconnected, ...).
    pub fn system(
        platform: Platform,
        device: Arc<str>,
        app: Option<Arc<str>>,
        time_ns: i64,
        level: Level,
        message: impl Into<Box<str>>,
    ) -> Self {
        Self {
            platform,
            device,
            app,
            time_ns,
            pid: 0,
            tid: 0,
            level,
            tag: Arc::from("mobicat"),
            subsystem: None,
            category: None,
            library: None,
            source: Source::System,
            app_only: true,
            crash: false,
            framework: Framework::Native,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_up_down_saturate() {
        assert_eq!(Level::Fatal.up(), Level::Fatal);
        assert_eq!(Level::Verbose.down(), Level::Verbose);
        assert_eq!(Level::Info.up(), Level::Warn);
        assert_eq!(Level::Info.down(), Level::Debug);
    }

    #[test]
    fn level_mappings() {
        assert_eq!(Level::from_android('A'), Some(Level::Fatal));
        assert_eq!(Level::from_android('x'), None);
        assert_eq!(Level::from_os_log("Default"), Level::Info);
        assert_eq!(Level::from_os_log("Fault"), Level::Fatal);
    }
}
