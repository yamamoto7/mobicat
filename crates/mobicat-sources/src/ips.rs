//! Crash reports (`.ips`) that macOS writes when an app crashes on a
//! simulator. They have what the log stream often lacks: the exception, and
//! the crashed thread's frames with file and line.
//!
//! An `.ips` file is one JSON header line followed by a JSON body.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use mobicat_core::{Entry, Level, Platform};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::oslog::AppInfo;

/// Frames shown from the crashed thread.
const MAX_FRAMES: usize = 25;

#[derive(Deserialize)]
struct Header {
    #[serde(default, rename = "bundleID")]
    bundle_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Body {
    #[serde(default)]
    proc_path: String,
    #[serde(default)]
    pid: u32,
    #[serde(default)]
    exception: Option<Exception>,
    #[serde(default)]
    termination: Option<Termination>,
    #[serde(default)]
    faulting_thread: usize,
    #[serde(default)]
    threads: Vec<Thread>,
    #[serde(default)]
    used_images: Vec<Image>,
}

#[derive(Deserialize)]
struct Exception {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    signal: String,
}

#[derive(Deserialize)]
struct Termination {
    #[serde(default)]
    indicator: String,
}

#[derive(Deserialize)]
struct Thread {
    #[serde(default)]
    queue: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    frames: Vec<Frame>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
    #[serde(default)]
    image_index: usize,
    #[serde(default)]
    image_offset: u64,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    symbol_location: Option<u64>,
    #[serde(default)]
    source_file: Option<String>,
    #[serde(default)]
    source_line: Option<u64>,
}

#[derive(Deserialize)]
struct Image {
    #[serde(default)]
    name: Option<String>,
}

/// A parsed report: who crashed, and the text to show.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub bundle_id: String,
    pub proc_path: String,
    pub pid: u32,
    pub text: String,
}

/// Parse an `.ips` crash report. `None` for other kinds of reports.
pub fn parse(text: &str) -> Option<Report> {
    let (head, body) = text.split_once('\n')?;
    let head: Header = serde_json::from_str(head).ok()?;
    let b: Body = serde_json::from_str(body).ok()?;
    let ex = b.exception.as_ref()?;
    let mut out = format!("Crash report: {}", ex.kind);
    if !ex.signal.is_empty() {
        out.push_str(&format!(" ({})", ex.signal));
    }
    if let Some(t) = &b.termination
        && !t.indicator.is_empty()
    {
        out.push_str(&format!(", {}", t.indicator));
    }
    if let Some(t) = b.threads.get(b.faulting_thread) {
        let label = t.queue.as_ref().or(t.name.as_ref());
        out.push_str(&format!("\nThread {} crashed", b.faulting_thread));
        if let Some(l) = label {
            out.push_str(&format!(" ({l})"));
        }
        out.push(':');
        let width = t
            .frames
            .iter()
            .take(MAX_FRAMES)
            .map(|f| image_name(&b, f).len())
            .max()
            .unwrap_or(0);
        for (i, f) in t.frames.iter().take(MAX_FRAMES).enumerate() {
            let image = image_name(&b, f);
            let what = match (&f.symbol, f.symbol_location) {
                (Some(s), Some(loc)) => format!("{s} + {loc}"),
                (Some(s), None) => s.clone(),
                (None, _) => format!("{:#x}", f.image_offset),
            };
            out.push_str(&format!("\n{i:<3} {image:<width$}  {what}"));
            if let Some(file) = &f.source_file
                && !file.starts_with('/')
            {
                match f.source_line {
                    Some(line) => out.push_str(&format!(" ({file}:{line})")),
                    None => out.push_str(&format!(" ({file})")),
                }
            }
        }
        if t.frames.len() > MAX_FRAMES {
            out.push_str(&format!(
                "\n... {} more frames",
                t.frames.len() - MAX_FRAMES
            ));
        }
    }
    Some(Report {
        bundle_id: head.bundle_id,
        proc_path: b.proc_path,
        pid: b.pid,
        text: out,
    })
}

fn image_name<'a>(b: &'a Body, f: &Frame) -> &'a str {
    b.used_images
        .get(f.image_index)
        .and_then(|i| i.name.as_deref())
        .unwrap_or("???")
}

/// Where macOS keeps crash reports, simulator apps included.
pub fn reports_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| Path::new(&h).join("Library/Logs/DiagnosticReports"))
}

/// Whether a report is about this app on this simulator.
fn is_ours(r: &Report, udid: &str, app: &AppInfo) -> bool {
    r.bundle_id == app.bundle_id && r.proc_path.contains(&format!("/Devices/{udid}/"))
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// New `.ips` files for the app's executable, modified after `since`.
fn new_reports(dir: &Path, executable: &str, since: SystemTime) -> Vec<(SystemTime, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let prefix = format!("{executable}-");
    let mut v: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "ips")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
        })
        .filter_map(|p| Some((modified(&p)?, p)))
        .filter(|(t, _)| *t > since)
        .collect();
    v.sort();
    v
}

