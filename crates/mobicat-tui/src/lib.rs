//! Terminal UI: one log list for the chosen target (All, an OS, a device),
//! with follow/pause, multi-line blocks, queries, search, crash navigation,
//! a detail pane, copy and save.

mod app_pick;
mod clipboard;
mod complete;
mod detail;
mod keys;
mod list;
mod palette;
mod picker;
mod stats;
mod targets;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Result;
use mobicat_core::{Clock, Filter, Platform, Query, QueryError, Seq, Store, View};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{DefaultTerminal, Frame};

pub use app_pick::{AppItem, pick_app};
pub use stats::Stats;

use crate::detail::Detail;
pub(crate) use crate::keys::handle_key;
use crate::list::Viewport;
use crate::palette::Palette;
use crate::picker::Picker;
use crate::targets::TargetPanel;

pub type SharedStore = Arc<Mutex<Store>>;

#[derive(Clone, Debug, Default)]
pub struct TuiOptions {
    /// Shown in the header after "mobicat ·", usually the app ID.
    pub title: String,
    /// Minimum interval between frames.
    pub frame_interval: Duration,
    /// Quit automatically after this long (for benchmarks).
    pub run_for: Option<Duration>,
    /// Simulate user activity (scroll, pause, filter changes) for benchmarks.
    pub exercise: bool,
    /// Show ingest rate and frame times in the status bar.
    pub show_perf: bool,
    /// Saved filters for keys 1–9: (name, query).
    pub profiles: Vec<(String, String)>,
    /// Hint about `--launch` when an iOS app shows no logs of its own.
    pub ios_launch_hint: bool,
    /// Initial query text (already applied to the store's filter).
    pub query: String,
    /// Start wrapped.
    pub wrap: bool,
    /// Devices can come and go (not a file): announce connects and disconnects.
    pub live: bool,
    /// Where `/` and `f` history is kept between runs.
    pub history_path: Option<PathBuf>,
    /// Offer "Change app…" (the caller restarts with the app picker).
    pub can_change_app: bool,
}

/// Detail pane splits the screen only when the body has at least this many rows.
const SPLIT_MIN_ROWS: u16 = 24;
/// How long a toast stays.
const MESSAGE_FOR: Duration = Duration::from_secs(4);
/// Second Ctrl-C / Ctrl-D within this quits.
const QUIT_WINDOW: Duration = Duration::from_secs(2);
/// Queries kept in the history file.
const HISTORY_MAX: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputKind {
    Query,
    Search,
}

/// The `/` or `f` input box.
struct InputState {
    kind: InputKind,
    text: String,
    error: Option<QueryError>,
    /// Restored by Esc.
    before: Filter,
    before_text: String,
    /// Completion candidates for the token starting at `.0`.
    suggest: (usize, Vec<String>),
    /// Tab cycling: the text before the first Tab, and the candidate shown.
    cycling: Option<(String, usize)>,
    /// Browsing history with ↑ ↓: the position, and the text typed before.
    hist: Option<(usize, String)>,
}

enum Mode {
    Normal,
    Input(InputState),
    /// Tag picker (`t`).
    Picker(Picker),
    /// Target panel (`d`).
    Targets(TargetPanel),
    /// `:` actions.
    Palette(Palette),
    /// `c` asks before clearing.
    ConfirmClear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Info,
    Ok,
    Warn,
    Crash,
}

struct Toast {
    text: String,
    tone: Tone,
    at: Instant,
}

/// Per-device bookkeeping for badges and toasts.
#[derive(Clone, Copy, Default)]
struct DeviceMark {
    /// Newest seq seen while the device was on screen.
    seen: Option<Seq>,
    /// Newest seq already announced by a crash toast.
    announced: Option<Seq>,
    connected: bool,
}

