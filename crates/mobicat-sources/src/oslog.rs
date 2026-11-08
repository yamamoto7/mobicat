//! Parser for `log stream --style ndjson` records, with the app-only decision.

use std::borrow::Cow;
use std::sync::Arc;

use mobicat_core::{Entry, Interner, Level, Platform, Source};
use serde::Deserialize;

/// The fields of an ndjson record that mobicat uses.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record<'a> {
    #[serde(borrow)]
    timestamp: Cow<'a, str>,
    #[serde(default, borrow)]
    event_type: Cow<'a, str>,
    #[serde(default, borrow)]
    message_type: Cow<'a, str>,
    #[serde(default, borrow)]
    subsystem: Cow<'a, str>,
    #[serde(default, borrow)]
    category: Cow<'a, str>,
    #[serde(default, rename = "processID")]
    process_id: u32,
    #[serde(default, rename = "threadID")]
    thread_id: u64,
    // Paths arrive with escaped slashes (`\/`), so they cannot always borrow.
    #[serde(default, borrow)]
    process_image_path: Cow<'a, str>,
    #[serde(default, borrow)]
    sender_image_path: Cow<'a, str>,
    #[serde(default, rename = "processImageUUID", borrow)]
    process_image_uuid: Cow<'a, str>,
    #[serde(default, rename = "senderImageUUID", borrow)]
    sender_image_uuid: Cow<'a, str>,
    #[serde(default, borrow)]
    event_message: Cow<'a, str>,
}

/// The app whose logs are being streamed.
#[derive(Clone, Debug)]
pub struct AppInfo {
    pub bundle_id: String,
    pub executable: String,
    /// Path of the `.app` bundle, without a trailing slash.
    pub app_path: String,
}

pub struct OsLogContext {
    pub device: Arc<str>,
    pub app: Option<AppInfo>,
    app_id: Option<Arc<str>>,
    pub interner: Interner,
}

impl OsLogContext {
    pub fn new(device: Arc<str>, app: Option<AppInfo>) -> Self {
        let app_id = app.as_ref().map(|a| Arc::from(a.bundle_id.as_str()));
        Self {
            device,
            app,
            app_id,
            interner: Interner::default(),
        }
    }
}

/// Subsystem React Native logs to (RCTLog).
const REACT_NATIVE_SUBSYSTEM: &str = "com.facebook.react.log";

/// Parse one ndjson line. Returns `None` for non-JSON lines, non-log events
/// (activities), and records from a different app with the same executable name.
pub fn parse_line(line: &str, ctx: &mut OsLogContext) -> Option<Entry> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    let r: Record = serde_json::from_str(line).ok()?;
    if r.event_type != "logEvent" {
        return None;
    }
    if let Some(app) = &ctx.app
        && !is_inside(&r.process_image_path, &app.app_path)
    {
        return None;
    }
    let time_ns = parse_timestamp(&r.timestamp)?;
    let library = basename(&r.sender_image_path);
    let tag = if !r.category.is_empty() {
        &r.category
    } else if !r.subsystem.is_empty() {
        r.subsystem.rsplit('.').next().unwrap_or(&r.subsystem)
    } else {
        library
    };
    let app_only = match &ctx.app {
        Some(app) => is_app_only(&r, app),
        None => true,
    };
    Some(Entry {
        platform: Platform::Ios,
        device: ctx.device.clone(),
        app: ctx.app_id.clone(),
        time_ns,
        pid: r.process_id,
        tid: r.thread_id,
        level: Level::from_os_log(&r.message_type),
        tag: ctx.interner.get(tag),
        subsystem: ctx.interner.opt(&r.subsystem),
        category: ctx.interner.opt(&r.category),
        library: ctx.interner.opt(library),
        source: Source::Log,
        app_only,
        crash: false,
        framework: mobicat_core::Framework::Native,
        message: r.event_message.into_owned().into_boxed_str(),
    })
}

/// Whether a record comes from the app's own code rather than Apple frameworks.
///
/// Modeled on the predicate Flutter's tooling uses for simulators, plus
/// subsystem rules for apps that use OSLog/Logger and for React Native.
/// Apple subsystems are excluded even when emitted from inside the bundle:
/// Xcode's preview stub (`com.apple.Previews.StubExecutor`) logs from there.
fn is_app_only(r: &Record, app: &AppInfo) -> bool {
    if r.sender_image_path.ends_with("/libswiftCore.dylib") {
        return true; // fatalError, precondition failures
    }
    if r.subsystem == REACT_NATIVE_SUBSYSTEM {
        return true;
    }
    if !app.bundle_id.is_empty() && r.subsystem.starts_with(app.bundle_id.as_str()) {
        return true;
    }
    if r.subsystem.starts_with("com.apple.") {
        return false;
    }
    is_inside(&r.sender_image_path, &app.app_path)
        || (!r.sender_image_uuid.is_empty() && r.sender_image_uuid == r.process_image_uuid)
}

