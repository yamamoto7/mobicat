//! File and stdin source. Detects the format per line: `log stream` ndjson
//! (lines starting with `{`) or logcat text.

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
        Some(p) => {
            let f = tokio::fs::File::open(&p)
                .await
                .with_context(|| format!("could not open {}", p.display()))?;
            read(f, name, tx).await
        }
        None => read(tokio::io::stdin(), name, tx).await,
    }
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
