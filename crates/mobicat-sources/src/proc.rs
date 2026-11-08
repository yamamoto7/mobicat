//! Running external commands (adb, xcrun) and reading their output line by line.

use std::io;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tokio::task::JoinHandle;

/// Longest line kept; the rest of a longer line is discarded.
pub const MAX_LINE: usize = 1 << 20;

/// Reads lines with a length cap and lossy UTF-8 decoding.
///
/// Cancel-safe: if `next_line` is dropped mid-read (for example by a timeout),
/// the bytes read so far stay buffered and the next call continues the line.
pub struct LineReader<R> {
    reader: BufReader<R>,
    buf: Vec<u8>,
    /// Discarding the rest of an overlong line.
    skipping: bool,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    pub fn new(r: R) -> Self {
        Self {
            reader: BufReader::with_capacity(64 * 1024, r),
            buf: Vec::new(),
            skipping: false,
        }
    }

    fn take_line(&mut self) -> String {
        while matches!(self.buf.last(), Some(b'\n' | b'\r')) {
            self.buf.pop();
        }
        let line = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        line
    }

    pub async fn next_line(&mut self) -> io::Result<Option<String>> {
        loop {
            let room = MAX_LINE.saturating_sub(self.buf.len());
            if room == 0 {
                // Overlong: deliver what fits and drop the rest of the line.
                self.skipping = true;
                return Ok(Some(self.take_line()));
            }
            let n = (&mut self.reader)
                .take(room as u64)
                .read_until(b'\n', &mut self.buf)
                .await?;
            if self.skipping {
                let ended = self.buf.last() == Some(&b'\n');
                self.buf.clear();
                if ended {
                    self.skipping = false;
                }
                if n == 0 {
                    return Ok(None);
                }
                continue;
            }
            if n == 0 {
                return Ok((!self.buf.is_empty()).then(|| self.take_line()));
            }
            if self.buf.last() == Some(&b'\n') {
                return Ok(Some(self.take_line()));
            }
        }
    }
}

/// A running command whose stdout is read line by line.
pub struct Stream {
    pub child: Child,
    pub lines: LineReader<ChildStdout>,
    stderr: JoinHandle<String>,
}

impl Stream {
    /// Wait for the process to exit and return the start of its stderr.
    pub async fn finish(mut self) -> String {
        let _ = self.child.wait().await;
        (&mut self.stderr).await.unwrap_or_default()
    }
}

/// Stop the command politely: SIGTERM first, SIGKILL a second later.
///
/// `xcrun simctl spawn` forwards SIGTERM to the process inside the simulator,
/// but SIGKILL leaves that process running until it next writes to the closed
/// pipe (verified with `log stream`).
impl Drop for Stream {
    fn drop(&mut self) {
        self.stderr.abort();
        if !matches!(self.child.try_wait(), Ok(None)) {
            return; // already exited
        }
        #[cfg(unix)]
        if let Some(pid) = self.child.id().and_then(|p| i32::try_from(p).ok()) {
            use nix::sys::signal::{Signal, kill};
            use nix::unistd::Pid;
            let pid = Pid::from_raw(pid);
            if kill(pid, Signal::SIGTERM).is_ok() {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(1));
                    let _ = kill(pid, Signal::SIGKILL);
                });
                return;
            }
        }
        let _ = self.child.start_kill();
    }
}

pub fn spawn_stream(mut cmd: Command) -> Result<Stream> {
    // Not kill_on_drop: `Drop for Stream` stops the process with SIGTERM first.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let program = cmd.as_std().get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .spawn()
        .with_context(|| format!("could not run `{program}`"))?;
    let stdout = child.stdout.take().context("no stdout")?;
    let stderr = child.stderr.take().context("no stderr")?;
    let stderr = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stderr.take(4096).read_to_string(&mut s).await;
        s
    });
    Ok(Stream {
        child,
        lines: LineReader::new(stdout),
        stderr,
    })
}

/// Run a command to completion and return its stdout. Fails on a non-zero
/// exit with the command's stderr in the message.
pub async fn output(mut cmd: Command, timeout: Duration) -> Result<String> {
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    let program = cmd.as_std().get_program().to_string_lossy().into_owned();
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .with_context(|| format!("`{program}` timed out"))?
        .with_context(|| format!("could not run `{program}`"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("`{program}` failed: {}", err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether `program` can be found on PATH.
pub fn on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let p = dir.join(program);
        p.is_file() || (cfg!(windows) && p.with_extension("exe").is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_lines_lossily_and_caps_length() {
        let mut data = b"one\r\ntwo\n\xffbad\n".to_vec();
        data.extend(std::iter::repeat_n(b'x', MAX_LINE + 10));
        data.extend(b"\nlast");
        let mut r = LineReader::new(&data[..]);
        assert_eq!(r.next_line().await.unwrap().as_deref(), Some("one"));
        assert_eq!(r.next_line().await.unwrap().as_deref(), Some("two"));
        assert_eq!(r.next_line().await.unwrap().as_deref(), Some("\u{fffd}bad"));
        assert_eq!(r.next_line().await.unwrap().unwrap().len(), MAX_LINE);
        assert_eq!(r.next_line().await.unwrap().as_deref(), Some("last"));
        assert_eq!(r.next_line().await.unwrap(), None);
    }

    #[tokio::test]
    async fn survives_cancellation_mid_line() {
        let (mut w, rd) = tokio::io::duplex(64);
        let mut r = LineReader::new(rd);
        use tokio::io::AsyncWriteExt;
        w.write_all(b"hel").await.unwrap();
        // Times out with a partial line buffered.
        let t = tokio::time::timeout(Duration::from_millis(20), r.next_line()).await;
        assert!(t.is_err());
        w.write_all(b"lo\nnext\n").await.unwrap();
        assert_eq!(r.next_line().await.unwrap().as_deref(), Some("hello"));
        assert_eq!(r.next_line().await.unwrap().as_deref(), Some("next"));
    }
}
