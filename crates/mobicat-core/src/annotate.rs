//! Classification applied to every entry after parsing: app-only for Android
//! (noise tags), crash detection, and framework detection.

use std::collections::HashSet;
use std::sync::Arc;

use crate::entry::{Entry, Framework, Level, Platform, Source};

/// Android tags emitted inside the app's process by the OS rather than the
/// app's own code. Hidden while app-only is on. A trailing `*` is a prefix.
///
/// Chosen from a recorded API 35 emulator session; third-party SDKs bundled
/// with the app (Firebase, ads) are kept, as on iOS where frameworks inside
/// the `.app` count as the app.
pub const DEFAULT_ANDROID_NOISE: &[&str] = &[
    "nativeloader",
    "ApplicationLoaders",
    "GraphicsEnvironment",
    "ziparchive",
    "EGL_emulation",
    "HWUI",
    "OpenGLRenderer",
    "Gralloc4",
    "libEGL",
    "ProfileInstaller",
    "ResourcesManager",
    "WebViewFactory",
    "AutofillManager",
    "TrafficStats",
    "adservices",
    "CompatibilityChangeReporter",
    "InsetsController",
    "ImeTracker",
    "InputMethodManager",
    "chromium",
    "cr_*",
];

#[derive(Clone, Debug)]
pub struct Noise {
    exact: HashSet<String>,
    prefixes: Vec<String>,
}

impl Default for Noise {
    fn default() -> Self {
        Self::new(DEFAULT_ANDROID_NOISE.iter().copied(), std::iter::empty())
    }
}

impl Noise {
    /// `add` extends the default list; `keep` removes tags from it.
    pub fn new<'a>(
        add: impl IntoIterator<Item = &'a str>,
        keep: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let keep: HashSet<&str> = keep.into_iter().collect();
        let mut exact = HashSet::new();
        let mut prefixes = Vec::new();
        for t in add {
            if keep.contains(t) {
                continue;
            }
            match t.strip_suffix('*') {
                Some(p) => prefixes.push(p.to_owned()),
                None => {
                    exact.insert(t.to_owned());
                }
            }
        }
        Self { exact, prefixes }
    }

    /// Defaults plus `add`, minus `keep`.
    pub fn with_overrides(add: &[String], keep: &[String]) -> Self {
        Self::new(
            DEFAULT_ANDROID_NOISE
                .iter()
                .copied()
                .chain(add.iter().map(String::as_str)),
            keep.iter().map(String::as_str),
        )
    }

    pub fn is_noise(&self, tag: &str) -> bool {
        self.exact.contains(tag) || self.prefixes.iter().any(|p| tag.starts_with(p.as_str()))
    }
}

/// ART logs with the process name as its tag, keeping only the last 15
/// characters (`com.acme.shopping` → `m.acme.shopping`).
fn is_runtime_tag(tag: &str, package: &str) -> bool {
    let tail = package.len().saturating_sub(15);
    package.get(tail..) == Some(tag)
}

pub struct Annotator {
    pub noise: Arc<Noise>,
}

impl Default for Annotator {
    fn default() -> Self {
        Self {
            noise: Arc::new(Noise::default()),
        }
    }
}

impl Annotator {
    pub fn new(noise: Arc<Noise>) -> Self {
        Self { noise }
    }

    /// `apply`, by value.
    pub fn done(&self, mut e: Entry) -> Entry {
        self.apply(&mut e);
        e
    }

    pub fn apply(&self, e: &mut Entry) {
        if e.source == Source::System {
            return;
        }
        detect_framework(e);
        e.crash = is_crash(e);
        if e.platform == Platform::Android && e.source == Source::Log {
            let runtime = e.app.as_deref().is_some_and(|p| is_runtime_tag(&e.tag, p));
            e.app_only = e.crash || !(runtime || self.noise.is_noise(&e.tag));
        }
    }
}

fn is_crash(e: &Entry) -> bool {
    let m = &e.message;
    match e.platform {
        Platform::Android => {
            (&*e.tag == "AndroidRuntime" && m.starts_with("FATAL EXCEPTION"))
                || (&*e.tag == "libc" && m.starts_with("Fatal signal"))
                || (&*e.tag == "DEBUG" && m.starts_with("*** *** ***"))
        }
        Platform::Ios => {
            (e.level == Level::Fatal && m.contains("Fatal error:"))
                || m.contains("Terminating app due to uncaught exception")
                || (e.source != Source::Log && m.contains("Fatal error:"))
        }
    }
}

/// Kermit's default emoji per level.
fn kermit_level(emoji: &str) -> Option<Level> {
    Some(match emoji.trim_end_matches('\u{fe0f}') {
        "🔍" => Level::Verbose,
        "🐛" => Level::Debug,
        "ℹ" | "🔵" => Level::Info,
        "⚠" | "🟡" => Level::Warn,
        "❌" | "🔴" => Level::Error,
        "💣" | "🟣" => Level::Fatal,
        _ => return None,
    })
}

/// `<emoji> (Tag) message` → (emoji, tag, message).
fn split_kermit(m: &str) -> Option<(&str, &str, &str)> {
    let (emoji, rest) = m.split_once(' ')?;
    if emoji.is_empty() || emoji.chars().any(|c| c.is_ascii()) {
        return None;
    }
    let rest = rest.strip_prefix('(')?;
    let (tag, msg) = rest.split_once(") ")?;
    (!tag.is_empty() && tag.len() <= 64).then_some((emoji, tag, msg))
}

