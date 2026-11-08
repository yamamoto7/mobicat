//! Android source: `adb logcat --uid=<app uid>`.
//!
//! Filtering by UID on the device keeps following the app across restarts
//! without tracking PIDs (verified on API 35; see docs/technical-notes.md).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use mobicat_core::{Annotator, Entry, Grouper, Level, Noise, Platform};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::logcat::{self, LogcatContext};
use crate::proc;

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for the next line of a multi-line block before emitting it.
const BLOCK_IDLE: Duration = Duration::from_millis(30);

#[derive(Clone, Debug)]
pub struct Adb {
    pub program: String,
}

impl Default for Adb {
    fn default() -> Self {
        Self {
            program: std::env::var("ADB").unwrap_or_else(|_| "adb".into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AndroidDevice {
    pub serial: String,
    pub model: Option<String>,
}

impl Adb {
    pub fn available(&self) -> bool {
        std::path::Path::new(&self.program).is_file() || proc::on_path(&self.program)
    }

    fn cmd(&self, serial: Option<&str>) -> Command {
        let mut c = Command::new(&self.program);
        if let Some(s) = serial {
            c.args(["-s", s]);
        }
        c
    }

    pub async fn devices(&self) -> Result<Vec<AndroidDevice>> {
        let mut c = self.cmd(None);
        c.args(["devices", "-l"]);
        Ok(parse_devices(&proc::output(c, QUERY_TIMEOUT).await?))
    }

    /// A name to show: the AVD name for emulators, else the model, else the serial.
    pub async fn display_name(&self, d: &AndroidDevice) -> String {
        if d.serial.starts_with("emulator-") {
            let mut c = self.cmd(Some(&d.serial));
            c.args(["emu", "avd", "name"]);
            if let Ok(out) = proc::output(c, QUERY_TIMEOUT).await
                && let Some(name) = out
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty() && *l != "OK")
            {
                // AVD names can't hold spaces; "Pixel_8" reads as "Pixel 8".
                return name.replace('_', " ");
            }
        }
        d.model.clone().unwrap_or_else(|| d.serial.clone())
    }

    /// The app's UID, or `None` if the package is not installed.
    pub async fn package_uid(&self, serial: &str, package: &str) -> Result<Option<u32>> {
        let mut c = self.cmd(Some(serial));
        c.args(["shell", "cmd", "package", "list", "packages", "-U", package]);
        Ok(parse_package_uid(
            &proc::output(c, QUERY_TIMEOUT).await?,
            package,
        ))
    }

    /// Third-party (user-installed) packages.
    pub async fn user_packages(&self, serial: &str) -> Result<Vec<String>> {
        let mut c = self.cmd(Some(serial));
        c.args(["shell", "pm", "list", "packages", "-3"]);
        let out = proc::output(c, QUERY_TIMEOUT).await?;
        let mut v: Vec<String> = out
            .lines()
            .filter_map(|l| l.trim().strip_prefix("package:").map(str::to_owned))
            .collect();
        v.sort();
        Ok(v)
    }

    /// Names of running processes (an app's main process is its package name).
    pub async fn running(&self, serial: &str) -> Result<Vec<String>> {
        let mut c = self.cmd(Some(serial));
        c.args(["shell", "ps", "-A", "-o", "NAME"]);
        let out = proc::output(c, QUERY_TIMEOUT).await?;
        Ok(out.lines().skip(1).map(|l| l.trim().to_owned()).collect())
    }

    /// The device's clock, in Unix nanoseconds (logcat `-T` uses device time).
    pub async fn device_time_ns(&self, serial: &str) -> Result<i64> {
        let mut c = self.cmd(Some(serial));
        c.args(["shell", "date", "+%s"]);
        let out = proc::output(c, QUERY_TIMEOUT).await?;
        let secs: i64 = out
            .trim()
            .parse()
            .with_context(|| format!("unexpected `date` output: {out:?}"))?;
        Ok(secs * 1_000_000_000)
    }
}

/// Parse `adb devices -l`, keeping devices in the `device` state.
pub fn parse_devices(text: &str) -> Vec<AndroidDevice> {
    text.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let serial = it.next()?;
            if it.next()? != "device" {
                return None;
            }
            let model = it
                .find_map(|kv| kv.strip_prefix("model:"))
                .map(|m| m.replace('_', " "));
            Some(AndroidDevice {
                serial: serial.to_owned(),
                model,
            })
        })
        .collect()
}

/// Parse `cmd package list packages -U <pkg>`. The command matches substrings,
/// so the package name must match exactly.
pub fn parse_package_uid(text: &str, package: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let name = it.next()?.strip_prefix("package:")?;
        if name != package {
            return None;
        }
        it.find_map(|kv| kv.strip_prefix("uid:"))?
            .split(',')
            .next()?
            .parse()
            .ok()
    })
}

#[derive(Clone, Debug)]
pub struct AndroidStream {
    pub adb: Adb,
    pub serial: String,
    pub package: String,
    pub uid: u32,
    /// How far back to start, in device time. `Duration::ZERO` = only new lines.
    pub backlog: Duration,
    /// Tags hidden by app-only.
    pub noise: Arc<Noise>,
}

/// `-T` argument in epoch form.
fn epoch_arg(ns: i64) -> String {
    let ms = ns.div_euclid(1_000_000);
    format!("{}.{:03}", ms.div_euclid(1000), ms.rem_euclid(1000))
}

fn looks_like_unsupported_uid(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("uid")
        && (s.contains("unrecognized") || s.contains("unknown") || s.contains("invalid"))
}

