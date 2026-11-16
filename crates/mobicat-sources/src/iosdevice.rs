//! Physical iOS devices over usbmuxd (USB, or Wi-Fi once paired), through
//! the device's `com.apple.os_trace_relay` service. Works without sudo and
//! without an iOS 17+ tunnel; the pairing record usbmuxd keeps is enough.
//!
//! The relay sends every process's entries (thousands a second), so the
//! app's are picked out here by process path, as on simulators.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use idevice::installation_proxy::InstallationProxyClient;
use idevice::lockdown::LockdownClient;
use idevice::os_trace_relay::{LogLevel, OsTraceLog, OsTraceRelayClient};
use idevice::provider::UsbmuxdProvider;
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection, UsbmuxdDevice};
use idevice::{IdeviceError, IdeviceService};
use mobicat_core::{Annotator, Entry, Level, Platform};
use tokio::sync::mpsc;

pub use crate::oslog::AppInfo;
use crate::oslog::{self, OsLogContext, OsRecord};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const LABEL: &str = "mobicat";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IosDevice {
    pub udid: String,
    pub name: String,
    /// iOS version, e.g. `26.6.1`.
    pub version: String,
    /// Connected over the network rather than USB.
    pub network: bool,
}

fn addr() -> UsbmuxdAddr {
    UsbmuxdAddr::from_env_var().unwrap_or_default()
}

async fn timed<T>(what: &str, f: impl Future<Output = Result<T, IdeviceError>>) -> Result<T> {
    tokio::time::timeout(QUERY_TIMEOUT, f)
        .await
        .map_err(|_| anyhow!("{what} timed out"))?
        .map_err(|e| anyhow!("{what}: {e}"))
}

/// Whether usbmuxd is reachable (it comes with macOS, iTunes on Windows,
/// and the `usbmuxd` package on Linux).
pub async fn available() -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(2), UsbmuxdConnection::default()).await,
        Ok(Ok(_))
    )
}

async fn usbmuxd_devices() -> Result<Vec<UsbmuxdDevice>> {
    let mut mux = timed("usbmuxd", UsbmuxdConnection::default()).await?;
    let mut devs = timed("usbmuxd", mux.get_devices()).await?;
    // A device on USB and Wi-Fi at once is listed twice; keep USB.
    devs.sort_by_key(|d| !matches!(d.connection_type, Connection::Usb));
    let mut seen = HashSet::new();
    devs.retain(|d| seen.insert(d.udid.clone()));
    Ok(devs)
}

fn provider(d: &UsbmuxdDevice) -> UsbmuxdProvider {
    d.to_provider(addr(), LABEL)
}

async fn find(udid: &str) -> Result<UsbmuxdDevice> {
    usbmuxd_devices()
        .await?
        .into_iter()
        .find(|d| d.udid == udid)
        .with_context(|| format!("iPhone {udid} is not connected"))
}

/// UDIDs of connected devices, without asking the devices anything.
pub async fn connected() -> Result<Vec<String>> {
    Ok(usbmuxd_devices()
        .await?
        .into_iter()
        .map(|d| d.udid)
        .collect())
}

/// The device's name, or its UDID if it does not answer.
pub async fn name(udid: &str) -> String {
    devices()
        .await
        .ok()
        .and_then(|ds| ds.into_iter().find(|d| d.udid == udid))
        .map_or_else(|| udid.to_owned(), |d| d.name)
}

/// Connected devices. Devices that do not answer (locked before the first
/// unlock, not trusted yet) are listed with what lockdown could tell.
pub async fn devices() -> Result<Vec<IosDevice>> {
    let mut out = Vec::new();
    for d in usbmuxd_devices().await? {
        let p = provider(&d);
        let mut name = String::new();
        let mut version = String::new();
        if let Ok(mut l) = timed("lockdown", LockdownClient::connect(&p)).await {
            let mut get = async |key: &str| {
                timed("lockdown", l.get_value(Some(key), None))
                    .await
                    .ok()
                    .and_then(|v| v.as_string().map(str::to_owned))
                    .unwrap_or_default()
            };
            name = get("DeviceName").await;
            version = get("ProductVersion").await;
        }
        out.push(IosDevice {
            name: if name.is_empty() {
                d.udid.clone()
            } else {
                name
            },
            udid: d.udid,
            version,
            network: !matches!(d.connection_type, Connection::Usb),
        });
    }
    Ok(out)
}

