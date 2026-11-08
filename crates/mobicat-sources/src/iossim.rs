//! iOS simulator source: `xcrun simctl spawn <udid> log stream --style ndjson`.
//!
//! Filtering by `process == "<executable>"` keeps following the app across
//! restarts (verified on iOS 26.5; see docs/technical-notes.md).

use std::collections::HashSet;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use mobicat_core::{Annotator, Entry, Level, Platform};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;

pub use crate::oslog::AppInfo;
use crate::oslog::{self, OsLogContext};
use crate::proc;

const QUERY_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimDevice {
    pub udid: String,
    pub name: String,
    pub runtime: String,
}

pub fn available() -> bool {
    cfg!(target_os = "macos") && proc::on_path("xcrun")
}

fn simctl() -> Command {
    let mut c = Command::new("xcrun");
    c.arg("simctl");
    c
}

pub async fn booted() -> Result<Vec<SimDevice>> {
    let mut c = simctl();
    c.args(["list", "devices", "booted", "-j"]);
    parse_booted(&proc::output(c, QUERY_TIMEOUT).await?)
}

#[derive(Deserialize)]
struct DeviceList {
    devices: std::collections::BTreeMap<String, Vec<DeviceJson>>,
}

#[derive(Deserialize)]
struct DeviceJson {
    udid: String,
    name: String,
    state: String,
}

pub fn parse_booted(json: &str) -> Result<Vec<SimDevice>> {
    let list: DeviceList = serde_json::from_str(json).context("unexpected `simctl list` output")?;
    Ok(list
        .devices
        .into_iter()
        .flat_map(|(runtime, devs)| {
            let runtime = runtime
                .rsplit('.')
                .next()
                .unwrap_or(&runtime)
                .replacen('-', " ", 1)
                .replace('-', ".");
            devs.into_iter()
                .filter(|d| d.state == "Booted")
                .map(move |d| SimDevice {
                    udid: d.udid,
                    name: d.name,
                    runtime: runtime.clone(),
                })
        })
        .collect())
}

/// Look up an installed app. `None` if it is not installed on this simulator.
pub async fn app_info(udid: &str, bundle_id: &str) -> Result<Option<AppInfo>> {
    let mut c = simctl();
    c.args(["listapps", udid]);
    let plist = proc::output(c, QUERY_TIMEOUT).await?;
    let json = plist_to_json(&plist).await?;
    let Some((executable, path_hint)) = parse_listapps(&json, bundle_id)? else {
        return Ok(None);
    };
    let mut c = simctl();
    c.args(["get_app_container", udid, bundle_id, "app"]);
    let app_path = match proc::output(c, QUERY_TIMEOUT).await {
        Ok(p) => p.trim().trim_end_matches('/').to_owned(),
        Err(e) => path_hint.ok_or(e)?,
    };
    Ok(Some(AppInfo {
        bundle_id: bundle_id.to_owned(),
        executable,
        app_path,
    }))
}

/// Bundle IDs of user-installed apps.
/// Bundle IDs of running apps (`launchctl list` inside the simulator).
pub async fn running_apps(udid: &str) -> Result<Vec<String>> {
    let mut c = simctl();
    c.args(["spawn", udid, "launchctl", "list"]);
    Ok(parse_running(&proc::output(c, QUERY_TIMEOUT).await?))
}

/// `UIKitApplication:com.example.app[7cc7][rb-legacy]` → `com.example.app`.
pub fn parse_running(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split_once("UIKitApplication:"))
        .map(|(_, rest)| rest.split('[').next().unwrap_or(rest).trim().to_owned())
        .collect()
}

pub async fn user_apps(udid: &str) -> Result<Vec<String>> {
    let mut c = simctl();
    c.args(["listapps", udid]);
    let json = plist_to_json(&proc::output(c, QUERY_TIMEOUT).await?).await?;
    parse_user_apps(&json)
}

pub fn parse_user_apps(json: &str) -> Result<Vec<String>> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("unexpected `listapps` output")?;
    let mut ids: Vec<String> = v
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(_, a)| a.get("ApplicationType").and_then(|t| t.as_str()) == Some("User"))
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    Ok(ids)
}

