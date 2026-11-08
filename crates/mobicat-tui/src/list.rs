//! The log list: a cursor over filtered entries, rendered with variable row
//! heights (expanded multi-line blocks, wrapping). Only the visible rows are
//! materialized per frame.

use std::collections::HashSet;

use mobicat_core::output::{TAG_WIDTH, pad_tag};
use mobicat_core::store::now_ns;
use mobicat_core::{Clock, Entry, Level, Platform, Seq, Source, Store, View};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeMode {
    /// `HH:MM:SS.mmm`
    #[default]
    Clock,
    /// Milliseconds since the previous visible entry.
    Delta,
    Hidden,
}

impl TimeMode {
    pub fn next(self) -> Self {
        match self {
            TimeMode::Clock => TimeMode::Delta,
            TimeMode::Delta => TimeMode::Hidden,
            TimeMode::Hidden => TimeMode::Clock,
        }
    }

    fn width(self) -> usize {
        match self {
            TimeMode::Clock | TimeMode::Delta => 13,
            TimeMode::Hidden => 0,
        }
    }
}

/// How rows are laid out.
#[derive(Clone, Copy, Debug, Default)]
pub struct Layout {
    pub wrap: bool,
    pub time: TimeMode,
}

impl Layout {
    /// Columns before the message: time, badge, level, tag.
    fn prefix_width(self) -> usize {
        self.time.width() + 2 + 4 + TAG_WIDTH + 1
    }
}

/// Cursor and scroll state.
///
/// `cursor == None` means following the tail. Positions are kept as sequence
/// numbers, so they stay put while lines arrive and old ones are evicted.
#[derive(Debug, Default)]
pub struct Viewport {
    /// Which filtered list this viewport shows.
    pub view: View,
    cursor: Option<Seq>,
    top: Option<Seq>,
    expanded: HashSet<Seq>,
    /// Visual selection, inclusive (M4).
    pub marked: Option<(Seq, Seq)>,
    pub layout: Layout,
    height: usize,
    new_below: usize,
}

impl Viewport {
    pub fn cursor(&self) -> Option<Seq> {
        self.cursor
    }

    /// `None` while following; `Some(n)` = paused with `n` matching entries below the view.
    pub fn new_below(&self) -> Option<usize> {
        self.cursor.map(|_| self.new_below)
    }

    fn page(&self) -> usize {
        (self.height / 2).max(1)
    }

    fn cursor_pos(&self, store: &Store) -> Option<usize> {
        let len = store.filtered_len(self.view);
        (len > 0).then(|| {
            self.cursor
                .map_or(len, |c| store.filtered_position(self.view, c))
                .min(len)
        })
    }

    /// Put the cursor on the `pos`-th entry of this view (pausing).
    pub fn jump_to(&mut self, store: &Store, pos: usize) {
        self.set_pos(store, pos);
    }

    /// Position of the cursor in this view, or the end while following.
    pub fn position(&self, store: &Store) -> usize {
        self.cursor_pos(store).unwrap_or(0)
    }

    pub fn new(view: View) -> Self {
        Self {
            view,
            ..Default::default()
        }
    }

    fn set_pos(&mut self, store: &Store, pos: usize) {
        self.cursor = store.filtered_seq(self.view, pos);
    }

    /// Move up. From the tail, the first step selects the newest entry.
    pub fn up(&mut self, store: &Store, n: usize) {
        if let Some(pos) = self.cursor_pos(store) {
            let pos = pos.saturating_sub(n);
            self.set_pos(store, pos);
        }
    }

    /// Move down. Moving past the newest entry resumes following.
    pub fn down(&mut self, store: &Store, n: usize) {
        let Some(pos) = self.cursor_pos(store) else {
            return;
        };
        if self.cursor.is_none() {
            return;
        }
        let last = store.filtered_len(self.view) - 1;
        if pos >= last {
            self.follow();
        } else {
            self.set_pos(store, (pos + n).min(last));
        }
    }

    pub fn page_up(&mut self, store: &Store) {
        self.up(store, self.page());
    }

    pub fn page_down(&mut self, store: &Store) {
        let n = self.page();
        self.down(store, n);
    }

    pub fn first(&mut self, store: &Store) {
        self.set_pos(store, 0);
        self.top = self.cursor;
    }

    pub fn follow(&mut self) {
        self.cursor = None;
        self.top = None;
    }

    pub fn toggle_pause(&mut self, store: &Store) {
        if self.cursor.is_some() {
            self.follow();
        } else {
            self.cursor = store.last_filtered(self.view);
        }
    }