fn is_inside(path: &str, dir: &str) -> bool {
    path.len() > dir.len() && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'/'
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `2026-11-07 22:16:43.066999+0900` → Unix nanoseconds.
fn parse_timestamp(s: &str) -> Option<i64> {
    let ts = jiff::fmt::strtime::parse("%Y-%m-%d %H:%M:%S%.f%z", s)
        .ok()?
        .to_timestamp()
        .ok()?;
    i64::try_from(ts.as_nanosecond()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_PATH: &str = "/Users/dev/Library/Developer/CoreSimulator/Devices/42177705-0307-42BA-B39E-8153FD6D932D/data/Containers/Bundle/Application/FB5BC8B4-9FC9-4080-BE7F-711AE66AD299/ExampleApp.app";

    fn ctx() -> OsLogContext {
        OsLogContext::new(
            Arc::from("SIM"),
            Some(AppInfo {
                bundle_id: "com.example.app".into(),
                executable: "ExampleApp".into(),
                app_path: APP_PATH.into(),
            }),
        )
    }

    fn parse_all(text: &str) -> Vec<Entry> {
        let mut c = ctx();
        text.lines().filter_map(|l| parse_line(l, &mut c)).collect()
    }

    #[test]
    fn timestamp_with_offset() {
        assert_eq!(
            parse_timestamp("2026-11-07 22:17:10.100001+0900"),
            Some(1_794_057_430_100_001_000)
        );
    }

    #[test]
    fn recorded_stream_is_all_framework_noise() {
        let text = include_str!("../../../testdata/ios/sim-log-stream.ndjson");
        let entries = parse_all(text);
        let activities = text
            .lines()
            .filter(|l| l.contains("\"activityCreateEvent\""))
            .count();
        assert_eq!(entries.len(), text.lines().count() - activities);
        // The recorded app emits no logs of its own: everything is Apple frameworks
        // or Xcode's preview stub inside the bundle.
        assert!(
            entries.iter().all(|e| !e.app_only),
            "{:?}",
            entries
                .iter()
                .filter(|e| e.app_only)
                .map(|e| &e.message)
                .collect::<Vec<_>>()
        );
        let pids: std::collections::BTreeSet<_> = entries.iter().map(|e| e.pid).collect();
        assert_eq!(pids.into_iter().collect::<Vec<_>>(), vec![80225, 80248]);
        assert!(entries.iter().any(|e| e.level == Level::Error));
        // Multi-line messages arrive as one record.
        assert!(entries.iter().any(|e| e.message.contains('\n')));
    }

    #[test]
    fn handmade_app_only_rules() {
        let entries = parse_all(include_str!("../../../testdata/ios/sim-handmade.ndjson"));
        let by_msg = |m: &str| {
            entries
                .iter()
                .find(|e| e.message.contains(m))
                .unwrap_or_else(|| panic!("missing {m}"))
        };

        let own = by_msg("GET /api/items");
        assert!(own.app_only);
        assert_eq!(&*own.tag, "network");
        assert_eq!(own.library.as_deref(), Some("ExampleApp.debug.dylib"));
        assert_eq!(own.app.as_deref(), Some("com.example.app"));

        assert!(by_msg("timed out").app_only);
        assert_eq!(by_msg("timed out").level, Level::Error);

        let kermit = by_msg("(Sync) started");
        assert!(kermit.app_only, "embedded framework inside the bundle");
        assert_eq!(
            &*kermit.tag, "Kermit",
            "no category or subsystem: library name"
        );

        assert!(by_msg("rootTag").app_only, "React Native");
        assert!(!by_msg("nw_connection_create").app_only, "Apple framework");

        let fatal = by_msg("Fatal error");
        assert!(fatal.app_only);
        assert_eq!(fatal.level, Level::Fatal);

        assert!(
            entries
                .iter()
                .all(|e| !e.message.contains("same executable")),
            "a different app with the same executable name is dropped"
        );
    }

    #[test]
    fn without_app_info_everything_is_kept() {
        let mut c = OsLogContext::new(Arc::from("file"), None);
        let text = include_str!("../../../testdata/ios/sim-handmade.ndjson");
        let n = text
            .lines()
            .filter_map(|l| parse_line(l, &mut c))
            .filter(|e| e.app_only)
            .count();
        assert_eq!(n, text.lines().count());
    }

    #[test]
    fn escaped_slashes_as_log_stream_prints_them() {
        // `log stream` escapes `/` as `\/`; borrowed &str fields would fail here.
        let line = format!(
            r#"{{"timestamp":"2026-11-07 22:17:10.100001+0900","eventType":"logEvent","messageType":"Info","subsystem":"com.example.app","category":"net","processID":1,"threadID":2,"processImagePath":"{p}\/ExampleApp","senderImagePath":"{p}\/ExampleApp.debug.dylib","processImageUUID":"A","senderImageUUID":"B","eventMessage":"ok \"quoted\""}}"#,
            p = APP_PATH.replace('/', "\\/")
        );
        let e = parse_line(&line, &mut ctx()).expect("parses");
        assert!(e.app_only);
        assert_eq!(e.library.as_deref(), Some("ExampleApp.debug.dylib"));
        assert_eq!(&*e.message, "ok \"quoted\"");
    }

    #[test]
    fn ignores_non_json() {
        let mut c = ctx();
        assert!(parse_line("Filtering the log data using \"process == x\"", &mut c).is_none());
        assert!(parse_line("{not json", &mut c).is_none());
    }
}
