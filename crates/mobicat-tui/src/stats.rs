use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

/// Frame and filter timings collected during a run.
#[derive(Debug, Default)]
pub struct Stats {
    frames: Vec<Duration>,
    recent: VecDeque<Duration>,
    refilters: Vec<(Duration, usize)>,
    pub keys: u64,
    pub ingested: u64,
    pub elapsed: Duration,
    /// The user asked to switch to another app (from the target list or `:`).
    pub change_app: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Recent {
    pub avg_ms: f64,
    pub max_ms: f64,
}

const RECENT_WINDOW: usize = 60;

impl Stats {
    pub fn record_frame(&mut self, d: Duration) {
        self.frames.push(d);
        self.recent.push_back(d);
        if self.recent.len() > RECENT_WINDOW {
            self.recent.pop_front();
        }
    }

    pub fn record_refilter(&mut self, d: Duration, entries: usize) {
        self.refilters.push((d, entries));
    }

    /// Average and max over the last `RECENT_WINDOW` frames.
    pub fn recent(&self) -> Recent {
        if self.recent.is_empty() {
            return Recent::default();
        }
        let sum: Duration = self.recent.iter().sum();
        let max = self.recent.iter().max().copied().unwrap_or_default();
        Recent {
            avg_ms: ms(sum) / self.recent.len() as f64,
            max_ms: ms(max),
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.elapsed.as_secs_f64().max(f64::EPSILON);
        let mut sorted = self.frames.clone();
        sorted.sort();
        writeln!(f, "elapsed        {:.1}s", secs)?;
        writeln!(
            f,
            "ingested       {} lines ({:.0}/s)",
            self.ingested,
            self.ingested as f64 / secs
        )?;
        writeln!(
            f,
            "frames drawn   {} ({:.1}/s)",
            sorted.len(),
            sorted.len() as f64 / secs
        )?;
        writeln!(
            f,
            "draw time      p50 {:.3}ms  p99 {:.3}ms  max {:.3}ms",
            ms(percentile(&sorted, 0.50)),
            ms(percentile(&sorted, 0.99)),
            ms(sorted.last().copied().unwrap_or_default()),
        )?;
        if !self.refilters.is_empty() {
            let mut r: Vec<_> = self.refilters.iter().map(|(d, _)| *d).collect();
            r.sort();
            let n = self.refilters.iter().map(|(_, n)| *n).max().unwrap_or(0);
            writeln!(
                f,
                "refilter       {} times over ≤{} entries: p50 {:.3}ms  max {:.3}ms",
                r.len(),
                n,
                ms(percentile(&r, 0.50)),
                ms(r.last().copied().unwrap_or_default()),
            )?;
        }
        write!(f, "keys           {}", self.keys)
    }
}
