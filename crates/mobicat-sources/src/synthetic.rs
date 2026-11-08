//! Synthetic log source: emits plausible Android/iOS entries at a fixed rate.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mobicat_core::{Entry, Level, Platform, Source};
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct SynthConfig {
    /// Total lines per second across both platforms.
    pub rate: u32,
    /// How often a batch is emitted.
    pub tick: Duration,
}

const ANDROID_TAGS: &[&str] = &[
    "MainActivity",
    "OkHttp",
    "ViewRootImpl",
    "Choreographer",
    "ReactNativeJS",
    "RecyclerView",
    "WorkManager",
    "FirebaseMessaging",
    "AndroidRuntime",
    "nativeloader",
];

const IOS_TAGS: &[&str] = &[
    "network",
    "ui",
    "persistence",
    "auth",
    "javascript",
    "SwiftUI",
    "CFNetwork",
    "analytics",
];

const WORDS: &[&str] = &[
    "request",
    "response",
    "decoded",
    "items",
    "cache",
    "miss",
    "hit",
    "user",
    "session",
    "token",
    "refresh",
    "layout",
    "frame",
    "dropped",
    "timeout",
    "retry",
    "GET",
    "POST",
    "/api/items",
    "/api/user",
    "200",
    "404",
    "500",
    "ms",
    "bytes",
    "scroll",
    "tap",
    "navigation",
    "push",
    "pop",
    "sync",
    "完了",
    "失敗",
    "読み込み",
];

/// Small deterministic PRNG (xorshift64*), so runs are reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

pub struct Generator {
    rng: Rng,
    device: Arc<str>,
    android_tags: Vec<Arc<str>>,
    ios_tags: Vec<Arc<str>>,
}

impl Generator {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng(seed | 1),
            device: Arc::from("synthetic"),
            android_tags: ANDROID_TAGS.iter().map(|&t| Arc::from(t)).collect(),
            ios_tags: IOS_TAGS.iter().map(|&t| Arc::from(t)).collect(),
        }
    }

    pub fn entry(&mut self, time_ns: i64) -> Entry {
        let platform = if self.rng.below(2) == 0 {
            Platform::Android
        } else {
            Platform::Ios
        };
        let tags = match platform {
            Platform::Android => &self.android_tags,
            Platform::Ios => &self.ios_tags,
        };
        let tag = tags[self.rng.below(tags.len())].clone();
        // Roughly: 25% V, 30% D, 30% I, 9% W, 5% E, 1% F.
        let level = match self.rng.below(100) {
            0..25 => Level::Verbose,
            25..55 => Level::Debug,
            55..85 => Level::Info,
            85..94 => Level::Warn,
            94..99 => Level::Error,
            _ => Level::Fatal,
        };
        let n_words = 3 + self.rng.below(25);
        let mut message = String::with_capacity(n_words * 8);
        for i in 0..n_words {
            if i > 0 {
                message.push(' ');
            }
            message.push_str(WORDS[self.rng.below(WORDS.len())]);
        }
        Entry {
            platform,
            device: self.device.clone(),
            app: None,
            time_ns,
            pid: 3000 + self.rng.below(3) as u32,
            tid: 3000 + self.rng.below(20) as u64,
            level,
            tag,
            subsystem: None,
            category: None,
            library: None,
            source: Source::Log,
            app_only: true,
            crash: false,
            framework: mobicat_core::entry::Framework::Native,
            message: message.into_boxed_str(),
        }
    }
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Emit batches at `cfg.rate` lines/sec until the receiver is dropped.
pub async fn run(cfg: SynthConfig, tx: mpsc::Sender<Vec<Entry>>) {
    let mut generator = Generator::new(0x5EED);
    let mut interval = tokio::time::interval(cfg.tick);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let per_tick = f64::from(cfg.rate) * cfg.tick.as_secs_f64();
    let mut carry = 0.0_f64;
    loop {
        interval.tick().await;
        carry += per_tick;
        let n = carry as usize;
        carry -= n as f64;
        let t = now_ns();
        let batch: Vec<Entry> = (0..n).map(|_| generator.entry(t)).collect();
        if tx.send(batch).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_is_deterministic() {
        let mut a = Generator::new(42);
        let mut b = Generator::new(42);
        for _ in 0..100 {
            let (x, y) = (a.entry(0), b.entry(0));
            assert_eq!(x.message, y.message);
            assert_eq!(x.level, y.level);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn emits_at_requested_rate() {
        let (tx, mut rx) = mpsc::channel(1024);
        let cfg = SynthConfig {
            rate: 2000,
            tick: Duration::from_millis(10),
        };
        let handle = tokio::spawn(run(cfg, tx));
        let mut total = 0;
        // 100 ticks of 10 ms = 1 s of virtual time.
        for _ in 0..100 {
            total += rx.recv().await.unwrap().len();
        }
        handle.abort();
        assert!((1950..=2050).contains(&total), "got {total}");
    }
}
