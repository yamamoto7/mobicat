//! Resolving settings, apps and devices, starting sources, and delivering
//! entries to the TUI or to stdout.

use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use mobicat_core::output::{write_json, write_plain};
use mobicat_core::store::now_ns;
use mobicat_core::{Clock, Entry, Filter, Level, Noise, Platform, Query, Source, Store};
use mobicat_sources::android::{self, Adb, AndroidStream};
use mobicat_sources::iossim::{self, SimStream};
use mobicat_sources::{file, launch};
use mobicat_tui::{AppItem, TuiOptions};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::config::{self, Config};
use crate::{Cli, ColorChoice};

const BACKLOG: Duration = Duration::from_secs(300);
const CHANNEL: usize = 8192;
const DEFAULT_BUFFER: usize = 100_000;

fn runtime() -> Result<Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("start tokio runtime")
}

/// `mobicat devices`
pub fn devices() -> Result<()> {
    let rt = runtime()?;
    rt.block_on(async {
        let adb = Adb::default();
        if adb.available() {
            match adb.devices().await {
                Ok(ds) if ds.is_empty() => println!("Android: no devices"),
                Ok(ds) => {
                    for d in ds {
                        println!("A  {:<24} {}", d.serial, d.model.unwrap_or_default());
                    }
                }
                Err(e) => println!("Android: {e:#}"),
            }
        } else {
            println!("Android: adb not found (set ADB or add platform-tools to PATH)");
        }
        if iossim::available() {
            match iossim::booted().await {
                Ok(ds) if ds.is_empty() => println!("iOS: no booted simulators"),
                Ok(ds) => {
                    for d in ds {
                        println!("i  {:<38} {} ({})", d.udid, d.name, d.runtime);
                    }
                }
                Err(e) => println!("iOS: {e:#}"),
            }
        } else if cfg!(target_os = "macos") {
            println!("iOS: xcrun not found (install the Xcode command line tools)");
        }
    });
    Ok(())
}