struct App {
    title: String,
    store: SharedStore,
    clock: Clock,
    /// What the list shows.
    target: View,
    /// One viewport per target, so switching back keeps the position.
    viewports: HashMap<View, Viewport>,
    /// Wrap and time column, shared by all targets.
    layout: list::Layout,
    marks: Vec<DeviceMark>,
    detail: bool,
    help: bool,
    mode: Mode,
    /// Applied query, as typed.
    query_text: String,
    search: Option<Arc<Query>>,
    /// First key of a `]c` / `[c` chord.
    pending: Option<char>,
    /// Visual selection anchor.
    visual: Option<Seq>,
    toast: Option<Toast>,
    /// First Ctrl-C / Ctrl-D (or q with nothing to close): when, and which key.
    quit_armed: Option<(Instant, &'static str)>,
    profiles: Vec<(String, String)>,
    /// Query and search history, oldest first.
    history: Vec<String>,
    history_path: Option<PathBuf>,
    can_change_app: bool,
    ios_launch_hint: bool,
    live: bool,
    started: Instant,
    show_perf: bool,
    stats: Stats,
    rate: RateMeter,
    quit: bool,
}

impl App {
    fn new(store: SharedStore, opts: &TuiOptions, clock: Clock) -> Self {
        let layout = list::Layout {
            wrap: opts.wrap,
            ..Default::default()
        };
        let marks = {
            let s = store.lock().expect("store lock");
            let last = s.last_seq();
            s.devices()
                .iter()
                .map(|d| DeviceMark {
                    seen: None,
                    announced: last,
                    connected: d.connected,
                })
                .collect()
        };
        Self {
            title: opts.title.clone(),
            store,
            clock,
            target: View::All,
            viewports: HashMap::new(),
            layout,
            marks,
            detail: false,
            help: false,
            mode: Mode::Normal,
            query_text: opts.query.clone(),
            search: None,
            pending: None,
            visual: None,
            toast: None,
            quit_armed: None,
            profiles: opts.profiles.clone(),
            history: opts
                .history_path
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|t| {
                    t.lines()
                        .filter(|l| !l.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            history_path: opts.history_path.clone(),
            can_change_app: opts.can_change_app,
            ios_launch_hint: opts.ios_launch_hint,
            live: opts.live,
            started: Instant::now(),
            show_perf: opts.show_perf,
            stats: Stats::default(),
            rate: RateMeter::default(),
            quit: false,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Store> {
        self.store.lock().expect("store lock")
    }

    /// The viewport of the current target.
    fn vp(&mut self) -> &mut Viewport {
        let (t, layout) = (self.target, self.layout);
        let vp = self.viewports.entry(t).or_insert_with(|| Viewport::new(t));
        vp.layout = layout;
        vp
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.toast_as(Tone::Info, msg);
    }

    fn toast_as(&mut self, tone: Tone, msg: impl Into<String>) {
        self.toast = Some(Toast {
            text: msg.into(),
            tone,
            at: Instant::now(),
        });
    }

    /// Remember an applied query or search, newest last, without repeats.
    fn remember(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.history.retain(|h| h != text);
        self.history.push(text.to_owned());
        let extra = self.history.len().saturating_sub(HISTORY_MAX);
        self.history.drain(..extra);
        if let Some(p) = &self.history_path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, self.history.join("\n") + "\n");
        }
    }

    fn switch_to(&mut self, v: View) {
        if v == self.target {
            return;
        }
        self.visual = None;
        if let Some(vp) = self.viewports.get_mut(&self.target) {
            vp.marked = None;
        }
        self.target = v;
    }

    /// New crashes on devices outside the current target, within `v`.
    fn unseen(&self, store: &Store, v: View) -> usize {
        // All would repeat every other badge.
        if v == self.target || v == View::All {
            return 0;
        }
        (0..store.devices().len())
            .filter(|&d| targets::covers(store, v, d) && !targets::covers(store, self.target, d))
            .map(|d| {
                store.crashes_after(
                    View::Device(d as u16),
                    self.marks.get(d).and_then(|m| m.seen),
                )
            })
            .sum()
    }

    /// Update marks after new entries: what is on screen counts as seen, crashes
    /// elsewhere are announced once, devices that come and go are announced.
    fn observe(&mut self, store: &Store) {
        let last = store.last_seq();
        for (d, dev) in store.devices().iter().enumerate() {
            if d >= self.marks.len() {
                self.marks.push(DeviceMark {
                    seen: None,
                    announced: None,
                    connected: dev.connected,
                });
                if self.live {
                    self.toast_as(Tone::Ok, format!("+ {} connected", dev.name));
                }
            }
            let on_screen = targets::covers(store, self.target, d);
            let m = self.marks[d];
            if m.connected != dev.connected {
                self.marks[d].connected = dev.connected;
                if self.live {
                    if dev.connected {
                        self.toast_as(Tone::Ok, format!("+ {} reconnected", dev.name));
                    } else {
                        self.toast_as(Tone::Warn, format!("− {} disconnected", dev.name));
                    }
                }
            }
            if on_screen {
                self.marks[d].seen = last;
                self.marks[d].announced = last;
            } else if store.crashes_after(View::Device(d as u16), m.announced) > 0 {
                self.marks[d].announced = last;
                self.toast_as(
                    Tone::Crash,
                    format!("✖ new crash on {} · tab or d to view", dev.name),
                );
            }
        }
    }
}

pub fn run(store: SharedStore, opts: TuiOptions) -> Result<Stats> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, store, &opts);
    ratatui::restore();
    result
}

fn run_loop(
    terminal: &mut DefaultTerminal,
    store: SharedStore,
    opts: &TuiOptions,
) -> Result<Stats> {
    let mut app = App::new(store, opts, Clock::system());
    let start = Instant::now();
    let deadline = opts.run_for.map(|d| start + d);
    let frame = opts.frame_interval.max(Duration::from_millis(1));
    let mut next_frame = start;
    let mut last_generation = u64::MAX;
    let mut dirty = true;
    let mut exercise = opts.exercise.then(Exerciser::default);

    while !app.quit {
        let now = Instant::now();
        if deadline.is_some_and(|d| now >= d) {
            break;
        }

        // Wait for input until the next frame is due, then drain all pending input.
        let timeout = next_frame.saturating_duration_since(now);
        if event::poll(timeout)? {
            loop {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => {
                        app.stats.keys += 1;
                        handle_key(&mut app, k.code, k.modifiers);
                    }
                    Event::Resize(..) => {}
                    _ => continue,
                }
                dirty = true;
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        if let Some(ex) = exercise.as_mut() {
            dirty |= ex.step(&mut app, start.elapsed());
        }
        if app
            .toast
            .as_ref()
            .is_some_and(|t| t.at.elapsed() > MESSAGE_FOR)
        {
            app.toast = None;
            dirty = true;
        }
        if app
            .quit_armed
            .is_some_and(|(t, _)| t.elapsed() > QUIT_WINDOW)
        {
            app.quit_armed = None;
            dirty = true;
        }

        let now = Instant::now();
        if now < next_frame {
            continue;
        }
        let generation = app.lock().generation();
        if dirty || generation != last_generation {
            let t0 = Instant::now();
            terminal.draw(|f| draw(f, &mut app))?;
            app.stats.record_frame(t0.elapsed());
            last_generation = generation;
            dirty = false;
        }
        next_frame = now + frame;
    }
    let ingested = app.lock().total_in();
    app.stats.ingested = ingested;
    app.stats.elapsed = start.elapsed();
    Ok(app.stats)
}

fn draw(f: &mut Frame, app: &mut App) {
    let [header, tabs, body, status, hint] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let store = app.store.clone();
    let store = store.lock().expect("store lock");
    app.observe(&store);

    f.render_widget(header_line(app, header.width), header);
    let unseen = |v: View| app.unseen(&store, v);
    f.render_widget(
        Paragraph::new(targets::tab_line(&store, app.target, &unseen)),
        tabs,
    );

    // The input box sits at the bottom of the list, above the status line.
    let (body, input_area) = match &app.mode {
        Mode::Input(st) => {
            let h = (3 + u16::from(!st.suggest.1.is_empty())).min(body.height);
            let [b, i] = Layout::vertical([Constraint::Min(0), Constraint::Length(h)]).areas(body);
            (b, Some(i))
        }
        _ => (body, None),
    };
    let (list_area, detail_area) = if !app.detail {
        (body, None)
    } else if body.height >= SPLIT_MIN_ROWS {
        let [l, d] =
            Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(body);
        (l, Some(d))
    } else {
        (Rect::new(body.x, body.y, body.width, 0), Some(body))
    };

    let clock = app.clock.clone();
    let view = app.vp().build(&store, list_area, &clock);
    f.render_widget(view, list_area);
    let cursor = app.vp().cursor();

    if let Some(area) = detail_area {
        match cursor.and_then(|s| store.get(s)) {
            Some(e) => f.render_widget(
                Detail {
                    entry: e,
                    clock: &app.clock,
                },
                area,
            ),
            None => f.render_widget(
                Paragraph::new(" no entry selected").block(Block::new().borders(Borders::TOP)),
                area,
            ),
        }
    }

    let status_line = status_line(app, &store, status.width);
    let shows_ios = (0..store.devices().len()).any(|d| {
        store.devices()[d].platform == Platform::Ios && targets::covers(&store, app.target, d)
    });
    let ios_quiet = app.ios_launch_hint
        && shows_ios
        && app.started.elapsed() > Duration::from_secs(10)
        && store.received(Platform::Ios) > 0
        && store.received_app(Platform::Ios) == 0;
    f.render_widget(Paragraph::new(status_line), status);
    f.render_widget(hint_line(app, ios_quiet), hint);
    if let (Some(area), Mode::Input(st)) = (input_area, &app.mode) {
        let n = store.filtered_len(app.target);
        draw_input(f, st, n, area);
    }

    match &app.mode {
        Mode::Picker(p) => p.draw(f),
        Mode::Palette(p) => p.draw(f),
        Mode::Targets(p) => p.draw(f, &store, app.target, &|v| app.unseen(&store, v)),
        _ if app.help => draw_help(f, &app.profiles),
        _ => {}
    }
    if let Some(first) = app.pending {
        draw_which_key(f, first, status);
    }
}

fn header_line(app: &App, width: u16) -> Paragraph<'static> {
    let mut left = vec![
        Span::raw(" "),
        Span::styled("mobicat", theme::accent_bold()),
    ];
    if !app.title.is_empty() {
        left.push(Span::styled(" · ", theme::dim()));
        left.push(Span::raw(app.title.clone()));
    }
    let right = Span::styled("tab next · d targets · ? keys ", theme::dim());
    let used: usize = left.iter().map(Span::width).sum::<usize>() + right.width();
    left.push(Span::raw(
        " ".repeat((width as usize).saturating_sub(used).max(1)),
    ));
    left.push(right);
    Paragraph::new(Line::from(left))
}

/// Keys for the current state: what can be done now, and how to go back.
fn hints(app: &App) -> &'static str {
    if let Some(first) = app.pending {
        return if first == ']' {
            " c next crash · e error · w warning · esc cancel"
        } else {
            " c previous crash · e error · w warning · esc cancel"
        };
    }
    match &app.mode {
        Mode::Targets(_) => " ↑↓ choose · ← → fold · enter switch · type to narrow · esc close",
        Mode::Picker(_) => " ↑↓ choose · enter include · - exclude · type to narrow · esc close",
        Mode::Palette(_) => " ↑↓ choose · enter run · type to narrow · esc close",
        _ if app.help => " any key closes",
        _ if app.visual.is_some() => " ↑↓ extend · y copy · esc cancel",
        _ if app.detail => " esc close · ↑↓ move · y copy · o open/close block",
        _ if app
            .viewports
            .get(&app.target)
            .is_some_and(|v| v.cursor().is_some()) =>
        {
            " esc back to live · ↑↓ move · enter detail · y copy · V select · ? keys"
        }
        _ => " / filter · f search · tab next · d targets · ]c next crash · : actions · ? keys",
    }
}

