//! File and stdin source. Detects the format per line: `log stream` ndjson
//! (lines starting with `{`) or logcat text. Android bug reports
//! (`adb bugreport`, a zip) are opened and their logcat sections read.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use mobicat_core::{Annotator, Entry, Grouper, Level, Platform};
use tokio::io::AsyncRead;
use tokio::sync::mpsc;

use crate::logcat::{self, LogcatContext};
use crate::oslog::{self, OsLogContext};
use crate::proc::LineReader;

/// `None` reads stdin.
pub async fn stream(path: Option<PathBuf>, tx: mpsc::Sender<Entry>) -> Result<()> {
    let name: Arc<str> = match &path {
        Some(p) => Arc::from(
            p.file_name()
                .map_or_else(|| p.to_string_lossy(), |n| n.to_string_lossy())
                .as_ref(),
        ),
        None => Arc::from("stdin"),
    };
    match path {
        Some(p) if is_zip(&p) => {
            let p2 = p.clone();
            let text = tokio::task::spawn_blocking(move || bugreport_logs(&p2))
                .await
                .context("reading the bug report")??;
            read(text.as_bytes(), name, tx).await
        }
        Some(p) => {
            let f = tokio::fs::File::open(&p)
                .await
                .with_context(|| format!("could not open {}", p.display()))?;
            read(f, name, tx).await
        }
        None => read(tokio::io::stdin(), name, tx).await,
    }
}

fn is_zip(p: &std::path::Path) -> bool {
    use std::io::Read;
    let mut magic = [0_u8; 4];
    std::fs::File::open(p)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic == *b"PK\x03\x04"
}

/// Sections of a bug report that hold logcat output (`-v threadtime`).
const LOG_SECTIONS: [&str; 3] = ["SYSTEM LOG", "EVENT LOG", "CRASH LOG"];

/// The logcat lines of an `adb bugreport` zip: the main text file's
/// system, event and crash log sections.
pub fn bugreport_logs(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let f =
        std::fs::File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(f).context("not a readable zip file")?;
    let mut main = String::new();
    if let Ok(mut e) = zip.by_name("main_entry.txt") {
        e.read_to_string(&mut main)?;
    }
    let main = main.trim().to_owned();
    let name = if !main.is_empty() && zip.by_name(&main).is_ok() {
        main
    } else {
        let mut best: Option<(u64, String)> = None;
        for i in 0..zip.len() {
            let e = zip.by_index(i)?;
            let n = e.name()?.into_owned();
            if n.starts_with("bugreport")
                && n.ends_with(".txt")
                && best.as_ref().is_none_or(|b| e.size() > b.0)
            {
                best = Some((e.size(), n));
            }
        }
        best.map(|b| b.1)
            .context("no bugreport-*.txt in the zip; is it from `adb bugreport`?")?
    };
    let mut text = String::new();
    {
        let mut e = zip.by_name(&name)?;
        let mut raw = Vec::new();
        e.read_to_end(&mut raw)?;
        text.push_str(&String::from_utf8_lossy(&raw));
    }
    Ok(log_sections(&text))
}