    /// Open or close the multi-line block under the cursor.
    pub fn toggle_expand(&mut self, store: &Store) {
        let Some(seq) = self.cursor.or_else(|| store.last_filtered(self.view)) else {
            return;
        };
        if !self.expanded.remove(&seq) {
            self.expanded.insert(seq);
        }
    }

    /// Reset after the store was cleared.
    pub fn reset(&mut self) {
        self.follow();
        self.expanded.clear();
    }

    /// Resolve the viewport against the store and materialize visible rows.
    pub fn build(&mut self, store: &Store, area: Rect, clock: &Clock) -> ListView {
        let view = self.view;
        let now = now_ns();
        self.height = area.height as usize;
        let height = self.height;
        let width = area.width as usize;
        let len = store.filtered_len(view);
        if len == 0 || height == 0 {
            self.new_below = 0;
            return ListView { lines: Vec::new() };
        }
        let rows_of = |vp: &Viewport, pos: usize| -> Vec<Line<'static>> {
            let Some(seq) = store.filtered_seq(view, pos) else {
                return Vec::new();
            };
            let Some(e) = store.get(seq) else {
                return Vec::new();
            };
            let prev = pos
                .checked_sub(1)
                .and_then(|p| store.filtered_seq(view, p))
                .and_then(|s| store.get(s));
            // Crash blocks start open; `o` flips either way.
            let expanded = vp.expanded.contains(&seq) != e.crash;
            let style = RowStyle {
                selected: vp.cursor == Some(seq)
                    || vp.marked.is_some_and(|(a, b)| a <= seq && seq <= b),
                highlight: store.filter().highlights(e, now),
            };
            let mut rows = entry_rows(
                e,
                prev,
                vp.layout,
                width.saturating_sub(1),
                expanded,
                style,
                clock,
            );
            // One-column gutter: the selection bar.
            let gutter = if style.selected {
                Span::styled(SELECTED_BAR, crate::theme::accent())
            } else {
                Span::raw(" ")
            };
            for l in &mut rows {
                l.spans.insert(0, gutter.clone());
            }
            rows
        };

        let mut lines: Vec<Line<'static>> = Vec::new();
        let last_pos;
        match self.cursor {
            None => {
                // Bottom-anchored: walk back from the newest entry.
                let mut blocks = Vec::new();
                let mut total = 0;
                let mut pos = len;
                while pos > 0 && total < height {
                    pos -= 1;
                    let r = rows_of(self, pos);
                    total += r.len();
                    blocks.push(r);
                }
                for b in blocks.into_iter().rev() {
                    lines.extend(b);
                }
                let skip = lines.len().saturating_sub(height);
                lines.drain(..skip);
                last_pos = len - 1;
                // Remember the top so pausing keeps the screen where it is.
                self.top = store.filtered_seq(view, pos);
            }
            Some(_) => {
                let cpos = self.cursor_pos(store).unwrap_or(0).min(len - 1);
                self.set_pos(store, cpos);
                let fits_above = |vp: &Viewport, mut t: usize, mut total: usize| {
                    while t > 0 {
                        let r = rows_of(vp, t - 1).len();
                        if total + r > height {
                            break;
                        }
                        total += r;
                        t -= 1;
                    }
                    t
                };
                let mut tpos = match self.top {
                    Some(t) => store.filtered_position(view, t).min(cpos),
                    // No anchor yet (paused before the first frame): start from what
                    // following would show; the step below brings the cursor into view.
                    None => fits_above(self, len - 1, rows_of(self, len - 1).len()).min(cpos),
                };
                // Keep the cursor's entry on screen.
                let mut span: usize = (tpos..=cpos).map(|p| rows_of(self, p).len()).sum();
                while span > height && tpos < cpos {
                    span -= rows_of(self, tpos).len();
                    tpos += 1;
                }
                // No blank rows at the bottom while earlier entries exist.
                let mut below = 0;
                let mut p = tpos;
                while p < len && below < height {
                    below += rows_of(self, p).len();
                    p += 1;
                }
                if below < height {
                    tpos = fits_above(self, tpos, below);
                }
                let mut pos = tpos;
                while pos < len && lines.len() < height {
                    lines.extend(rows_of(self, pos));
                    pos += 1;
                }
                lines.truncate(height);
                last_pos = pos - 1;
                self.top = store.filtered_seq(view, tpos);
            }
        }
        self.new_below = len - 1 - last_pos;
        ListView { lines }
    }
}

pub struct ListView {
    lines: Vec<Line<'static>>,
}

impl Widget for ListView {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.lines).render(area, buf);
    }
}

