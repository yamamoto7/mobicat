//! Rejoining multi-line messages.
//!
//! logcat prints a message that contains newlines (stack traces, pretty-printed
//! JSON) as several lines with an identical header: same pid, tid, level, tag
//! and timestamp. `Grouper` turns those lines back into one entry. OSLog
//! records already carry newlines, so iOS entries pass through untouched.

use crate::entry::{Entry, Platform, Source};

/// Upper bound on lines joined into one entry, so a runaway block cannot
/// grow without limit.
pub const MAX_BLOCK_LINES: usize = 2000;

#[derive(Default)]
pub struct Grouper {
    pending: Option<Entry>,
    message: String,
    lines: usize,
}

fn continues(block: &Entry, next: &Entry) -> bool {
    block.platform == Platform::Android
        && block.source == Source::Log
        && next.source == Source::Log
        && block.time_ns == next.time_ns
        && block.pid == next.pid
        && block.tid == next.tid
        && block.level == next.level
        && block.tag == next.tag
}

impl Grouper {
    /// Feed one entry. Returns the previous block once `e` does not continue it.
    pub fn push(&mut self, e: Entry) -> Option<Entry> {
        if let Some(p) = &self.pending
            && continues(p, &e)
            && self.lines < MAX_BLOCK_LINES
        {
            self.message.push('\n');
            self.message.push_str(&e.message);
            self.lines += 1;
            return None;
        }
        let done = self.flush();
        self.message.clear();
        self.message.push_str(&e.message);
        self.lines = 1;
        self.pending = Some(e);
        done
    }

    /// Emit the block being collected, if any.
    pub fn flush(&mut self) -> Option<Entry> {
        let mut e = self.pending.take()?;
        if self.lines > 1 {
            e.message = std::mem::take(&mut self.message).into_boxed_str();
        }
        self.lines = 0;
        Some(e)
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::entry::Level;

    fn line(time_ns: i64, tid: u64, tag: &str, msg: &str) -> Entry {
        let mut e = Entry::system(
            Platform::Android,
            Arc::from("d"),
            None,
            time_ns,
            Level::Error,
            msg,
        );
        e.source = Source::Log;
        e.pid = 100;
        e.tid = tid;
        e.tag = Arc::from(tag);
        e
    }

    fn run(input: Vec<Entry>) -> Vec<Entry> {
        let mut g = Grouper::default();
        let mut out: Vec<Entry> = input.into_iter().filter_map(|e| g.push(e)).collect();
        out.extend(g.flush());
        out
    }

    #[test]
    fn joins_lines_with_identical_headers() {
        let out = run(vec![
            line(1, 1, "AndroidRuntime", "FATAL EXCEPTION: main"),
            line(1, 1, "AndroidRuntime", "Process: com.example.app, PID: 100"),
            line(
                1,
                1,
                "AndroidRuntime",
                "java.lang.IllegalStateException: boom",
            ),
            line(2, 1, "AndroidRuntime", "next message"),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(
            &*out[0].message,
            "FATAL EXCEPTION: main\nProcess: com.example.app, PID: 100\njava.lang.IllegalStateException: boom"
        );
        assert_eq!(&*out[1].message, "next message");
    }

    #[test]
    fn different_thread_or_tag_breaks_the_block() {
        let out = run(vec![
            line(1, 1, "A", "a1"),
            line(1, 2, "A", "other thread"),
            line(1, 2, "B", "other tag"),
        ]);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn system_entries_are_never_joined() {
        let mut a = line(1, 1, "mobicat", "x");
        a.source = Source::System;
        let b = a.clone();
        assert_eq!(run(vec![a, b]).len(), 2);
    }

    #[test]
    fn block_size_is_bounded() {
        let input: Vec<_> = (0..MAX_BLOCK_LINES + 5)
            .map(|i| line(1, 1, "T", &i.to_string()))
            .collect();
        let out = run(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].message.lines().count(), MAX_BLOCK_LINES);
    }
}