/// The crash entry for a report.
pub fn entry(r: &Report, device: Arc<str>, app_id: Arc<str>, time_ns: i64, origin: &str) -> Entry {
    let mut e = Entry::system(
        Platform::Ios,
        device,
        Some(app_id),
        time_ns,
        Level::Fatal,
        format!("{}\nReport: {origin}", r.text),
    );
    e.pid = r.pid;
    e.source = mobicat_core::Source::Report;
    e.tag = Arc::from("CrashReport");
    e.crash = true;
    e
}

pub struct Watch {
    pub udid: String,
    pub app: AppInfo,
    /// Also report crashes from this far back.
    pub backlog: Duration,
    /// Look once and stop (`--last`).
    pub once: bool,
}

/// Turn new crash reports for the app into crash entries, until the
/// receiver is gone.
pub async fn watch(w: Watch, tx: mpsc::Sender<Entry>) {
    let Some(dir) = reports_dir() else { return };
    let device: Arc<str> = Arc::from(w.udid.as_str());
    let app_id: Arc<str> = Arc::from(w.app.bundle_id.as_str());
    let mut since = SystemTime::now()
        .checked_sub(w.backlog)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    loop {
        for (t, path) in new_reports(&dir, &w.app.executable, since) {
            since = since.max(t);
            // The file may still be being written; read it on the next round if not complete.
            let Ok(text) = tokio::fs::read_to_string(&path).await else {
                continue;
            };
            let Some(r) = parse(&text) else { continue };
            if !is_ours(&r, &w.udid, &w.app) {
                continue;
            }
            let time_ns = t
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
            let e = entry(
                &r,
                device.clone(),
                app_id.clone(),
                time_ns,
                &path.display().to_string(),
            );
            if tx.send(e).await.is_err() {
                return;
            }
        }
        if w.once {
            return;
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {}
            () = tx.closed() => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IPS: &str = r#"{"app_name":"Shelf","bundleID":"com.example.shelf","bug_type":"309","name":"Shelf"}
{
  "procPath" : "/Users/dev/Library/Developer/CoreSimulator/Devices/SIM-1/data/Containers/Bundle/Application/X/Shelf.app/Shelf",
  "pid" : 4242,
  "exception" : {"type" : "EXC_BREAKPOINT", "signal" : "SIGTRAP"},
  "termination" : {"indicator" : "Trace/BPT trap: 5"},
  "faultingThread" : 0,
  "threads" : [{"queue" : "com.apple.main-thread", "frames" : [
    {"imageIndex" : 0, "imageOffset" : 1055052, "symbol" : "_assertionFailure(_:_:file:line:flags:)", "symbolLocation" : 236},
    {"imageIndex" : 1, "imageOffset" : 47288, "symbol" : "CheckoutService.makeOrder(_:)", "symbolLocation" : 272, "sourceFile" : "ShelfApp.swift", "sourceLine" : 139},
    {"imageIndex" : 1, "imageOffset" : 41936, "symbol" : "thunk", "symbolLocation" : 48, "sourceFile" : "/<compiler-generated>"},
    {"imageIndex" : 2, "imageOffset" : 4096}
  ]}],
  "usedImages" : [{"name" : "libswiftCore.dylib"}, {"name" : "Shelf.debug.dylib"}, {}]
}"#;

    #[test]
    fn formats_the_crashed_thread() {
        let r = parse(IPS).unwrap();
        assert_eq!(r.bundle_id, "com.example.shelf");
        assert_eq!(r.pid, 4242);
        assert_eq!(
            r.text,
            "Crash report: EXC_BREAKPOINT (SIGTRAP), Trace/BPT trap: 5\n\
             Thread 0 crashed (com.apple.main-thread):\n\
             0   libswiftCore.dylib  _assertionFailure(_:_:file:line:flags:) + 236\n\
             1   Shelf.debug.dylib   CheckoutService.makeOrder(_:) + 272 (ShelfApp.swift:139)\n\
             2   Shelf.debug.dylib   thunk + 48\n\
             3   ???                 0x1000"
        );
        let app = AppInfo {
            bundle_id: "com.example.shelf".into(),
            executable: "Shelf".into(),
            app_path: String::new(),
        };
        assert!(is_ours(&r, "SIM-1", &app));
        assert!(!is_ours(&r, "SIM-2", &app));
    }

    #[test]
    fn other_reports_are_skipped() {
        assert!(parse("not json").is_none());
        assert!(parse("{\"bug_type\":\"298\"}\n{\"threads\":[]}").is_none());
    }
}