pub fn level_style(level: Level) -> Style {
    match level {
        Level::Verbose => Style::new().add_modifier(Modifier::DIM),
        Level::Debug => Style::new().fg(Color::Cyan),
        Level::Info => Style::new().fg(Color::Green),
        Level::Warn => Style::new().fg(Color::Yellow),
        Level::Error => Style::new().fg(Color::Red),
        Level::Fatal => Style::new()
            .fg(Color::White)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
    }
}

pub fn platform_style(p: Platform) -> Style {
    match p {
        Platform::Android => Style::new().fg(Color::Green),
        Platform::Ios => Style::new().fg(Color::Magenta),
    }
}

/// Selected rows: a bar in the gutter and a subtle background, so level and
/// crash colors stay readable.
const SELECTED: Style = crate::theme::SELECTED;
const SELECTED_BAR: &str = "▌";

fn time_text(e: &Entry, prev: Option<&Entry>, mode: TimeMode, clock: &Clock) -> String {
    match mode {
        TimeMode::Clock => clock.hms_millis(e.time_ns),
        TimeMode::Delta => match prev {
            Some(p) => {
                let ms = (e.time_ns - p.time_ns) as f64 / 1e6;
                if ms.abs() >= 100_000.0 {
                    format!("{:>+11.0}s", ms / 1000.0)
                } else {
                    format!("{ms:>+10.1}ms")
                }
            }
            None => format!("{:>12}", ""),
        },
        TimeMode::Hidden => String::new(),
    }
}

/// Split `s` into chunks of at most `width` display columns.
fn wrap_columns(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            w = 0;
        }
        cur.push(ch);
        w += cw;
    }
    out.push(cur);
    out
}

/// The rows for one entry.
#[derive(Clone, Copy, Default)]
struct RowStyle {
    selected: bool,
    highlight: bool,
}