async fn apps(udid: &str, ids: Option<Vec<String>>) -> Result<Vec<(String, plist::Value)>> {
    let d = find(udid).await?;
    let p = provider(&d);
    let mut ip = timed("installation_proxy", InstallationProxyClient::connect(&p)).await?;
    let kind = if ids.is_some() { "Any" } else { "User" };
    let mut v: Vec<_> = timed("installation_proxy", ip.get_apps(Some(kind), ids))
        .await?
        .into_iter()
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(v)
}

/// Bundle IDs of user-installed apps.
pub async fn user_apps(udid: &str) -> Result<Vec<String>> {
    Ok(apps(udid, None)
        .await?
        .into_iter()
        .map(|(k, _)| k)
        .collect())
}

/// Look up an installed app. `None` if it is not installed on this device.
pub async fn app_info(udid: &str, bundle_id: &str) -> Result<Option<AppInfo>> {
    let found = apps(udid, Some(vec![bundle_id.to_owned()])).await?;
    let Some((_, v)) = found.into_iter().find(|(k, _)| k == bundle_id) else {
        return Ok(None);
    };
    Ok(parse_app(bundle_id, &v))
}

fn parse_app(bundle_id: &str, v: &plist::Value) -> Option<AppInfo> {
    let d = v.as_dictionary()?;
    let s = |k: &str| d.get(k).and_then(|x| x.as_string()).map(str::to_owned);
    Some(AppInfo {
        bundle_id: bundle_id.to_owned(),
        executable: s("CFBundleExecutable")?,
        app_path: s("Path")?.trim_end_matches('/').to_owned(),
    })
}

#[derive(Clone, Debug)]
pub struct DeviceStream {
    pub udid: String,
    pub app: AppInfo,
}

/// The relay's levels, named as `log stream` names them.
fn message_type(l: LogLevel) -> &'static str {
    match l {
        LogLevel::Notice => "Default",
        LogLevel::Info => "Info",
        LogLevel::Debug => "Debug",
        LogLevel::Error => "Error",
        LogLevel::Fault => "Fault",
    }
}

/// Unix nanoseconds. idevice (0.1.68) builds the time from seconds and
/// microseconds but passes the microseconds as nanoseconds, so a sub-second
/// part below 1 ms is really microseconds.
fn time_ns(l: &OsTraceLog) -> i64 {
    let t = l.timestamp.and_utc();
    let sub = i64::from(t.timestamp_subsec_nanos());
    let sub = if sub < 1_000_000 { sub * 1000 } else { sub };
    t.timestamp() * 1_000_000_000 + sub
}

fn record_entry(l: &OsTraceLog, ctx: &mut OsLogContext) -> Option<Entry> {
    let (subsystem, category) = l
        .label
        .as_ref()
        .map_or(("", ""), |x| (x.subsystem.as_str(), x.category.as_str()));
    let (pu, su) = (l.process_image_uuid.to_string(), l.image_uuid.to_string());
    oslog::to_entry(
        &OsRecord {
            message_type: message_type(l.level),
            subsystem,
            category,
            process_id: l.pid,
            thread_id: l.thread_id,
            process_image_path: &l.filename,
            sender_image_path: &l.image_name,
            process_image_uuid: &pu,
            sender_image_uuid: &su,
            message: &l.message,
        },
        time_ns(l),
        ctx,
    )
}

/// Stream the app's logs until the receiver is dropped. Reconnects when the
/// device goes away and comes back.
pub async fn stream(cfg: DeviceStream, tx: mpsc::Sender<Entry>) {
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
    ctx.same_install = true;
    let annot = Annotator::default();
    let mut pids = HashSet::new();
    let mut failed_before = false;
    loop {
        match follow(&cfg, &mut ctx, &annot, &mut pids, &tx).await {
            Ok(()) => return, // receiver gone
            Err(e) => {
                if !failed_before
                    && tx
                        .send(note(Level::Warn, format!("{e:#}; retrying")))
                        .await
                        .is_err()
                {
                    return;
                }
                failed_before = true;
            }
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {}
            () = tx.closed() => return,
        }
    }
}