fn detect_framework(e: &mut Entry) {
    match e.platform {
        Platform::Android => {
            e.framework = match &*e.tag {
                "ReactNativeJS" | "ReactNative" => Framework::ReactNative,
                "flutter" => Framework::Flutter,
                t if t.starts_with("Capacitor") => Framework::Capacitor,
                _ => Framework::Native,
            };
        }
        Platform::Ios => {
            if e.subsystem.as_deref() == Some("com.facebook.react.log") {
                e.framework = Framework::ReactNative;
            } else if let Some(rest) = e.message.strip_prefix("flutter: ") {
                e.framework = Framework::Flutter;
                e.message = rest.into();
            } else if let Some(rest) = e.message.strip_prefix("⚡️  [") {
                // Capacitor: `⚡️  [log] - message`
                if let Some((kind, msg)) = rest.split_once("] - ") {
                    e.framework = Framework::Capacitor;
                    e.level = match kind {
                        "warn" => Level::Warn,
                        "error" => Level::Error,
                        _ => e.level,
                    };
                    e.message = msg.into();
                }
            } else if e.subsystem.is_none()
                && e.category.is_none()
                && let Some((emoji, tag, msg)) = split_kermit(&e.message)
                && let Some(level) = kermit_level(emoji)
            {
                e.framework = Framework::Kmp;
                e.level = level;
                e.tag = Arc::from(tag);
                e.message = msg.into();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn android(tag: &str, msg: &str) -> Entry {
        let mut e = Entry::system(
            Platform::Android,
            Arc::from("d"),
            Some(Arc::from("com.example.app")),
            0,
            Level::Info,
            msg,
        );
        e.source = Source::Log;
        e.tag = Arc::from(tag);
        e
    }

    fn ios(sub: Option<&str>, level: Level, msg: &str) -> Entry {
        let mut e = Entry::system(
            Platform::Ios,
            Arc::from("d"),
            Some(Arc::from("com.example.app")),
            0,
            level,
            msg,
        );
        e.source = Source::Log;
        e.tag = Arc::from("x");
        e.subsystem = sub.map(Arc::from);
        e
    }

    fn annotate(mut e: Entry) -> Entry {
        Annotator::default().apply(&mut e);
        e
    }

    #[test]
    fn android_noise_and_runtime_tag() {
        assert!(!annotate(android("nativeloader", "x")).app_only);
        assert!(!annotate(android("cr_media", "x")).app_only);
        assert!(!annotate(android("com.example.app", "Using CollectorTypeCMC GC.")).app_only);
        assert!(annotate(android("MainActivity", "x")).app_only);
        assert!(
            annotate(android("FirebaseApp", "x")).app_only,
            "bundled SDKs count as the app"
        );
        // 16+ character package: ART truncates the tag from the left.
        let mut e = android("m.acme.shopping", "x");
        e.app = Some(Arc::from("com.acme.shopping"));
        assert!(!annotate(e).app_only);
        let mut e = android("shopping", "x");
        e.app = Some(Arc::from("com.acme.shopping"));
        assert!(
            annotate(e).app_only,
            "a short tag that merely ends the package name is the app's"
        );
    }

    #[test]
    fn noise_overrides() {
        let n = Noise::with_overrides(&["Chatty".into(), "OkHttp*".into()], &["chromium".into()]);
        assert!(n.is_noise("Chatty") && n.is_noise("OkHttpClient") && n.is_noise("nativeloader"));
        assert!(!n.is_noise("chromium"));
    }

    #[test]
    fn crashes() {
        assert!(
            annotate(android(
                "AndroidRuntime",
                "FATAL EXCEPTION: main\nProcess: x"
            ))
            .crash
        );
        assert!(annotate(android("libc", "Fatal signal 11 (SIGSEGV), code 1")).crash);
        assert!(annotate(android("DEBUG", "*** *** *** *** *** ***")).crash);
        assert!(!annotate(android("AndroidRuntime", "Shutting down VM")).crash);
        let mut noisy_crash = android("DEBUG", "*** *** *** ***");
        noisy_crash.tag = Arc::from("DEBUG");
        assert!(annotate(noisy_crash).app_only);
        assert!(annotate(ios(None, Level::Fatal, "Foo.swift:1: Fatal error: nil")).crash);
        assert!(
            annotate(ios(
                None,
                Level::Error,
                "*** Terminating app due to uncaught exception 'NSRangeException'"
            ))
            .crash
        );
        assert!(!annotate(ios(None, Level::Fatal, "some fault")).crash);
    }

    #[test]
    fn frameworks() {
        assert_eq!(
            annotate(android("ReactNativeJS", "x")).framework,
            Framework::ReactNative
        );
        assert_eq!(
            annotate(android("flutter", "x")).framework,
            Framework::Flutter
        );
        assert_eq!(
            annotate(android("Capacitor/Console", "x")).framework,
            Framework::Capacitor
        );
        assert_eq!(
            annotate(android("MainActivity", "x")).framework,
            Framework::Native
        );

        assert_eq!(
            annotate(ios(Some("com.facebook.react.log"), Level::Info, "x")).framework,
            Framework::ReactNative
        );
        let f = annotate(ios(None, Level::Info, "flutter: hello"));
        assert_eq!((f.framework, &*f.message), (Framework::Flutter, "hello"));

        let k = annotate(ios(None, Level::Info, "⚠️ (Sync) slow response"));
        assert_eq!(
            (k.framework, k.level, &*k.tag, &*k.message),
            (Framework::Kmp, Level::Warn, "Sync", "slow response")
        );

        let c = annotate(ios(None, Level::Info, "⚡️  [error] - boom"));
        assert_eq!(
            (c.framework, c.level, &*c.message),
            (Framework::Capacitor, Level::Error, "boom")
        );

        let plain = annotate(ios(None, Level::Info, "(not kermit) text"));
        assert_eq!(plain.framework, Framework::Native);
    }
}