fn entry_rows(
    e: &Entry,
    prev: Option<&Entry>,
    layout: Layout,
    width: usize,
    expanded: bool,
    style: RowStyle,
    clock: &Clock,
) -> Vec<Line<'static>> {
    let selected = style.selected;
    let mut head: Vec<Span<'static>> = Vec::new();
    if layout.time != TimeMode::Hidden {
        head.push(Span::styled(
            time_text(e, prev, layout.time, clock),
            Style::new().add_modifier(Modifier::DIM),
        ));
        head.push(Span::raw(" "));
    }
    head.push(Span::styled(
        e.platform.badge().to_string(),
        platform_style(e.platform),
    ));

    if e.source == Source::System {
        let style = match e.level {
            Level::Error | Level::Fatal => Style::new().fg(Color::Red),
            Level::Warn => Style::new().fg(Color::Yellow),
            _ => Style::new().add_modifier(Modifier::DIM),
        };
        head.push(Span::styled(format!("   ── {} ──", e.message), style));
        let line = Line::from(head);
        return vec![if selected { line.style(SELECTED) } else { line }];
    }

    let lvl = level_style(e.level);
    head.push(Span::raw(" "));
    head.push(Span::styled(format!(" {} ", e.level.letter()), lvl));
    head.push(Span::raw(" "));
    head.push(Span::raw(pad_tag(&e.tag)));
    head.push(Span::raw(" "));

    let mut msg_style = if e.level >= Level::Error {
        lvl
    } else {
        Style::new()
    };
    if style.highlight {
        msg_style = msg_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    }
    // Stack traces indent with tabs, which terminals render inconsistently.
    let message: std::borrow::Cow<str> = if e.message.contains('\t') {
        e.message.replace('\t', "    ").into()
    } else {
        e.message.as_ref().into()
    };
    let mut msg_lines = message.split('\n');
    let first = msg_lines.next().unwrap_or("");
    let extra = message.split('\n').count() - 1;
    let text_width = width.saturating_sub(layout.prefix_width()).max(10);
    let indent = " ".repeat(layout.prefix_width());

    let push_text =
        |rows: &mut Vec<Line<'static>>, text: &str, lead: Option<Vec<Span<'static>>>| {
            let chunks = if layout.wrap {
                wrap_columns(text, text_width)
            } else {
                vec![text.to_owned()]
            };
            for (i, chunk) in chunks.into_iter().enumerate() {
                let mut spans = match (&lead, i) {
                    (Some(l), 0) => l.clone(),
                    _ => vec![Span::raw(indent.clone())],
                };
                spans.push(Span::styled(chunk, msg_style));
                rows.push(Line::from(spans));
            }
        };

    let mut rows: Vec<Line<'static>> = Vec::new();
    if e.crash {
        // Marked by shape as well as color.
        head.push(Span::styled(
            "✖ CRASH ",
            Style::new()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        ));
        head.push(Span::raw(" "));
    }
    push_text(&mut rows, first, Some(head));
    if extra > 0 {
        if expanded {
            for l in msg_lines {
                push_text(&mut rows, l, None);
            }
        } else if let Some(last) = rows.last_mut() {
            last.spans.push(Span::styled(
                format!("  +{extra} lines"),
                Style::new().add_modifier(Modifier::DIM),
            ));
        }
    }
    if selected {
        rows = rows.into_iter().map(|l| l.style(SELECTED)).collect();
    }
    rows
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use jiff::tz::TimeZone;
    use mobicat_core::{Entry, Level, Platform, Source, Store};

    use super::*;

    fn clock() -> Clock {
        Clock::with_tz(TimeZone::UTC)
    }

    fn store_with(n: usize, cap: usize) -> Store {
        let mut s = Store::new(cap);
        push(&mut s, n);
        s
    }

    fn entry(msg: String) -> Entry {
        let mut e = Entry::system(Platform::Android, Arc::from("d"), None, 0, Level::Info, msg);
        e.source = Source::Log;
        e.tag = Arc::from("T");
        e
    }

    fn push(s: &mut Store, n: usize) {
        let base = s.total_in();
        s.push_batch((0..n).map(|i| entry((base + i as u64).to_string())));
    }

    const AREA: Rect = Rect::new(0, 0, 80, 10);

    /// Row text without the one-column selection gutter.
    fn text(v: &ListView) -> Vec<String> {
        v.lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .skip(1)
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    /// Message of the first visible row.
    fn first_msg(v: &ListView) -> String {
        text(v)[0].rsplit(' ').next().unwrap().to_owned()
    }

    fn build(vp: &mut Viewport, s: &Store) -> ListView {
        vp.build(s, AREA, &clock())
    }

    #[test]
    fn follows_tail_by_default() {
        let mut s = store_with(100, 1000);
        let mut vp = Viewport::default();
        assert_eq!(first_msg(&build(&mut vp, &s)), "90");
        assert_eq!(vp.new_below(), None);
        push(&mut s, 5);
        assert_eq!(first_msg(&build(&mut vp, &s)), "95");
    }

    #[test]
    fn moving_up_pauses_and_holds_position_while_lines_arrive() {
        let mut s = store_with(100, 1000);
        let mut vp = Viewport::default();
        build(&mut vp, &s);
        vp.up(&s, 1);
        assert_eq!(vp.cursor(), Some(99), "first step selects the newest entry");
        vp.up(&s, 14);
        let v = build(&mut vp, &s);
        assert_eq!(vp.cursor(), Some(85));
        assert_eq!(first_msg(&v), "85", "the view follows the cursor upward");
        push(&mut s, 20);
        let v = build(&mut vp, &s);
        assert_eq!(first_msg(&v), "85", "view must not move while paused");
        // 120 entries, rows 85..=94 visible.
        assert_eq!(vp.new_below(), Some(25));
    }

    #[test]
    fn moving_past_the_end_resumes_following() {
        let s = store_with(100, 1000);
        let mut vp = Viewport::default();
        vp.up(&s, 3);
        vp.down(&s, 1);
        vp.down(&s, 1);
        assert_eq!(vp.cursor(), Some(99));
        vp.down(&s, 1);
        assert_eq!(vp.cursor(), None);
    }

    #[test]
    fn space_pauses_in_place() {
        let mut s = store_with(100, 1000);
        let mut vp = Viewport::default();
        build(&mut vp, &s);
        vp.toggle_pause(&s);
        build(&mut vp, &s);
        push(&mut s, 10);
        let v = build(&mut vp, &s);
        assert_eq!(first_msg(&v), "90");
        assert_eq!(vp.new_below(), Some(10));
        vp.toggle_pause(&s);
        assert_eq!(first_msg(&build(&mut vp, &s)), "100");
    }

    #[test]
    fn first_jumps_to_oldest_and_survives_eviction() {
        let mut s = store_with(50, 50);
        let mut vp = Viewport::default();
        vp.first(&s);
        assert_eq!(first_msg(&build(&mut vp, &s)), "0");
        push(&mut s, 20); // evicts 0..20
        assert_eq!(first_msg(&build(&mut vp, &s)), "20");
    }

    #[test]
    fn empty_and_short_buffers() {
        let mut vp = Viewport::default();
        let empty = Store::new(10);
        assert!(build(&mut vp, &empty).lines.is_empty());
        vp.up(&empty, 1);
        vp.down(&empty, 1);
        let s = store_with(3, 10);
        assert_eq!(build(&mut vp, &s).lines.len(), 3);
        vp.up(&s, 100);
        vp.down(&s, 100);
        build(&mut vp, &s);
    }

    fn block_store() -> Store {
        let mut s = Store::new(100);
        push(&mut s, 3);
        s.push_batch([entry("Exception\n  at a\n  at b".into())]);
        push(&mut s, 1);
        s
    }

    #[test]
    fn blocks_are_collapsed_until_expanded() {
        let s = block_store();
        let mut vp = Viewport::default();
        let rows = text(&build(&mut vp, &s));
        assert_eq!(rows.len(), 5);
        assert!(rows[3].ends_with("Exception  +2 lines"), "{rows:?}");

        vp.up(&s, 2); // cursor on the block
        assert_eq!(vp.cursor(), Some(3));
        vp.toggle_expand(&s);
        let rows = text(&build(&mut vp, &s));
        assert_eq!(rows.len(), 7);
        assert!(rows[4].trim_start().starts_with("at a"));
        assert!(rows[4].starts_with(&" ".repeat(Layout::default().prefix_width())));
    }

    #[test]
    fn pausing_before_the_first_frame_keeps_the_screen_full() {
        let s = store_with(100, 1000);
        let mut vp = Viewport::default();
        vp.up(&s, 3); // no build yet
        let rows = text(&build(&mut vp, &s));
        assert_eq!(rows.len(), 10);
        assert!(rows[9].ends_with(" 99"), "{rows:?}");
        assert!(rows[7].ends_with(" 97"));
    }

    #[test]
    fn no_blank_rows_below_when_entries_exist_above() {
        let mut s = Store::new(100);
        push(&mut s, 20);
        let long = (0..30)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        s.push_batch([entry(long)]);
        let mut vp = Viewport::default();
        vp.up(&s, 1);
        vp.toggle_expand(&s);
        build(&mut vp, &s); // the tall block fills the screen from the top
        vp.toggle_expand(&s); // collapse: one row left at the top
        let rows = text(&build(&mut vp, &s));
        assert_eq!(rows.len(), 10, "{rows:?}");
        assert!(rows[9].contains("l0  +29 lines"), "{rows:?}");
        assert!(rows[8].ends_with(" 19"));
    }

    #[test]
    fn tabs_render_as_spaces() {
        let mut s = Store::new(10);
        s.push_batch([entry("Exception\n\tat a".into())]);
        let mut vp = Viewport::default();
        vp.up(&s, 1);
        vp.toggle_expand(&s);
        let rows = text(&build(&mut vp, &s));
        assert!(rows[1].ends_with("    at a"), "{rows:?}");
    }

    #[test]
    fn tall_cursor_entry_stays_visible() {
        let mut s = Store::new(100);
        push(&mut s, 20);
        let long = (0..30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        s.push_batch([entry(long)]);
        let mut vp = Viewport::default();
        vp.up(&s, 1);
        vp.toggle_expand(&s);
        let rows = text(&build(&mut vp, &s));
        assert_eq!(rows.len(), 10);
        assert!(
            rows[0].contains("line 0"),
            "the cursor's entry starts at the top: {rows:?}"
        );
    }

    #[test]
    fn wrapping_splits_long_messages_by_display_width() {
        let mut s = Store::new(10);
        s.push_batch([entry("あ".repeat(60))]);
        let mut vp = Viewport::default();
        vp.layout.wrap = true;
        let rows = text(&build(&mut vp, &s));
        // 80 columns - 1 gutter - 40 prefix = 39 columns = 19 wide characters per row.
        assert_eq!(rows.len(), 4, "{rows:?}");
    }

    #[test]
    fn time_modes() {
        let mut s = Store::new(10);
        let mut a = entry("a".into());
        a.time_ns = 1_000_000_000;
        let mut b = entry("b".into());
        b.time_ns = 1_012_500_000;
        s.push_batch([a, b]);
        let mut vp = Viewport::default();
        let rows = text(&build(&mut vp, &s));
        assert!(rows[1].starts_with("00:00:01.012 A"), "{rows:?}");
        vp.layout.time = TimeMode::Delta;
        let rows = text(&build(&mut vp, &s));
        assert!(rows[1].starts_with("     +12.5ms A"), "{rows:?}");
        vp.layout.time = TimeMode::Hidden;
        let rows = text(&build(&mut vp, &s));
        assert!(rows[1].starts_with("A "), "{rows:?}");
    }

    #[test]
    fn wrap_columns_counts_display_width() {
        assert_eq!(wrap_columns("abcdef", 4), vec!["abcd", "ef"]);
        assert_eq!(wrap_columns("ああa", 4), vec!["ああ", "a"]);
        assert_eq!(wrap_columns("", 4), vec![""]);
    }
}
