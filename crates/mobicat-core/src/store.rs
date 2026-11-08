use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::entry::{Entry, Level, Platform, Source};
use crate::query::Query;

/// Sequence number of an entry. Monotonic for the life of the store, so it
/// stays valid as a reference even after older entries are evicted.
pub type Seq = u64;

/// Which filtered list to read: everything, one platform, or one device.
/// Each is kept up to date, so switching is instant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum View {
    #[default]
    All,
    Only(Platform),
    /// A device by its index in [`Store::devices`].
    Device(u16),
}

impl View {
    fn index(self) -> usize {
        match self {
            View::All => 0,
            View::Only(Platform::Android) => 1,
            View::Only(Platform::Ios) => 2,
            View::Device(_) => unreachable!("device views have their own index"),
        }
    }
}

/// Device that mobicat's own notes (warnings at startup) are filed under.
/// It never becomes a device of its own.
pub const SELF_DEVICE: &str = "mobicat";

/// A device seen by the store: registered up front, or first seen in an entry.
#[derive(Debug)]
pub struct Device {
    /// Android serial or simulator UDID (`Entry::device`).
    pub key: Arc<str>,
    pub platform: Platform,
    /// Shown in the UI, e.g. "Pixel 9". Defaults to the key.
    pub name: String,
    pub connected: bool,
    index: VecDeque<Seq>,
    /// Crash entries in the buffer, ascending.
    crash_seqs: VecDeque<Seq>,
}

impl Device {
    /// Crash entries newer than `seq`.
    fn crashes_after(&self, seq: Option<Seq>) -> usize {
        match seq {
            None => self.crash_seqs.len(),
            Some(s) => self.crash_seqs.len() - self.crash_seqs.partition_point(|&c| c <= s),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Filter {
    pub min_level: Level,
    pub query: Option<Arc<Query>>,
    /// The query only highlights matches instead of hiding the rest.
    pub highlight: bool,
    /// Hide entries that are not from the app's own code.
    pub app_only: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            min_level: Level::Verbose,
            query: None,
            highlight: false,
            app_only: true,
        }
    }
}

impl Filter {
    pub fn matches(&self, e: &Entry, now_ns: i64) -> bool {
        if e.source == Source::System {
            return true; // restarts and disconnects stay visible
        }
        if e.level < self.min_level || (self.app_only && !e.app_only) {
            return false;
        }
        match &self.query {
            Some(q) if !self.highlight => q.matches(e, now_ns),
            _ => true,
        }
    }