fn hint_line(app: &App, ios_quiet: bool) -> Paragraph<'static> {
    if let Some((_, key)) = app.quit_armed {
        return Paragraph::new(format!(" Press {key} again to exit")).style(theme::warn());
    }
    if let Mode::Input(st) = &app.mode {
        return match &st.error {
            Some(e) => Paragraph::new(format!(" {e}")).style(Style::new().fg(Color::Red)),
            None => Paragraph::new(" tab complete · ↑↓ history · enter apply · esc cancel")
                .style(theme::dim()),
        };
    }
    if matches!(app.mode, Mode::ConfirmClear) {
        return Paragraph::new(Line::from(vec![
            Span::styled(" Clear the screen and the buffer? ", theme::warn()),
            Span::styled("y", theme::accent_bold()),
            Span::styled(" clear · ", theme::dim()),
            Span::styled("n", theme::accent_bold()),
            Span::styled(" / ", theme::dim()),
            Span::styled("esc", theme::accent_bold()),
            Span::styled(" keep", theme::dim()),
        ]));
    }
    if app.pending.is_none()
        && let Some(t) = &app.toast
    {
        let style = match t.tone {
            Tone::Info => theme::bold(),
            Tone::Ok => theme::ok(),
            Tone::Warn => theme::warn(),
            Tone::Crash => theme::crash(),
        };
        return Paragraph::new(format!(" {}", t.text)).style(style);
    }
    if ios_quiet && matches!(app.mode, Mode::Normal) {
        return Paragraph::new(
            " iOS: no logs from the app's own code yet. If it uses print(), restart mobicat with --launch.",
        )
        .style(theme::warn());
    }
    Paragraph::new(hints(app)).style(theme::dim())
}