/// Follow the relay. `Ok` when the receiver is gone, `Err` when the device
/// connection fails.
async fn follow(
    cfg: &DeviceStream,
    ctx: &mut OsLogContext,
    annot: &Annotator,
    pids: &mut HashSet<u32>,
    tx: &mpsc::Sender<Entry>,
) -> Result<()> {
    let d = find(&cfg.udid).await?;
    let p = provider(&d);
    let client = timed("os_trace_relay", OsTraceRelayClient::connect(&p)).await?;
    let mut rx = timed("os_trace_relay", client.start_trace(None)).await?;
    loop {
        let l = tokio::select! {
            r = rx.next() => r.map_err(|e| anyhow!("device log stream: {e}"))?,
            () = tx.closed() => return Ok(()),
        };
        let Some(e) = record_entry(&l, ctx) else {
            continue;
        };
        if pids.insert(e.pid) && pids.len() > 1 {
            let mut n = Entry::system(
                Platform::Ios,
                e.device.clone(),
                e.app.clone(),
                e.time_ns,
                Level::Info,
                format!("process started (pid {})", e.pid),
            );
            n.time_ns = e.time_ns;
            if tx.send(n).await.is_err() {
                return Ok(());
            }
        }
        if tx.send(annot.done(e)).await.is_err() {
            return Ok(());
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
    fn reads_an_app_from_installation_proxy() {
        let mut d = plist::Dictionary::new();
        d.insert("CFBundleExecutable".into(), "Shelf".into());
        d.insert(
            "Path".into(),
            "/private/var/containers/Bundle/Application/AB12/Shelf.app/".into(),
        );
        let app = parse_app("com.example.shelf", &plist::Value::Dictionary(d)).unwrap();
        assert_eq!(app.executable, "Shelf");
        assert_eq!(
            app.app_path,
            "/private/var/containers/Bundle/Application/AB12/Shelf.app"
        );
        assert!(parse_app("x", &plist::Value::Boolean(true)).is_none());
    }

    fn log(pid: u32, path: &str, image: &str, subsystem: &str) -> OsTraceLog {
        OsTraceLog {
            pid,
            timestamp: chrono::DateTime::from_timestamp(1_763_262_703, 761_834)
                .unwrap()
                .naive_utc(),
            level: LogLevel::Error,
            image_name: image.into(),
            filename: path.into(),
            message: "hello".into(),
            label: (!subsystem.is_empty()).then(|| idevice::os_trace_relay::SyslogLabel {
                subsystem: subsystem.into(),
                category: "net".into(),
            }),
            procid: pid.into(),
            thread_id: 7,
            image_offset: 0,
            image_uuid: uuid::Uuid::nil(),
            process_image_uuid: uuid::Uuid::max(),
            mach_timestamp: 0,
        }
    }

    #[test]
    fn picks_the_apps_entries() {
        let app = "/private/var/containers/Bundle/Application/AB12/Shelf.app";
        let mut ctx = OsLogContext::new(
            Arc::from("PHONE"),
            Some(AppInfo {
                bundle_id: "com.example.shelf".into(),
                executable: "Shelf".into(),
                app_path: app.into(),
            }),
        );
        ctx.same_install = true;
        let own = log(
            9,
            &format!("{app}/Shelf"),
            &format!("{app}/Shelf"),
            "com.example.shelf",
        );
        let e = record_entry(&own, &mut ctx).unwrap();
        assert!(e.app_only);
        assert_eq!(e.level, Level::Error);
        assert_eq!(&*e.tag, "net");
        // The microseconds come back as microseconds.
        assert_eq!(e.time_ns, 1_763_262_703_761_834_000);

        let webkit = log(
            9,
            &format!("{app}/Shelf"),
            "/System/Library/Frameworks/WebKit.framework/WebKit",
            "com.apple.WebKit",
        );
        assert!(!record_entry(&webkit, &mut ctx).unwrap().app_only);

        // The release build of the same app, installed next to the debug one.
        let other = log(
            10,
            "/private/var/containers/Bundle/Application/CD34/Shelf.app/Shelf",
            "",
            "",
        );
        assert!(record_entry(&other, &mut ctx).is_none());
        let springboard = log(
            11,
            "/System/Library/CoreServices/SpringBoard.app/SpringBoard",
            "",
            "",
        );
        assert!(record_entry(&springboard, &mut ctx).is_none());
    }
}