    /// Whether `e` should be highlighted (highlight mode only).
    pub fn highlights(&self, e: &Entry, now_ns: i64) -> bool {
        self.highlight
            && e.source != Source::System
            && self
                .query
                .as_ref()
                .is_some_and(|q| !q.is_empty() && q.matches(e, now_ns))
    }
}

pub fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Ring buffer of entries plus, per view, the sequence numbers that pass the filter.
///
/// Ingest never stops: when full, the oldest entry is evicted.
pub struct Store {
    buf: VecDeque<Entry>,
    cap: usize,
    /// Seq of `buf[0]`.
    first_seq: Seq,
    /// Ascending seqs passing `filter`: all, Android, iOS.
    index: [VecDeque<Seq>; 3],
    devices: Vec<Device>,
    /// Returned for an unknown device view.
    empty: VecDeque<Seq>,
    filter: Filter,
    generation: u64,
    total_in: u64,
    /// Entries received per platform: [Android, iOS].
    per_platform: [u64; 2],
    /// App-only log entries received per platform.
    app_lines: [u64; 2],
    /// Crash entries currently in the buffer.
    crashes: usize,
}

impl Store {
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "store capacity must be positive");
        Self {
            buf: VecDeque::with_capacity(cap.min(1 << 16)),
            cap,
            first_seq: 0,
            index: Default::default(),
            devices: Vec::new(),
            empty: VecDeque::new(),
            filter: Filter::default(),
            generation: 0,
            total_in: 0,
            per_platform: [0; 2],
            app_lines: [0; 2],
            crashes: 0,
        }
    }

    fn evict(&mut self) {
        if let Some(old) = self.buf.pop_front() {
            if old.crash {
                self.crashes -= 1;
            }
            self.first_seq += 1;
            let first = self.first_seq;
            let drop_old = |ix: &mut VecDeque<Seq>| {
                while ix.front().is_some_and(|&s| s < first) {
                    ix.pop_front();
                }
            };
            self.index.iter_mut().for_each(drop_old);
            for d in &mut self.devices {
                drop_old(&mut d.index);
                drop_old(&mut d.crash_seqs);
            }
        }
    }

    /// Index of the device `key`, registering it on first sight.
    fn device_id(&mut self, key: &Arc<str>, platform: Platform) -> Option<usize> {
        if &**key == SELF_DEVICE {
            return None;
        }
        if let Some(i) = self
            .devices
            .iter()
            .position(|d| Arc::ptr_eq(&d.key, key) || d.key == *key)
        {
            return Some(i);
        }
        self.devices.push(Device {
            key: key.clone(),
            platform,
            name: key.to_string(),
            connected: true,
            index: VecDeque::new(),
            crash_seqs: VecDeque::new(),
        });
        Some(self.devices.len() - 1)
    }

    fn index_entry(&mut self, seq: Seq, e: &Entry, dev: Option<usize>, now: i64) {
        if self.filter.matches(e, now) {
            self.index[0].push_back(seq);
            self.index[View::Only(e.platform).index()].push_back(seq);
            if let Some(d) = dev {
                self.devices[d].index.push_back(seq);
            }
        }
    }

    fn ix(&self, v: View) -> &VecDeque<Seq> {
        match v {
            View::Device(d) => self
                .devices
                .get(d as usize)
                .map_or(&self.empty, |d| &d.index),
            _ => &self.index[v.index()],
        }
    }

    /// Register a device before its first entry, so it can be shown and named.
    /// Updates the name of a known device.
    pub fn add_device(&mut self, key: &str, platform: Platform, name: &str) {
        let key: Arc<str> = Arc::from(key);
        if let Some(i) = self.device_id(&key, platform) {
            self.devices[i].name = name.to_owned();
            self.devices[i].connected = true;
        }
        self.generation += 1;
    }

    /// Mark a device connected or gone. Unknown keys are ignored.
    pub fn set_connected(&mut self, key: &str, connected: bool) {
        if let Some(d) = self.devices.iter_mut().find(|d| &*d.key == key)
            && d.connected != connected
        {
            d.connected = connected;
            self.generation += 1;
        }
    }

    /// Devices in the order they were first seen.
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Crash entries in `v` (ignoring the filter) with a seq above `seen`.
    pub fn crashes_after(&self, v: View, seen: Option<Seq>) -> usize {
        self.devices
            .iter()
            .enumerate()
            .filter(|(i, d)| match v {
                View::All => true,
                View::Only(p) => d.platform == p,
                View::Device(x) => *i == x as usize,
            })
            .map(|(_, d)| d.crashes_after(seen))
            .sum()
    }

    /// Seq of the newest entry in the buffer.
    pub fn last_seq(&self) -> Option<Seq> {
        (!self.buf.is_empty()).then(|| self.first_seq + self.buf.len() as Seq - 1)
    }

    pub fn push_batch(&mut self, entries: impl IntoIterator<Item = Entry>) {
        let now = now_ns();
        for e in entries {
            if self.buf.len() == self.cap {
                self.evict();
            }
            let seq = self.first_seq + self.buf.len() as Seq;
            let dev = self.device_id(&e.device, e.platform);
            self.index_entry(seq, &e, dev, now);
            if e.crash
                && let Some(d) = dev
            {
                self.devices[d].crash_seqs.push_back(seq);
            }
            let p = e.platform as usize;
            self.per_platform[p] += 1;
            if e.app_only && e.source != Source::System {
                self.app_lines[p] += 1;
            }
            if e.crash {
                self.crashes += 1;
            }
            self.buf.push_back(e);
            self.total_in += 1;
        }
        self.generation += 1;
    }

    pub fn get(&self, seq: Seq) -> Option<&Entry> {
        let i = seq.checked_sub(self.first_seq)?;
        self.buf.get(usize::try_from(i).ok()?)
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn filtered_len(&self, v: View) -> usize {
        self.ix(v).len()
    }

    /// Seq of the `i`-th entry of `v`.
    pub fn filtered_seq(&self, v: View, i: usize) -> Option<Seq> {
        self.ix(v).get(i).copied()
    }

    /// Position in `v` of the first entry with seq >= `seq`.
    pub fn filtered_position(&self, v: View, seq: Seq) -> usize {
        self.ix(v).partition_point(|&s| s < seq)
    }

    /// Seq of the newest entry of `v`.
    pub fn last_filtered(&self, v: View) -> Option<Seq> {
        self.ix(v).back().copied()
    }

    /// Nearest position in `v` strictly after (`forward`) or before `from`
    /// whose entry satisfies `pred`.
    pub fn find(
        &self,
        v: View,
        from: usize,
        forward: bool,
        mut pred: impl FnMut(&Entry) -> bool,
    ) -> Option<usize> {
        let ix = &self.ix(v);
        let mut hit = |i: usize| ix.get(i).and_then(|&s| self.get(s)).is_some_and(&mut pred);
        if forward {
            (from + 1..ix.len()).find(|&i| hit(i))
        } else {
            (0..from.min(ix.len())).rev().find(|&i| hit(i))
        }
    }

    pub fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Replace the filter and rebuild every view over the whole buffer.
    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        for ix in &mut self.index {
            ix.clear();
        }
        for d in &mut self.devices {
            d.index.clear();
        }
        let now = now_ns();
        let first = self.first_seq;
        let mut dev: Option<(Arc<str>, Option<usize>)> = None;
        for i in 0..self.buf.len() {
            let seq = first + i as Seq;
            let e = &self.buf[i];
            if self.filter.matches(e, now) {
                let p = e.platform;
                self.index[0].push_back(seq);
                self.index[View::Only(p).index()].push_back(seq);
                // Consecutive entries usually share a device.
                let d = match &dev {
                    Some((k, d)) if Arc::ptr_eq(k, &e.device) => *d,
                    _ => {
                        let d = self.devices.iter().position(|d| d.key == e.device);
                        dev = Some((e.device.clone(), d));
                        d
                    }
                };
                if let Some(d) = d {
                    self.devices[d].index.push_back(seq);
                }
            }
        }
        self.generation += 1;
    }

    /// Changes whenever the visible content may have changed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn total_in(&self) -> u64 {
        self.total_in
    }

    /// Entries received from `p` since start (including evicted and cleared ones).
    pub fn received(&self, p: Platform) -> u64 {
        self.per_platform[p as usize]
    }

    /// App-only log entries received from `p` since start.
    pub fn received_app(&self, p: Platform) -> u64 {
        self.app_lines[p as usize]
    }

    /// Crash entries in the buffer.
    pub fn crashes(&self) -> usize {
        self.crashes
    }

    /// Drop every buffered entry. Sequence numbers keep increasing.
    pub fn clear(&mut self) {
        self.first_seq += self.buf.len() as Seq;
        self.buf.clear();
        for ix in &mut self.index {
            ix.clear();
        }
        for d in &mut self.devices {
            d.index.clear();
            d.crash_seqs.clear();
        }
        self.crashes = 0;
        self.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: Level, msg: &str) -> Entry {
        let mut e = Entry::system(Platform::Android, Arc::from("d"), None, 0, level, msg);
        e.tag = Arc::from("Tag");
        e.source = Source::Log;
        e
    }

    fn seqs(s: &Store, v: View) -> Vec<Seq> {
        (0..s.filtered_len(v))
            .filter_map(|i| s.filtered_seq(v, i))
            .collect()
    }

    fn with_level(min_level: Level) -> Filter {
        Filter {
            min_level,
            ..Filter::default()
        }
    }

    #[test]
    fn evicts_oldest_and_keeps_seq_stable() {
        let mut s = Store::new(3);
        s.push_batch((0..5).map(|i| entry(Level::Info, &i.to_string())));
        assert_eq!(s.len(), 3);
        assert_eq!(s.total_in(), 5);
        assert!(s.get(1).is_none());
        assert_eq!(&*s.get(2).unwrap().message, "2");
        assert_eq!(&*s.get(4).unwrap().message, "4");
        assert_eq!(seqs(&s, View::All), vec![2, 3, 4]);
        assert_eq!(seqs(&s, View::Only(Platform::Android)), vec![2, 3, 4]);
    }

    #[test]
    fn filter_rebuild_and_incremental_agree() {
        let levels = [Level::Debug, Level::Error, Level::Info, Level::Error];
        let mut a = Store::new(10);
        a.set_filter(with_level(Level::Error));
        a.push_batch(levels.iter().map(|&l| entry(l, "x")));
        let mut b = Store::new(10);
        b.push_batch(levels.iter().map(|&l| entry(l, "x")));
        b.set_filter(with_level(Level::Error));
        assert_eq!(seqs(&a, View::All), vec![1, 3]);
        assert_eq!(seqs(&a, View::All), seqs(&b, View::All));
    }

    #[test]
    fn views_per_platform() {
        let mut s = Store::new(10);
        let mut i = entry(Level::Info, "ios");
        i.platform = Platform::Ios;
        s.push_batch([entry(Level::Info, "a"), i, entry(Level::Info, "b")]);
        assert_eq!(seqs(&s, View::All), vec![0, 1, 2]);
        assert_eq!(seqs(&s, View::Only(Platform::Android)), vec![0, 2]);
        assert_eq!(seqs(&s, View::Only(Platform::Ios)), vec![1]);
        assert_eq!(s.received(Platform::Ios), 1);
    }

    #[test]
    fn views_per_device_and_crashes_after() {
        let mut s = Store::new(10);
        s.add_device("p9", Platform::Android, "Pixel 9");
        let on = |dev: &str, p: Platform, crash: bool| {
            let mut e = entry(Level::Info, "x");
            e.device = Arc::from(dev);
            e.platform = p;
            e.crash = crash;
            e
        };
        s.push_batch([
            on("p9", Platform::Android, false),
            on("p7", Platform::Android, true),
            on("sim", Platform::Ios, true),
            on(SELF_DEVICE, Platform::Android, false),
        ]);
        let names: Vec<&str> = s.devices().iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["Pixel 9", "p7", "sim"],
            "the self device is not listed"
        );
        assert_eq!(seqs(&s, View::Device(0)), vec![0]);
        assert_eq!(seqs(&s, View::Device(1)), vec![1]);
        assert_eq!(seqs(&s, View::Device(9)), Vec::<Seq>::new());
        assert_eq!(s.crashes_after(View::All, None), 2);
        assert_eq!(s.crashes_after(View::Only(Platform::Android), None), 1);
        assert_eq!(s.crashes_after(View::Device(2), Some(1)), 1);
        assert_eq!(s.crashes_after(View::Device(2), Some(2)), 0);
        s.set_filter(with_level(Level::Error));
        assert_eq!(seqs(&s, View::Device(1)), Vec::<Seq>::new());
        s.set_filter(Filter::default());
        assert_eq!(seqs(&s, View::Device(1)), vec![1]);
        s.set_connected("p7", false);
        assert!(!s.devices()[1].connected);
        assert_eq!(s.last_seq(), Some(3));
    }

    #[test]
    fn system_entries_always_pass() {
        let mut s = Store::new(10);
        s.set_filter(with_level(Level::Fatal));
        s.push_batch([Entry::system(
            Platform::Android,
            Arc::from("d"),
            None,
            0,
            Level::Info,
            "process started",
        )]);
        assert_eq!(s.filtered_len(View::All), 1);
    }

    #[test]
    fn query_filters_or_highlights() {
        let q = Arc::new(Query::parse("needle").unwrap());
        let mut s = Store::new(10);
        s.push_batch([entry(Level::Info, "a needle"), entry(Level::Info, "hay")]);
        s.set_filter(Filter {
            query: Some(q.clone()),
            ..Filter::default()
        });
        assert_eq!(seqs(&s, View::All), vec![0]);
        let f = Filter {
            query: Some(q),
            highlight: true,
            ..Filter::default()
        };
        s.set_filter(f.clone());
        assert_eq!(seqs(&s, View::All), vec![0, 1]);
        assert!(f.highlights(s.get(0).unwrap(), 0));
        assert!(!f.highlights(s.get(1).unwrap(), 0));
    }

    #[test]
    fn find_and_crash_count() {
        let mut s = Store::new(3);
        let mut c = entry(Level::Fatal, "crash");
        c.crash = true;
        s.push_batch([entry(Level::Info, "a"), c, entry(Level::Info, "b")]);
        assert_eq!(s.crashes(), 1);
        assert_eq!(s.find(View::All, 0, true, |e| e.crash), Some(1));
        assert_eq!(s.find(View::All, 3, false, |e| e.crash), Some(1));
        assert_eq!(s.find(View::All, 1, true, |e| e.crash), None);
        s.push_batch([entry(Level::Info, "c"), entry(Level::Info, "d")]); // evicts the crash
        assert_eq!(s.crashes(), 0);
    }

    #[test]
    fn clear_keeps_sequence_numbers_increasing() {
        let mut s = Store::new(10);
        s.push_batch([entry(Level::Info, "a"), entry(Level::Info, "b")]);
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.filtered_len(View::All), 0);
        s.push_batch([entry(Level::Info, "c")]);
        assert_eq!(s.filtered_seq(View::All, 0), Some(2));
        assert_eq!(&*s.get(2).unwrap().message, "c");
        assert_eq!(s.received(Platform::Android), 3);
    }

    #[test]
    fn app_only_hides_framework_lines() {
        let mut s = Store::new(10);
        let mut fw = entry(Level::Info, "framework");
        fw.app_only = false;
        s.push_batch([entry(Level::Info, "app"), fw]);
        assert_eq!(seqs(&s, View::All), vec![0]);
        assert_eq!(s.received_app(Platform::Android), 1);
    }
}

