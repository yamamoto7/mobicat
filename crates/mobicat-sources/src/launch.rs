//! `--launch`: restart the app so its startup is captured. On iOS this also
//! captures the app's stdout/stderr (`print()`, Capacitor's `console.log`),
//! which never reaches `log stream`.

use std::sync::Arc;
use std::time::Duration;

use mobicat_core::{Annotator, Entry, Level, Platform, Source};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::android::Adb;
use crate::proc;

const TIMEOUT: Duration = Duration::from_secs(20);

/// Restart the app on a physical iPhone with `xcrun devicectl ... --console`
/// and turn its stdout and stderr into entries, like `ios_console` does for
/// simulators. devicectl prints its own progress first; app output starts
/// after "Waiting for the application to terminate".
pub async fn ios_device_console(udid: String, bundle_id: String, tx: mpsc::Sender<Entry>) {
    let device: Arc<str> = Arc::from(udid.as_str());
    let app: Arc<str> = Arc::from(bundle_id.as_str());
    let tag: Arc<str> = Arc::from("stdout");
    let annot = Annotator::default();
    let now = || jiff::Timestamp::now().as_nanosecond() as i64;
    let note = |level: Level, msg: String| {
        Entry::system(
            Platform::Ios,
            device.clone(),
            Some(app.clone()),
            now(),
            level,
            msg,
        )
    };
    if !cfg!(target_os = "macos") {
        let _ = tx
            .send(note(
                Level::Warn,
                "--launch on iPhones needs Xcode (macOS)".into(),
            ))
            .await;
        return;
    }
    let mut c = Command::new("xcrun");
    // Through a pipe the app's stdout is block-buffered and would arrive at
    // exit; Xcode sets the same variable for its console.
    c.env("DEVICECTL_CHILD_NSUnbufferedIO", "YES");
    c.args([
        "devicectl",
        "device",
        "process",
        "launch",
        "--console",
        "--terminate-existing",
        "--device",
        &udid,
        &bundle_id,
    ]);
    c.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => {
            let _ = tx
                .send(note(
                    Level::Error,
                    format!("--launch: could not run devicectl: {e}"),
                ))
                .await;
            return;
        }
    };
    // devicectl prints its progress and the app's stdout on stdout, and the
    // app's stderr (NSLog and friends) on stderr. Read both.
    let mut out = child.stdout.take().map(proc::LineReader::new);
    let mut err = child.stderr.take().map(proc::LineReader::new);
    let mut started = false;
    let mut failure = String::new();
    loop {
        let (line, is_err) = tokio::select! {
            l = async { out.as_mut().unwrap().next_line().await }, if out.is_some() => match l {
                Ok(Some(l)) => (l, false),
                _ => { out = None; continue; }
            },
            l = async { err.as_mut().unwrap().next_line().await }, if err.is_some() => match l {
                Ok(Some(l)) => (l, true),
                _ => { err = None; continue; }
            },
            else => break,
        };
        if !started {
            if line.starts_with("Waiting for the application to terminate") {
                started = true;
                if tx
                    .send(note(
                        Level::Info,
                        "relaunched the app; capturing stdout".into(),
                    ))
                    .await
                    .is_err()
                {
                    return;
                }
            } else if is_err && failure.is_empty() && !line.trim().is_empty() {
                failure = line.trim().to_owned();
            }
            continue;
        }
        if line.starts_with("App terminated due to") {
            let _ = tx.send(note(Level::Info, line)).await;
            continue;
        }
        let mut e = Entry::system(
            Platform::Ios,
            device.clone(),
            Some(app.clone()),
            now(),
            Level::Info,
            line,
        );
        e.source = if is_err {
            Source::Stderr
        } else {
            Source::Stdout
        };
        e.tag = tag.clone();
        if tx.send(annot.done(e)).await.is_err() {
            return;
        }
    }
    let _ = child.wait().await;
    if !started {
        let why = if failure.is_empty() {
            "devicectl failed".to_owned()
        } else {
            failure
        };
        let _ = tx
            .send(note(Level::Error, format!("--launch: {why}")))
            .await;
    }
}