/// Stream the app's logs until the receiver is dropped. Reconnects when the
/// device goes away and comes back.
pub async fn stream(cfg: AndroidStream, tx: mpsc::Sender<Entry>) {
    let device: Arc<str> = Arc::from(cfg.serial.as_str());
    let app: Arc<str> = Arc::from(cfg.package.as_str());
    let note = |level: Level, time_ns: i64, msg: String| {
        Entry::system(
            Platform::Android,
            device.clone(),
            Some(app.clone()),
            time_ns,
            level,
            msg,
        )
    };

    let now = match cfg.adb.device_time_ns(&cfg.serial).await {
        Ok(t) => t,
        Err(e) => {
            let _ = tx
                .send(note(Level::Error, 0, format!("{}: {e:#}", cfg.serial)))
                .await;
            return;
        }
    };
    let mut since = now - i64::try_from(cfg.backlog.as_nanos()).unwrap_or(0);
    let mut last_ns = i64::MIN;
    let mut resuming = false;
    let mut pids: HashSet<u32> = HashSet::new();
    let mut ctx = LogcatContext::new(device.clone(), Some(app.clone()));
    let annot = Annotator::new(cfg.noise.clone());

    loop {
        let mut cmd = cfg.adb.cmd(Some(&cfg.serial));
        cmd.args([
            "logcat",
            &format!("--uid={}", cfg.uid),
            "-v",
            "threadtime",
            "-v",
            "uid",
            "-v",
            "epoch",
            "-v",
            "usec",
            "-b",
            "main,system,crash",
            "-T",
            &epoch_arg(since),
        ]);
        let mut s = match proc::spawn_stream(cmd) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(note(Level::Error, since, format!("{e:#}"))).await;
                return;
            }
        };
        let mut grouper = Grouper::default();
        loop {
            // While a multi-line block is open, wait only briefly for its next line.
            let next = if grouper.is_pending() {
                match tokio::time::timeout(BLOCK_IDLE, s.lines.next_line()).await {
                    Ok(r) => r,
                    Err(_) => {
                        if let Some(b) = grouper.flush()
                            && tx.send(annot.done(b)).await.is_err()
                        {
                            return;
                        }
                        continue;
                    }
                }
            } else {
                s.lines.next_line().await
            };
            let line = match next {
                Ok(Some(l)) => l,
                Ok(None) | Err(_) => break,
            };
            let Some(e) = logcat::parse_line(&line, &mut ctx) else {
                continue;
            };
            if resuming {
                // `-T` is inclusive: skip what was already delivered.
                if e.time_ns <= last_ns {
                    continue;
                }
                resuming = false;
            }
            if pids.insert(e.pid) && pids.len() > 1 {
                // Close the previous process's block before the separator.
                if let Some(b) = grouper.flush()
                    && tx.send(annot.done(b)).await.is_err()
                {
                    return;
                }
                let msg = format!("process started (pid {})", e.pid);
                if tx.send(note(Level::Info, e.time_ns, msg)).await.is_err() {
                    return;
                }
            }
            last_ns = last_ns.max(e.time_ns);
            if let Some(b) = grouper.push(e)
                && tx.send(annot.done(b)).await.is_err()
            {
                return;
            }
        }
        if let Some(b) = grouper.flush()
            && tx.send(annot.done(b)).await.is_err()
        {
            return;
        }

        let stderr = s.finish().await;
        let at = last_ns.max(since);
        if looks_like_unsupported_uid(&stderr) {
            let msg = format!(
                "{}: logcat on this device does not support --uid (older Android). Not supported yet.",
                cfg.serial
            );
            let _ = tx.send(note(Level::Error, at, msg)).await;
            return;
        }
        let why = stderr.lines().next().unwrap_or("").trim().to_owned();
        let msg = if why.is_empty() {
            "logcat ended; waiting for the device".to_owned()
        } else {
            format!("logcat ended ({why}); waiting for the device")
        };
        if tx.send(note(Level::Warn, at, msg)).await.is_err() {
            return;
        }

        loop {
            let mut w = cfg.adb.cmd(Some(&cfg.serial));
            w.arg("wait-for-device");
            let waited = tokio::select! {
                r = proc::output(w, Duration::from_secs(3600)) => r.is_ok(),
                () = tx.closed() => return,
            };
            if waited {
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        if tx
            .send(note(Level::Info, at, "device reconnected".into()))
            .await
            .is_err()
        {
            return;
        }
        since = at;
        resuming = last_ns != i64::MIN;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devices() {
        let text = "* daemon started successfully\nList of devices attached\nemulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:61\nR58M123ABC unauthorized usb:1-1 transport_id:3\n\n";
        assert_eq!(
            parse_devices(text),
            vec![AndroidDevice {
                serial: "emulator-5554".into(),
                model: Some("sdk gphone64 arm64".into())
            }]
        );
    }

    #[test]
    fn package_uid_requires_exact_match() {
        let text = "package:com.example.app.stg uid:10213\npackage:com.example.app uid:10210\n";
        assert_eq!(parse_package_uid(text, "com.example.app"), Some(10210));
        assert_eq!(parse_package_uid(text, "com.example"), None);
        assert_eq!(parse_package_uid("", "com.example.app"), None);
        // Multi-user devices print shared uids as a list.
        assert_eq!(
            parse_package_uid("package:a uid:10001,1010001\n", "a"),
            Some(10001)
        );
    }

    #[test]
    fn epoch_argument() {
        assert_eq!(epoch_arg(1_794_057_757_468_383_000), "1794057757.468");
    }

    #[test]
    fn detects_missing_uid_support() {
        assert!(looks_like_unsupported_uid(
            "logcat: unrecognized option '--uid=10210'"
        ));
        assert!(!looks_like_unsupported_uid("error: device offline"));
    }
}