/// `simctl listapps` prints an old-style plist; `plutil` converts it to JSON.
async fn plist_to_json(plist: &str) -> Result<String> {
    let mut child = Command::new("plutil")
        .args(["-convert", "json", "-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("could not run `plutil`")?;
    let mut stdin = child.stdin.take().context("no stdin")?;
    let data = plist.as_bytes().to_vec();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&data).await;
    });
    let out = tokio::time::timeout(QUERY_TIMEOUT, child.wait_with_output())
        .await
        .context("`plutil` timed out")??;
    let _ = writer.await;
    if !out.status.success() {
        anyhow::bail!(
            "`plutil` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Find the app's executable name and (if present) bundle path.
pub fn parse_listapps(json: &str, bundle_id: &str) -> Result<Option<(String, Option<String>)>> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("unexpected `listapps` output")?;
    let Some(app) = v.get(bundle_id) else {
        return Ok(None);
    };
    let Some(exe) = app.get("CFBundleExecutable").and_then(|e| e.as_str()) else {
        return Ok(None);
    };
    let path = app
        .get("Path")
        .and_then(|p| p.as_str())
        .map(|p| p.trim_end_matches('/').to_owned());
    Ok(Some((exe.to_owned(), path)))
}

fn predicate(executable: &str) -> String {
    let escaped = executable.replace('\\', "\\\\").replace('"', "\\\"");
    format!("process == \"{escaped}\"")
}

#[derive(Clone, Debug)]
pub struct SimStream {
    pub udid: String,
    pub app: AppInfo,
}

/// Stream the app's logs until the receiver is dropped. Restarts the stream
/// if it ends (simulator shut down and booted again).
pub async fn stream(cfg: SimStream, tx: mpsc::Sender<Entry>) {
    let device: Arc<str> = Arc::from(cfg.udid.as_str());
    let app_id: Arc<str> = Arc::from(cfg.app.bundle_id.as_str());
    let note = |level: Level, msg: String| {
        Entry::system(
            Platform::Ios,
            device.clone(),
            Some(app_id.clone()),
            now_ns(),
            level,
            msg,
        )
    };
    let mut ctx = OsLogContext::new(device.clone(), Some(cfg.app.clone()));
    let mut pids: HashSet<u32> = HashSet::new();
    let annot = Annotator::default();

    loop {
        let mut cmd = simctl();
        cmd.args([
            "spawn",
            &cfg.udid,
            "log",
            "stream",
            "--style",
            "ndjson",
            "--level",
            "debug",
            "--predicate",
        ])
        .arg(predicate(&cfg.app.executable));
        let mut s = match proc::spawn_stream(cmd) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(note(Level::Error, format!("{e:#}"))).await;
                return;
            }
        };
        loop {
            let line = match s.lines.next_line().await {
                Ok(Some(l)) => l,
                Ok(None) | Err(_) => break,
            };
            let Some(e) = oslog::parse_line(&line, &mut ctx) else {
                continue;
            };
            if pids.insert(e.pid) && pids.len() > 1 {
                let msg = format!("process started (pid {})", e.pid);
                let mut n = note(Level::Info, msg);
                n.time_ns = e.time_ns;
                if tx.send(n).await.is_err() {
                    return;
                }
            }
            if tx.send(annot.done(e)).await.is_err() {
                return;
            }
        }
        let stderr = s.finish().await;
        let why = stderr
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("getpwuid_r"))
            .unwrap_or("")
            .to_owned();
        let msg = if why.is_empty() {
            "log stream ended; retrying".to_owned()
        } else {
            format!("log stream ended ({why}); retrying")
        };
        if tx.send(note(Level::Warn, msg)).await.is_err() {
            return;
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {}
            () = tx.closed() => return,
        }
    }
}

fn now_ns() -> i64 {
    jiff::Timestamp::now().as_nanosecond() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_running_apps() {
        let text = "PID\tStatus\tLabel\n93698\t0\tUIKitApplication:com.example.app[7cc7][rb-legacy]\n-\t0\tcom.apple.other\n";
        assert_eq!(parse_running(text), ["com.example.app"]);
    }

    #[test]
    fn parses_booted_devices() {
        let json = r#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-26-5":[
            {"udid":"42177705-0307-42BA-B39E-8153FD6D932D","name":"iPhone 17 Pro","state":"Booted","isAvailable":true},
            {"udid":"X","name":"iPad","state":"Shutdown"}]}}"#;
        assert_eq!(
            parse_booted(json).unwrap(),
            vec![SimDevice {
                udid: "42177705-0307-42BA-B39E-8153FD6D932D".into(),
                name: "iPhone 17 Pro".into(),
                runtime: "iOS 26.5".into(),
            }]
        );
    }

    #[test]
    fn parses_listapps() {
        let json = r#"{"com.example.app":{"CFBundleExecutable":"ExampleApp","Path":"/x/ExampleApp.app/"},"com.other":{}}"#;
        assert_eq!(
            parse_listapps(json, "com.example.app").unwrap(),
            Some(("ExampleApp".into(), Some("/x/ExampleApp.app".into())))
        );
        assert_eq!(parse_listapps(json, "com.missing").unwrap(), None);
        assert_eq!(parse_listapps(json, "com.other").unwrap(), None);
    }

    #[test]
    fn user_apps_only() {
        let json = r#"{"com.apple.Maps":{"ApplicationType":"System"},"com.example.app":{"ApplicationType":"User"}}"#;
        assert_eq!(parse_user_apps(json).unwrap(), vec!["com.example.app"]);
    }

    #[test]
    fn predicate_escapes_quotes() {
        assert_eq!(predicate(r#"My "App""#), r#"process == "My \"App\"""#);
    }
}