/// `mobicat config`
pub fn show_config() -> Result<()> {
    let cfg = config::load()?;
    let user = config::user_path().map_or_else(|| "(unknown)".into(), |p| p.display().to_string());
    println!("user file:  {user}");
    match std::env::current_dir()
        .ok()
        .and_then(|d| config::find_repo_file(&d))
    {
        Some(p) => println!("repo file:  {}", p.display()),
        None => println!(
            "repo file:  none ({} not found here or above)",
            config::REPO_FILE
        ),
    }
    println!(
        "loaded:     {}",
        if cfg.sources.is_empty() {
            "nothing (defaults)".into()
        } else {
            cfg.sources
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    println!();
    let show = |k: &str, v: Option<String>| println!("{k:<10} {}", v.unwrap_or_else(|| "-".into()));
    show("app", cfg.app.clone());
    show("android", cfg.android.clone());
    show("ios", cfg.ios.clone());
    show("level", cfg.level.clone());
    show("query", cfg.query.clone());
    show("all", cfg.all.map(|b| b.to_string()));
    show("wrap", cfg.wrap.map(|b| b.to_string()));
    show("buffer", cfg.buffer.map(|b| b.to_string()));
    match cfg.resolved_profiles() {
        Ok(ps) if ps.is_empty() => println!("profiles   -"),
        Ok(ps) => {
            for (i, (n, q)) in ps.iter().enumerate() {
                let key = if i < 9 {
                    format!("[{}]", i + 1)
                } else {
                    "   ".into()
                };
                println!("profile    {key} {n}: {q}");
            }
        }
        Err(e) => println!("profiles   error: {e:#}"),
    }
    if !cfg.noise_tags.is_empty() || !cfg.noise_keep.is_empty() {
        println!("noise      +{:?} -{:?}", cfg.noise_tags, cfg.noise_keep);
    }
    for w in &cfg.warnings {
        println!("warning:   {w}");
    }
    Ok(())
}

/// Everything the run needs, with config, profile and flags combined.
#[derive(Clone)]
struct Settings {
    android: Option<String>,
    ios: Option<String>,
    query: String,
    filter: Filter,
    buffer: usize,
    wrap: bool,
    profiles: Vec<(String, String)>,
    noise: Arc<Noise>,
    warnings: Vec<String>,
}

fn settings(cli: &Cli, cfg: Config) -> Result<Settings> {
    let mut warnings = cfg.warnings.clone();
    let profile_q = match &cli.profile {
        Some(p) => Some(cfg.profile_query(p)?),
        None => None,
    };
    let query = config::and_all(
        [
            cfg.query.as_deref(),
            profile_q.as_deref(),
            cli.query.as_deref(),
        ]
        .into_iter()
        .flatten(),
    );
    let parsed = Query::parse(&query).map_err(|e| anyhow::anyhow!("query `{query}`: {e}"))?;
    let min_level = cfg
        .level
        .as_deref()
        .and_then(Level::parse)
        .unwrap_or(Level::Verbose);
    let profiles = cfg.resolved_profiles().unwrap_or_else(|e| {
        warnings.push(format!("profiles: {e:#}"));
        vec![]
    });
    Ok(Settings {
        android: cli
            .android
            .clone()
            .or_else(|| cli.app.clone())
            .or(cfg.android)
            .or_else(|| cfg.app.clone()),
        ios: cli
            .ios
            .clone()
            .or_else(|| cli.app.clone())
            .or(cfg.ios)
            .or(cfg.app),
        filter: Filter {
            min_level,
            query: (!parsed.is_empty()).then(|| Arc::new(parsed)),
            highlight: false,
            app_only: !(cli.all || cfg.all.unwrap_or(false)),
        },
        query,
        buffer: cfg.buffer.unwrap_or(DEFAULT_BUFFER),
        wrap: cfg.wrap.unwrap_or(false),
        profiles,
        noise: Arc::new(Noise::with_overrides(&cfg.noise_tags, &cfg.noise_keep)),
        warnings,
    })
}

/// A source that resolved and is ready to start, with the device's name.
enum Planned {
    Android { cfg: AndroidStream, label: String },
    Ios { cfg: SimStream, label: String },
    File(Option<std::path::PathBuf>),
}

impl Planned {
    /// (device key, platform, name) for the TUI's targets.
    fn device(&self) -> Option<(String, Platform, String)> {
        match self {
            Planned::Android { cfg, label } => {
                Some((cfg.serial.clone(), Platform::Android, label.clone()))
            }
            Planned::Ios { cfg, label } => Some((cfg.udid.clone(), Platform::Ios, label.clone())),
            Planned::File(_) => None,
        }
    }
}

/// What the device watcher needs to start streams on devices that appear later.
struct Watch {
    tx: mpsc::Sender<Entry>,
    android: Option<String>,
    ios: Option<String>,
    only: Vec<String>,
    backlog: Duration,
    noise: Arc<Noise>,
}

/// Apps installed on the connected devices, for the picker: where each is
/// installed, whether it is running, and how recently it was used.
async fn installed_apps(cli: &Cli) -> Vec<AppItem> {
    use std::collections::{BTreeMap, HashSet};
    let wanted = |id: &str| cli.devices.is_empty() || cli.devices.iter().any(|d| d == id);
    let mut on: BTreeMap<String, (bool, bool)> = BTreeMap::new();
    let mut running: HashSet<String> = HashSet::new();
    let adb = Adb::default();
    if adb.available() {
        for d in adb.devices().await.unwrap_or_default() {
            if wanted(&d.serial) {
                for p in adb.user_packages(&d.serial).await.unwrap_or_default() {
                    on.entry(p).or_default().0 = true;
                }
                running.extend(adb.running(&d.serial).await.unwrap_or_default());
            }
        }
    }
    if iossim::available() {
        for d in iossim::booted().await.unwrap_or_default() {
            if wanted(&d.udid) {
                for b in iossim::user_apps(&d.udid).await.unwrap_or_default() {
                    on.entry(b).or_default().1 = true;
                }
                running.extend(iossim::running_apps(&d.udid).await.unwrap_or_default());
            }
        }
    }
    let recent = recent_apps();
    on.into_iter()
        .map(|(id, (android, ios))| AppItem {
            running: running.contains(&id),
            recent: recent
                .iter()
                .position(|r| *r == id)
                .filter(|&i| i < RECENT_SHOWN),
            id,
            android,
            ios,
        })
        .collect()
}

/// Recent apps shown in the picker, and kept in the file.
const RECENT_SHOWN: usize = 3;
const RECENT_KEPT: usize = 10;

fn recent_path() -> Option<std::path::PathBuf> {
    config::state_dir().map(|d| d.join("recent_apps"))
}

/// Apps used before, newest first.
fn recent_apps() -> Vec<String> {
    recent_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| {
            t.lines()
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn remember_app(id: &str) {
    let Some(p) = recent_path() else { return };
    let mut list = recent_apps();
    list.retain(|x| x != id);
    list.insert(0, id.to_owned());
    list.truncate(RECENT_KEPT);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, list.join("\n") + "\n");
}

/// Find the devices that have the app installed. Problems that do not stop
/// the run are returned as warnings.
async fn plan(cli: &Cli, s: &Settings) -> Result<(Vec<Planned>, Vec<String>)> {
    if let Some(path) = cli.file_path() {
        if let Some(p) = &path
            && !p.is_file()
        {
            bail!("{} is not a readable file", p.display());
        }
        return Ok((vec![Planned::File(path)], vec![]));
    }
    if s.android.is_none() && s.ios.is_none() {
        bail!("give an app ID, e.g. `mobicat com.example.app`, or set `app` in .mobicat.toml");
    }
    let wanted = |id: &str| cli.devices.is_empty() || cli.devices.iter().any(|d| d == id);
    let backlog = if cli.clear || cli.launch {
        Duration::ZERO
    } else {
        BACKLOG
    };
    let mut planned = Vec::new();
    let mut warnings = Vec::new();
    let mut checked = Vec::new();

    if let Some(pkg) = &s.android {
        let adb = Adb::default();
        if !adb.available() {
            warnings.push("Android: adb not found; skipping Android".into());
        } else {
            for d in adb.devices().await.unwrap_or_else(|e| {
                warnings.push(format!("Android: {e:#}"));
                vec![]
            }) {
                if !wanted(&d.serial) {
                    continue;
                }
                checked.push(d.serial.clone());
                match adb.package_uid(&d.serial, pkg).await {
                    Ok(Some(uid)) => planned.push(Planned::Android {
                        label: adb.display_name(&d).await,
                        cfg: AndroidStream {
                            adb: adb.clone(),
                            serial: d.serial,
                            package: pkg.clone(),
                            uid,
                            backlog,
                            noise: s.noise.clone(),
                        },
                    }),
                    Ok(None) => warnings.push(format!("{pkg} is not installed on {}", d.serial)),
                    Err(e) => warnings.push(format!("{}: {e:#}", d.serial)),
                }
            }
        }
    }

    if let Some(bundle) = &s.ios
        && iossim::available()
    {
        for d in iossim::booted().await.unwrap_or_else(|e| {
            warnings.push(format!("iOS: {e:#}"));
            vec![]
        }) {
            if !wanted(&d.udid) {
                continue;
            }
            checked.push(d.name.clone());
            match iossim::app_info(&d.udid, bundle).await {
                Ok(Some(app)) => planned.push(Planned::Ios {
                    label: d.name.clone(),
                    cfg: SimStream { udid: d.udid, app },
                }),
                Ok(None) => warnings.push(format!("{bundle} is not installed on {}", d.name)),
                Err(e) => warnings.push(format!("{}: {e:#}", d.name)),
            }
        }
    }

    if planned.is_empty() {
        let mut msg = String::from("no device has the app installed");
        if checked.is_empty() {
            msg = "no Android device or booted iOS simulator found (see `mobicat devices`)".into();
        }
        for w in &warnings {
            msg.push_str("\n  ");
            msg.push_str(w);
        }
        bail!(msg);
    }
    Ok((planned, warnings))
}

fn start(rt: &Runtime, planned: Vec<Planned>, tx: &mpsc::Sender<Entry>, relaunch: bool) {
    for p in planned {
        let tx = tx.clone();
        match p {
            Planned::Android { cfg, .. } => {
                if relaunch {
                    let (adb, serial, pkg) =
                        (cfg.adb.clone(), cfg.serial.clone(), cfg.package.clone());
                    let tx = tx.clone();
                    rt.spawn(async move {
                        // Let logcat attach first so the startup is captured.
                        tokio::time::sleep(Duration::from_millis(700)).await;
                        if let Err(e) = launch::restart_android(&adb, &serial, &pkg).await {
                            let msg = format!("--launch: {e:#}");
                            let _ = tx
                                .send(Entry::system(
                                    Platform::Android,
                                    Arc::from(serial.as_str()),
                                    None,
                                    now_ns(),
                                    Level::Error,
                                    msg,
                                ))
                                .await;
                        }
                    });
                }
                rt.spawn(android::stream(cfg, tx));
            }
            Planned::Ios { cfg, .. } => {
                if relaunch {
                    let (udid, bundle) = (cfg.udid.clone(), cfg.app.bundle_id.clone());
                    let tx = tx.clone();
                    rt.spawn(async move {
                        tokio::time::sleep(Duration::from_millis(700)).await;
                        launch::ios_console(udid, bundle, tx).await;
                    });
                }
                rt.spawn(iossim::stream(cfg, tx));
            }
            Planned::File(path) => {
                rt.spawn(async move {
                    let name = path
                        .as_ref()
                        .map_or_else(|| "stdin".into(), |p| p.display().to_string());
                    if let Err(e) = file::stream(path, tx.clone()).await {
                        let msg = format!("{e:#}");
                        let _ = tx
                            .send(Entry::system(
                                Platform::Android,
                                Arc::from(name.as_str()),
                                None,
                                0,
                                Level::Error,
                                msg,
                            ))
                            .await;
                    }
                });
            }
        }
    }
}

fn header(app: &str, planned: &[Planned]) -> String {
    match planned {
        [Planned::File(path)] => path
            .as_ref()
            .map_or_else(|| "stdin".into(), |p| p.display().to_string()),
        _ => app.to_owned(),
    }
}

/// `mobicat [APP]`: stream logs to the TUI, or to stdout with --json / --plain.
/// "Change app…" in the TUI comes back here: pick another app and start again.
pub fn logs(cli: &Cli) -> Result<()> {
    let base = settings(cli, config::load()?)?;
    let interactive = !(cli.json || cli.plain) && io::stdout().is_terminal();
    let mut chosen: Option<String> = None;
    let mut switching = false;
    loop {
        let mut s = base.clone();
        if let Some(id) = &chosen {
            s.android = Some(id.clone());
            s.ios = Some(id.clone());
        }
        let needs_app = cli.file.is_none() && s.android.is_none() && s.ios.is_none();
        if interactive && (needs_app || switching) {
            let items = runtime()?.block_on(installed_apps(cli));
            if items.is_empty() {
                bail!("no apps found on connected devices (see `mobicat devices`)");
            }
            match mobicat_tui::pick_app(items, switching)? {
                Some(id) => {
                    s.android = Some(id.clone());
                    s.ios = Some(id.clone());
                    chosen = Some(id);
                }
                // Back to the app that was open.
                None if switching => {}
                None => return Ok(()),
            }
        }
        if !session(cli, s, interactive)? {
            return Ok(());
        }
        switching = true;
    }
}

/// One run with one app. Returns true when the user asked to change app.
fn session(cli: &Cli, mut s: Settings, interactive: bool) -> Result<bool> {
    let rt = runtime()?;
    let (planned, mut warnings) = rt.block_on(plan(cli, &s))?;
    warnings.splice(0..0, s.warnings.drain(..));
    let app_label = s
        .android
        .clone()
        .or_else(|| s.ios.clone())
        .unwrap_or_default();
    let title = header(&app_label, &planned);
    if interactive && cli.file.is_none() && !app_label.is_empty() {
        remember_app(&app_label);
    }
    let has_ios = planned.iter().any(|p| matches!(p, Planned::Ios { .. }));
    let devices: Vec<_> = planned.iter().filter_map(Planned::device).collect();
    let live = cli.file.is_none();
    let (tx, rx) = mpsc::channel(CHANNEL);
    let watch = (live && interactive).then(|| Watch {
        tx: tx.clone(),
        android: s.android.clone(),
        ios: s.ios.clone(),
        only: cli.devices.clone(),
        backlog: if cli.clear { Duration::ZERO } else { BACKLOG },
        noise: s.noise.clone(),
    });
    start(&rt, planned, &tx, cli.launch);
    drop(tx);

    let result = if cli.json || cli.plain {
        for w in &warnings {
            eprintln!("mobicat: {w}");
        }
        let color = cli.plain
            && match cli.color {
                ColorChoice::Always => true,
                ColorChoice::Never => false,
                ColorChoice::Auto => {
                    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
                }
            };
        rt.block_on(print(rx, cli.json, color, s.filter.clone()))
            .map(|()| false)
    } else {
        let opts = TuiOptions {
            title,
            frame_interval: Duration::from_millis(33),
            profiles: s.profiles.clone(),
            ios_launch_hint: has_ios && !cli.launch,
            query: s.query.clone(),
            wrap: s.wrap,
            live,
            history_path: config::state_dir().map(|d| d.join("history")),
            can_change_app: interactive && cli.file.is_none(),
            ..Default::default()
        };
        tui(
            &rt,
            rx,
            opts,
            &warnings,
            s.filter.clone(),
            s.buffer,
            &devices,
            watch,
        )
    };
    // Dropping the tasks stops the child processes.
    rt.shutdown_timeout(Duration::from_millis(500));
    result
}

async fn print(
    mut rx: mpsc::Receiver<Entry>,
    json: bool,
    color: bool,
    filter: Filter,
) -> Result<()> {
    let clock = Clock::system();
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut batch = Vec::with_capacity(256);
    loop {
        batch.clear();
        let n = tokio::select! {
            n = rx.recv_many(&mut batch, 256) => n,
            _ = tokio::signal::ctrl_c() => return Ok(()),
        };
        if n == 0 {
            return Ok(()); // all sources finished (file input)
        }
        let now = now_ns();
        let written = (|| -> io::Result<()> {
            for e in &batch {
                if !filter.matches(e, now) || !script_wants(&filter, e, now) {
                    continue;
                }
                if json {
                    write_json(&mut out, e, &clock)?;
                } else {
                    write_plain(&mut out, e, &clock, color)?;
                }
            }
            out.flush()
        })();
        match written {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn tui(
    rt: &Runtime,
    mut rx: mpsc::Receiver<Entry>,
    opts: TuiOptions,
    warnings: &[String],
    filter: Filter,
    buffer: usize,
    devices: &[(String, Platform, String)],
    watch: Option<Watch>,
) -> Result<bool> {
    let mut store = Store::new(buffer);
    store.set_filter(filter);
    for (key, p, name) in devices {
        store.add_device(key, *p, name);
    }
    let now = now_ns();
    store.push_batch(warnings.iter().map(|w| {
        Entry::system(
            Platform::Android,
            Arc::from("mobicat"),
            None,
            now,
            Level::Warn,
            w.as_str(),
        )
    }));
    let store = Arc::new(Mutex::new(store));
    let ingest = store.clone();
    rt.spawn(async move {
        let mut batch = Vec::with_capacity(1024);
        while rx.recv_many(&mut batch, 1024).await > 0 {
            ingest
                .lock()
                .expect("store lock")
                .push_batch(batch.drain(..));
        }
    });
    if let Some(w) = watch {
        rt.spawn(watch_devices(store.clone(), w));
    }
    let stats = mobicat_tui::run(store, opts)?;
    let _ = io::stdout().flush();
    Ok(stats.change_app)
}

/// Every few seconds: mark devices connected or gone, and start streaming
/// from devices that appeared (and have the app installed).
async fn watch_devices(store: mobicat_tui::SharedStore, w: Watch) {
    let adb = Adb::default();
    let wanted = |id: &str| w.only.is_empty() || w.only.iter().any(|d| d == id);
    let use_adb = w.android.is_some() && adb.available();
    let use_sim = w.ios.is_some() && iossim::available();
    // Devices already looked at; a device without the app is not asked again.
    let mut checked: std::collections::HashSet<String> = Default::default();
    loop {
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(3)) => {}
            () = w.tx.closed() => return,
        }
        let mut present: Vec<(String, Platform, String)> = Vec::new();
        if use_adb && let Ok(ds) = adb.devices().await {
            for d in ds.into_iter().filter(|d| wanted(&d.serial)) {
                // Naming asks the emulator; only do it for devices not seen yet.
                let known = store
                    .lock()
                    .expect("store lock")
                    .devices()
                    .iter()
                    .any(|x| *x.key == *d.serial);
                let name = if known {
                    String::new()
                } else {
                    adb.display_name(&d).await
                };
                present.push((d.serial, Platform::Android, name));
            }
        }
        if use_sim && let Ok(ds) = iossim::booted().await {
            present.extend(
                ds.into_iter()
                    .filter(|d| wanted(&d.udid))
                    .map(|d| (d.udid, Platform::Ios, d.name)),
            );
        }
        let (known, new): (Vec<_>, Vec<_>) = {
            let st = store.lock().expect("store lock");
            present
                .into_iter()
                .partition(|(k, ..)| st.devices().iter().any(|d| &*d.key == k))
        };
        {
            let mut st = store.lock().expect("store lock");
            let keys: Vec<(String, Platform)> = st
                .devices()
                .iter()
                .map(|d| (d.key.to_string(), d.platform))
                .collect();
            for (k, p) in keys {
                let polled = match p {
                    Platform::Android => use_adb,
                    Platform::Ios => use_sim,
                };
                if polled {
                    st.set_connected(&k, known.iter().any(|(x, ..)| *x == k));
                }
            }
        }
        for (key, p, name) in new {
            if !checked.insert(key.clone()) {
                continue;
            }
            match p {
                Platform::Android => {
                    let Some(pkg) = &w.android else { continue };
                    if let Ok(Some(uid)) = adb.package_uid(&key, pkg).await {
                        store.lock().expect("store lock").add_device(&key, p, &name);
                        tokio::spawn(android::stream(
                            AndroidStream {
                                adb: adb.clone(),
                                serial: key,
                                package: pkg.clone(),
                                uid,
                                backlog: w.backlog,
                                noise: w.noise.clone(),
                            },
                            w.tx.clone(),
                        ));
                    }
                }
                Platform::Ios => {
                    let Some(bundle) = &w.ios else { continue };
                    if let Ok(Some(app)) = iossim::app_info(&key, bundle).await {
                        store.lock().expect("store lock").add_device(&key, p, &name);
                        tokio::spawn(iossim::stream(SimStream { udid: key, app }, w.tx.clone()));
                    }
                }
            }
        }
    }
}

/// The TUI always shows mobicat's own lines (restarts, disconnects). A script
/// that asks for `-q is:crash` wants exactly that, so with a query, mobicat's
/// own lines must match it too (`-q is:system` still gets them).
fn script_wants(filter: &Filter, e: &Entry, now: i64) -> bool {
    e.source != Source::System
        || filter
            .query
            .as_ref()
            .is_none_or(|q| q.is_empty() || q.matches(e, now))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn scripts_get_only_what_the_query_asks_for() {
        let restart = Entry::system(
            Platform::Android,
            Arc::from("d"),
            None,
            0,
            Level::Info,
            "process started (pid 1)",
        );
        let mut f = Filter::default();
        assert!(script_wants(&f, &restart, 0), "no query: keep restarts");
        f.query = Some(Arc::new(Query::parse("is:crash").unwrap()));
        assert!(!script_wants(&f, &restart, 0));
        f.query = Some(Arc::new(Query::parse("is:system").unwrap()));
        assert!(script_wants(&f, &restart, 0));
    }
}