/// Force-stop and start the app's launcher activity.
pub async fn restart_android(adb: &Adb, serial: &str, package: &str) -> anyhow::Result<()> {
    let mut c = Command::new(&adb.program);
    c.args(["-s", serial, "shell", "am", "force-stop", package]);
    proc::output(c, TIMEOUT).await?;
    let mut c = Command::new(&adb.program);
    c.args([
        "-s",
        serial,
        "shell",
        "cmd",
        "package",
        "resolve-activity",
        "--brief",
        "-c",
        "android.intent.category.LAUNCHER",
        package,
    ]);
    let out = proc::output(c, TIMEOUT).await?;
    let component = parse_resolved_activity(&out, package)
        .ok_or_else(|| anyhow::anyhow!("{package} has no launcher activity"))?;
    let mut c = Command::new(&adb.program);
    c.args(["-s", serial, "shell", "am", "start", "-n", &component]);
    proc::output(c, TIMEOUT).await?;
    Ok(())
}

/// `cmd package resolve-activity --brief` prints details, then `pkg/.Activity`.
pub fn parse_resolved_activity(text: &str, package: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| l.starts_with(package) && l.contains('/'))
        .map(str::to_owned)
}

/// Relaunch the iOS app attached to a pseudo-terminal and stream its output
/// as `stdout` entries until it exits or the receiver is dropped.
pub async fn ios_console(udid: String, bundle_id: String, tx: mpsc::Sender<Entry>) {
    let device: Arc<str> = Arc::from(udid.as_str());
    let app: Arc<str> = Arc::from(bundle_id.as_str());
    let tag: Arc<str> = Arc::from("stdout");
    let annot = Annotator::default();
    let now = || jiff::Timestamp::now().as_nanosecond() as i64;
    let note = |level: Level, msg: String| {
        Entry::system(
            Platform::Ios,
            device.clone(),
            Some(app.clone()),
            now(),
            level,
            msg,
        )
    };

    let mut c = Command::new("xcrun");
    // --console-pty keeps the app's stdout line-buffered, as in a terminal.
    c.args([
        "simctl",
        "launch",
        "--console-pty",
        "--terminate-running-process",
        &udid,
        &bundle_id,
    ]);
    let mut s = match proc::spawn_stream(c) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx
                .send(note(Level::Error, format!("--launch: {e:#}")))
                .await;
            return;
        }
    };
    if tx
        .send(note(
            Level::Info,
            "relaunched the app; capturing stdout".into(),
        ))
        .await
        .is_err()
    {
        return;
    }
    // simctl's own `<bundle id>: <pid>` line is block-buffered through the pipe
    // and may arrive late or only at exit, so it is dropped rather than relied on.
    let pid_line = format!("{bundle_id}: ");
    let pid = 0;
    while let Ok(Some(line)) = s.lines.next_line().await {
        if line.starts_with(&pid_line) && line[pid_line.len()..].trim().parse::<u32>().is_ok() {
            continue;
        }
        let mut e = Entry::system(
            Platform::Ios,
            device.clone(),
            Some(app.clone()),
            now(),
            Level::Info,
            line,
        );
        e.source = Source::Stdout;
        e.tag = tag.clone();
        e.pid = pid;
        if tx.send(annot.done(e)).await.is_err() {
            return;
        }
    }
    let _ = s.finish().await;
    let _ = tx
        .send(note(Level::Info, "app exited; stdout capture ended".into()))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_activity() {
        let text = "priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false\ncom.example.app/.MainActivity\n";
        assert_eq!(
            parse_resolved_activity(text, "com.example.app").as_deref(),
            Some("com.example.app/.MainActivity")
        );
        assert_eq!(
            parse_resolved_activity("No activity found\n", "com.example.app"),
            None
        );
    }
}