/// The Claude Code-like input box: prompt, text, match count; candidates below.
fn draw_input(f: &mut Frame, st: &InputState, shown: usize, area: Rect) {
    let (prompt, title) = match st.kind {
        InputKind::Query => ("/", " filter "),
        InputKind::Search => ("f", " search "),
    };
    let border = if st.error.is_some() {
        Style::new().fg(Color::Red)
    } else {
        theme::accent()
    };
    let mut block = Block::bordered()
        .border_type(theme::BORDER)
        .border_style(border)
        .title(Span::styled(title, theme::accent_bold()));
    if st.kind == InputKind::Query {
        block = block.title_top(
            Line::styled(
                format!(" {} shown ", thousands(shown as u64)),
                theme::accent(),
            )
            .right_aligned(),
        );
    }
    let [boxed, cands] = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {prompt} "), theme::accent_bold()),
            Span::raw(st.text.clone()),
            Span::styled("▏", theme::accent()),
        ]))
        .block(block),
        boxed,
    );
    if cands.height > 0 && !st.suggest.1.is_empty() {
        let current = st.cycling.as_ref().map_or(0, |(_, i)| *i);
        let mut spans = vec![Span::raw("   ")];
        for (i, c) in st.suggest.1.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(if i == current {
                Span::styled(c.clone(), theme::accent_bold().patch(theme::SELECTED))
            } else {
                Span::styled(c.clone(), theme::dim())
            });
        }
        f.render_widget(Paragraph::new(Line::from(spans)), cands);
    }
}

