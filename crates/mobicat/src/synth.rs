//! `mobicat dev synth`: the M0 performance spike on a synthetic stream.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use mobicat_core::Store;
use mobicat_sources::synthetic::{self, SynthConfig};
use mobicat_tui::TuiOptions;
use tokio::sync::mpsc;

#[derive(clap::Args)]
pub struct SynthArgs {
    /// Lines per second.
    #[arg(long, default_value_t = 2000)]
    rate: u32,
    /// Ring buffer capacity (entries).
    #[arg(long, default_value_t = 100_000)]
    buffer: usize,
    /// Minimum milliseconds between frames.
    #[arg(long, default_value_t = 33)]
    frame_ms: u64,
    /// Quit after this many seconds.
    #[arg(long)]
    seconds: Option<u64>,
    /// Simulate scrolling, pausing and filter changes.
    #[arg(long)]
    exercise: bool,
    /// Pre-fill the buffer before starting.
    #[arg(long)]
    prefill: bool,
    /// Write run statistics to this file on exit.
    #[arg(long)]
    stats_out: Option<PathBuf>,
}

pub fn run(args: SynthArgs) -> Result<()> {
    let mut store = Store::new(args.buffer);
    let prefilled = if args.prefill { args.buffer as u64 } else { 0 };
    if args.prefill {
        let mut g = synthetic::Generator::new(1);
        store.push_batch((0..args.buffer).map(|_| g.entry(0)));
    }
    let store = Arc::new(Mutex::new(store));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .context("start tokio runtime")?;

    let (tx, mut rx) = mpsc::channel(256);
    let cfg = SynthConfig {
        rate: args.rate,
        tick: Duration::from_millis(10),
    };
    runtime.spawn(synthetic::run(cfg, tx));
    let ingest_store = store.clone();
    runtime.spawn(async move {
        while let Some(batch) = rx.recv().await {
            ingest_store.lock().expect("store lock").push_batch(batch);
        }
    });

    let mut stats = mobicat_tui::run(
        store,
        TuiOptions {
            title: "mobicat  [synthetic]".into(),
            frame_interval: Duration::from_millis(args.frame_ms),
            run_for: args.seconds.map(Duration::from_secs),
            exercise: args.exercise,
            show_perf: true,
            ..Default::default()
        },
    )?;
    runtime.shutdown_background();
    stats.ingested -= prefilled;

    let report = format!(
        "rate {}/s  buffer {}  frame interval {}ms  exercise {}  prefill {}\n{stats}\n",
        args.rate, args.buffer, args.frame_ms, args.exercise, args.prefill
    );
    match args.stats_out {
        Some(p) => std::fs::write(&p, report).with_context(|| format!("write {}", p.display()))?,
        None => print!("{report}"),
    }
    Ok(())
}