#[cfg(test)]
mod perf {
    use super::*;

    /// `cargo test --release -p mobicat-core -- --ignored --nocapture refilter_cost`
    #[test]
    #[ignore]
    fn refilter_cost() {
        let tags = [
            "OkHttp",
            "MainActivity",
            "Chatty",
            "ReactNativeJS",
            "network",
        ];
        let mut s = Store::new(100_000);
        s.push_batch((0..100_000).map(|i| {
            let mut e = Entry::system(
                Platform::Android,
                Arc::from("d"),
                None,
                0,
                Level::Info,
                format!("request {i} took {} ms timeout retry", i % 997),
            );
            e.source = Source::Log;
            e.tag = Arc::from(tags[i % tags.len()]);
            e
        }));
        for q in [
            "timeout",
            "tag:OkHttp | message~:time(out|d) -tag:Chatty level:info",
            "tag~:^(Ok|Main) -retry",
        ] {
            let f = Filter {
                query: Some(Arc::new(Query::parse(q).unwrap())),
                ..Filter::default()
            };
            let t = std::time::Instant::now();
            s.set_filter(f);
            println!(
                "{:>8.2} ms  {:>6} shown  {q}",
                t.elapsed().as_secs_f64() * 1000.0,
                s.filtered_len(View::All)
            );
        }
    }
}