/// Which-key popup for `]` / `[`, just above the status line.
fn draw_which_key(f: &mut Frame, first: char, status: Rect) {
    let dir = if first == ']' { "next" } else { "previous" };
    let rows = [("c", "crash"), ("e", "error"), ("w", "warning")];
    let w = 26.min(status.width);
    let h = rows.len() as u16 + 2;
    if status.y < h {
        return;
    }
    let rect = Rect::new(status.x + 1, status.y - h, w, h);
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!(" {k}  "), theme::accent_bold()),
                Span::raw(*v),
            ])
        })
        .collect();
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(theme::BORDER)
                .border_style(theme::accent())
                .title(Span::styled(
                    format!(" {first} {dir} "),
                    theme::accent_bold(),
                )),
        ),
        rect,
    );
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `● FOLLOW · ✖ 2 · level ≥ V · app-only · q: tag:OkHttp        7 shown`
fn status_line(app: &mut App, store: &Store, width: u16) -> Line<'static> {
    let filter = store.filter();
    let sep = || Span::styled(" · ", theme::dim());
    let target = app.target;
    let below = app.viewports.get(&target).and_then(|v| v.new_below());
    let mut left = vec![
        Span::raw(" "),
        match (app.visual.is_some(), below) {
            (true, _) => Span::styled("▍VISUAL", theme::accent_bold()),
            (false, None) => Span::styled("● FOLLOW", theme::ok()),
            (false, Some(0)) => Span::styled("‖ PAUSED", theme::warn()),
            (false, Some(n)) => Span::styled(
                format!("‖ PAUSED +{} below", thousands(n as u64)),
                theme::warn(),
            ),
        },
    ];
    let crashes = store.crashes_after(target, None);
    if crashes > 0 {
        left.push(sep());
        left.push(Span::styled(format!("✖ {crashes}"), theme::crash()));
    }
    left.push(sep());
    left.push(Span::raw(format!("level ≥ {}", filter.min_level.letter())));
    left.push(sep());
    left.push(Span::raw(if filter.app_only {
        "app-only"
    } else {
        "all lines"
    }));
    if !app.query_text.is_empty() {
        let verb = if filter.highlight { "highlight" } else { "q" };
        left.push(sep());
        left.push(Span::raw(format!("{verb}: {}", app.query_text)));
    }
    if let Some(s) = &app.search {
        left.push(sep());
        left.push(Span::raw(format!("search: {}", s.as_str())));
    }
    if app.layout.wrap {
        left.push(sep());
        left.push(Span::raw("wrap"));
    }
    let mut right = format!("{} shown ", thousands(store.filtered_len(target) as u64));
    if app.show_perf {
        let in_rate = app.rate.update(store.total_in());
        let frame = app.stats.recent();
        right = format!(
            "in {in_rate:.0}/s · buf {} · frame {:.2}ms (max {:.2}) · {right}",
            store.len(),
            frame.avg_ms,
            frame.max_ms
        );
    }
    let used: usize = left.iter().map(Span::width).sum::<usize>() + right.chars().count();
    left.push(Span::raw(
        " ".repeat((width as usize).saturating_sub(used).max(1)),
    ));
    left.push(Span::styled(right, theme::dim()));
    Line::from(left)
}