/// Keep the lines inside `------ SYSTEM LOG (...) ------` and the other
/// logcat sections, up to the next `------ ` header.
fn log_sections(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("------ ") {
            inside = LOG_SECTIONS.iter().any(|s| rest.starts_with(s));
            continue;
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

async fn read(r: impl AsyncRead + Unpin, name: Arc<str>, tx: mpsc::Sender<Entry>) -> Result<()> {
    let mut lines = LineReader::new(r);
    let mut android = LogcatContext::new(name.clone(), None);
    let mut ios = OsLogContext::new(name.clone(), None);
    let mut unparsed = 0_u64;
    let mut grouper = Grouper::default();
    let annot = Annotator::default();
    while let Some(line) = lines.next_line().await? {
        let e = if line.trim_start().starts_with('{') {
            oslog::parse_line(&line, &mut ios)
        } else {
            logcat::parse_line(&line, &mut android)
        };
        match e {
            Some(e) => {
                if let Some(b) = grouper.push(e)
                    && tx.send(annot.done(b)).await.is_err()
                {
                    return Ok(());
                }
            }
            None if !line.trim().is_empty() && !line.starts_with("---------") => unparsed += 1,
            None => {}
        }
    }
    if let Some(b) = grouper.flush()
        && tx.send(annot.done(b)).await.is_err()
    {
        return Ok(());
    }
    if unparsed > 0 {
        let msg = format!("{name}: skipped {unparsed} lines in an unknown format");
        let _ = tx
            .send(Entry::system(
                Platform::Android,
                name,
                None,
                0,
                Level::Warn,
                msg,
            ))
            .await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "========================================================\n\
== dumpstate: 2026-11-16 13:20:00\n\
------ MEMORY INFO (/proc/meminfo) ------\n\
MemTotal:        2014464 kB\n\
------ SYSTEM LOG (logcat -v threadtime -v printable -v uid -d *:v) ------\n\
--------- beginning of main\n\
11-16 13:19:58.100  1000  1234  1250 I ActivityManager: Start proc 4242:com.example.shelf/u0a190 for top-activity\n\
11-16 13:19:59.200 10190  4242  4242 E AndroidRuntime: FATAL EXCEPTION: main\n\
------ 0.150s was the duration of 'SYSTEM LOG' ------\n\
------ EVENT LOG (logcat -b events -v threadtime -v printable -v uid -d *:v) ------\n\
11-16 13:19:59.300  1000  1234  1250 I am_crash: [4242,0,com.example.shelf]\n\
------ CPU INFO (top -b -n 1) ------\n\
Tasks: 400 total\n";

    #[test]
    fn keeps_only_the_log_sections() {
        let logs = log_sections(REPORT);
        assert_eq!(logs.lines().count(), 4, "{logs}");
        assert!(logs.contains("FATAL EXCEPTION"));
        assert!(logs.contains("am_crash"));
        assert!(!logs.contains("MemTotal") && !logs.contains("Tasks:"));
    }

    #[tokio::test]
    async fn opens_a_bugreport_zip() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("mobicat-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bugreport.zip");
        {
            let f = std::fs::File::create(&path).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("main_entry.txt", opts).unwrap();
            z.write_all(b"bugreport-sdk-2026-11-16.txt").unwrap();
            z.start_file("bugreport-sdk-2026-11-16.txt", opts).unwrap();
            z.write_all(REPORT.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        assert!(is_zip(&path));
        let (tx, mut rx) = mpsc::channel(64);
        stream(Some(path.clone()), tx).await.unwrap();
        let mut n = 0;
        while let Some(e) = rx.recv().await {
            assert_ne!(e.source, mobicat_core::Source::System, "{}", e.message);
            n += 1;
        }
        assert_eq!(n, 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn mixed_formats_from_a_reader() {
        let android = include_str!("../../../testdata/android/logcat-epoch-uid.txt");
        let ios = include_str!("../../../testdata/ios/sim-handmade.ndjson");
        let data = format!("{android}{ios}not a log line\n");
        let (tx, mut rx) = mpsc::channel(1024);
        read(data.as_bytes(), Arc::from("mixed.log"), tx)
            .await
            .unwrap();
        let mut entries = Vec::new();
        while let Some(e) = rx.recv().await {
            entries.push(e);
        }
        let n_android = entries
            .iter()
            .filter(|e| e.platform == Platform::Android && e.source == mobicat_core::Source::Log)
            .count();
        let n_ios = entries
            .iter()
            .filter(|e| e.platform == Platform::Ios)
            .count();
        // Consecutive lines with identical headers are joined into one entry.
        let n_lines: usize = entries
            .iter()
            .filter(|e| e.platform == Platform::Android && e.source == mobicat_core::Source::Log)
            .map(|e| e.message.lines().count().max(1))
            .sum();
        assert_eq!(
            n_lines,
            android.lines().filter(|l| !l.starts_with("---")).count()
        );
        assert!(n_android > 0);
        assert_eq!(n_ios, ios.lines().count());
        let last = entries.last().unwrap();
        assert_eq!(last.source, mobicat_core::Source::System);
        assert!(last.message.contains("skipped 1 lines"));
    }
}