/// Help, grouped, in two columns.
type Group = (&'static str, &'static [(&'static str, &'static str)]);
const HELP_LEFT: &[Group] = &[
    (
        "Move",
        &[
            ("j k  ↓ ↑", "move (up pauses)"),
            ("PgDn PgUp", "half a page"),
            ("g  G", "oldest / follow newest"),
            ("Space", "pause / resume"),
            ("]c [c", "next / previous crash"),
            ("]e [e  ]w [w", "… error, warning"),
            ("f  n N", "search, next / previous"),
        ],
    ),
    (
        "Switch",
        &[
            ("Tab  S-Tab", "next / previous target"),
            ("d", "targets: All, OS, device"),
            (":", "all actions by name"),
        ],
    ),
    (
        "Back and quit",
        &[
            ("Esc  q", "close what is open"),
            ("^c ^c  ^d ^d", "quit (q counts once)"),
        ],
    ),
];
const HELP_RIGHT: &[Group] = &[
    (
        "Filter",
        &[
            ("/", "query: tag:x level:warn …"),
            ("x", "hide others / highlight"),
            ("t", "pick a tag or subsystem"),
            ("= !", "only / hide this tag"),
            ("+ -", "minimum level up / down"),
            ("a", "app-only on / off"),
        ],
    ),
    (
        "View",
        &[
            ("Enter  o", "detail / open a block"),
            ("^w  T", "wrap / time column"),
            ("c", "clear (asks first)"),
        ],
    ),
    (
        "Copy",
        &[
            ("y  V…y  Y", "entry / range / all shown"),
            ("^s", "save all shown to a file"),
        ],
    ),
];

fn help_column(groups: &[(String, Vec<(String, String)>)]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (i, (g, rows)) in groups.iter().enumerate() {
        if i > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(format!(" {g}"), theme::accent_bold()));
        for (k, v) in rows {
            lines.push(Line::from(vec![
                Span::styled(format!("   {k:<14}"), theme::bold()),
                Span::raw(v.clone()),
            ]));
        }
    }
    lines
}

fn owned(groups: &[Group]) -> Vec<(String, Vec<(String, String)>)> {
    groups
        .iter()
        .map(|(g, rows)| {
            (
                (*g).to_owned(),
                rows.iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            )
        })
        .collect()
}

fn draw_help(f: &mut Frame, profiles: &[(String, String)]) {
    let area = f.area();
    let left = owned(HELP_LEFT);
    let mut right = owned(HELP_RIGHT);
    if !profiles.is_empty() {
        right.push((
            "Profiles".into(),
            profiles
                .iter()
                .take(9)
                .enumerate()
                .map(|(i, (name, q))| (format!("{}", i + 1), format!("{name}: {q}")))
                .collect(),
        ));
    }
    let (l, r) = (help_column(&left), help_column(&right));
    let two = area.width >= 92;
    let col_w = 44u16;
    let (w, h) = if two {
        (col_w * 2 + 2, l.len().max(r.len()) as u16 + 2)
    } else {
        (col_w + 4, (l.len() + r.len() + 1) as u16 + 2)
    };
    let (w, h) = (w.min(area.width), h.min(area.height));
    let rect = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    let block = Block::bordered()
        .border_type(theme::BORDER)
        .border_style(theme::accent())
        .title(Span::styled(" Keys ", theme::accent_bold()))
        .title_bottom(Line::styled(" any key closes ", theme::dim()));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    if two {
        let [a, b] =
            Layout::horizontal([Constraint::Length(col_w), Constraint::Min(1)]).areas(inner);
        f.render_widget(Paragraph::new(l), a);
        f.render_widget(Paragraph::new(r), b);
    } else {
        let mut all = l;
        all.push(Line::raw(""));
        all.extend(r);
        f.render_widget(Paragraph::new(all), inner);
    }
}

/// Ingest rate over a sliding one-second window.
#[derive(Default)]
struct RateMeter {
    last: Option<(Instant, u64)>,
    rate: f64,
}

impl RateMeter {
    fn update(&mut self, total: u64) -> f64 {
        let now = Instant::now();
        match self.last {
            None => self.last = Some((now, total)),
            Some((t, n)) => {
                let dt = now.duration_since(t).as_secs_f64();
                if dt >= 1.0 {
                    self.rate = (total - n) as f64 / dt;
                    self.last = Some((now, total));
                }
            }
        }
        self.rate
    }
}

/// Scripted user activity for benchmarks: every 500 ms it scrolls, pauses,
/// resumes or changes the level filter, in a fixed cycle.
#[derive(Default)]
struct Exerciser {
    step: u32,
}

impl Exerciser {
    fn step(&mut self, app: &mut App, elapsed: Duration) -> bool {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        let due = (elapsed.as_millis() / 500) as u32;
        if due <= self.step {
            return false;
        }
        self.step = due;
        let key = match self.step % 8 {
            0 => KeyCode::PageUp,
            1 => KeyCode::Char('k'),
            2 => KeyCode::PageDown,
            3 => KeyCode::Char('G'),
            4 | 5 => KeyCode::Char('+'),
            _ => KeyCode::Char('-'),
        };
        handle_key(app, key, KeyModifiers::NONE);
        true
    }
}

#[cfg(test)]
mod tests;
