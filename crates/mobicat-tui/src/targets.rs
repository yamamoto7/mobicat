//! What the log list shows: All, one OS, or one device. The tab row under
//! the header, Tab / Shift-Tab cycling, and the `d` panel with the tree
//! (OS ▸ device).

use mobicat_core::{Platform, Store, View};
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::theme;

pub fn name(store: &Store, v: View) -> String {
    match v {
        View::All => "All".into(),
        View::Only(Platform::Android) => "Android".into(),
        View::Only(Platform::Ios) => "iOS".into(),
        View::Device(d) => store
            .devices()
            .get(d as usize)
            .map_or_else(|| "?".into(), |d| d.name.clone()),
    }
}

/// Whether device `d` is part of view `v`.
pub fn covers(store: &Store, v: View, d: usize) -> bool {
    match v {
        View::All => true,
        View::Only(p) => store.devices().get(d).is_some_and(|x| x.platform == p),
        View::Device(x) => x as usize == d,
    }
}

/// Devices, Android first, in the order they were seen.
pub fn device_order(store: &Store) -> Vec<u16> {
    let mut ids: Vec<u16> = (0..store.devices().len() as u16).collect();
    ids.sort_by_key(|&i| store.devices()[i as usize].platform as u8);
    ids
}

/// The targets Tab cycles through: All, then each device.
pub fn cycle(store: &Store, current: View, forward: bool) -> View {
    let mut all = vec![View::All];
    all.extend(device_order(store).into_iter().map(View::Device));
    let pos = match current {
        View::Only(p) => {
            // From an OS view, go to its first device (or back to All).
            let first = all.iter().position(
                |v| matches!(v, View::Device(d) if store.devices()[*d as usize].platform == p),
            );
            return match (forward, first) {
                (true, Some(i)) => all[i],
                _ => View::All,
            };
        }
        v => all.iter().position(|x| *x == v).unwrap_or(0),
    };
    let n = all.len();
    all[if forward {
        (pos + 1) % n
    } else {
        (pos + n - 1) % n
    }]
}

pub fn platform_dot(p: Platform) -> Span<'static> {
    Span::styled("●", crate::list::platform_style(p))
}

/// Compact counts for tabs: 999, 1.2k, 12k, 1.2M.
pub fn compact(n: usize) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..10_000 => format!("{:.1}k", n as f64 / 1000.0),
        10_000..1_000_000 => format!("{}k", n / 1000),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// The tab row: `All 7  ● Pixel 9 4  ● iPhone 17 Pro 3 ✖1`.
pub fn tab_line(store: &Store, current: View, unseen: &dyn Fn(View) -> usize) -> Line<'static> {
    let mut tabs = vec![View::All];
    let order = device_order(store);
    for &d in &order {
        let p = store.devices()[d as usize].platform;
        if current == View::Only(p) && !tabs.contains(&current) {
            tabs.push(current);
        }
        tabs.push(View::Device(d));
    }
    let mut spans = vec![Span::raw(" ")];
    for (i, &v) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        let cur = v == current;
        let gone = matches!(v, View::Device(d) if !store.devices()[d as usize].connected);
        if let View::Device(d) = v {
            let p = store.devices()[d as usize].platform;
            spans.push(if gone {
                Span::styled("○", theme::dim())
            } else {
                platform_dot(p)
            });
            spans.push(Span::raw(" "));
        }
        let label = name(store, v);
        let count = compact(store.filtered_len(v));
        if cur {
            spans.push(Span::styled(label, theme::accent_bold().underlined()));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(count, theme::accent()));
        } else {
            spans.push(Span::styled(
                label,
                if gone { theme::dim() } else { Style::new() },
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(count, theme::dim()));
            let n = unseen(v);
            if n > 0 {
                spans.push(Span::styled(format!(" ✖{n}"), theme::crash()));
            }
        }
    }
    Line::from(spans)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Row {
    view: View,
    /// 0 = All / OS, 1 = device.
    depth: u8,
    last: bool,
    /// The "Change app…" row at the bottom.
    change_app: bool,
}

impl Row {
    fn target(view: View, depth: u8) -> Self {
        Self {
            view,
            depth,
            last: false,
            change_app: false,
        }
    }
}

/// The `d` panel.
pub struct TargetPanel {
    filter: String,
    cursor: usize,
    /// Folded OS nodes: [Android, iOS].
    folded: [bool; 2],
    can_change_app: bool,
}

pub enum PanelResult {
    Open,
    Close,
    Switch(View),
    ChangeApp,
}

impl TargetPanel {
    pub fn new(store: &Store, current: View, can_change_app: bool) -> Self {
        let mut p = Self {
            filter: String::new(),
            cursor: 0,
            folded: [false; 2],
            can_change_app,
        };
        p.cursor = p
            .rows(store)
            .iter()
            .position(|r| r.view == current)
            .unwrap_or(0);
        p
    }

    fn rows(&self, store: &Store) -> Vec<Row> {
        let f = self.filter.to_lowercase();
        let hit = |s: &str| f.is_empty() || s.to_lowercase().contains(&f);
        let mut rows = Vec::new();
        if hit("All") {
            rows.push(Row::target(View::All, 0));
        }
        for p in [Platform::Android, Platform::Ios] {
            let os = View::Only(p);
            let kids: Vec<Row> = device_order(store)
                .into_iter()
                .filter(|&d| store.devices()[d as usize].platform == p)
                .filter(|&d| hit(&store.devices()[d as usize].name) || hit(&name(store, os)))
                .map(|d| Row::target(View::Device(d), 1))
                .collect();
            if kids.is_empty() {
                continue;
            }
            rows.push(Row::target(os, 0));
            if self.folded[p as usize] && f.is_empty() {
                continue;
            }
            let n = kids.len();
            rows.extend(kids.into_iter().enumerate().map(|(i, mut r)| {
                r.last = i + 1 == n;
                r
            }));
        }
        if self.can_change_app && hit("Change app") {
            rows.push(Row {
                change_app: true,
                ..Row::target(View::All, 0)
            });
        }
        rows
    }

    pub fn key(&mut self, store: &Store, code: KeyCode, ctrl: bool) -> PanelResult {
        let rows = self.rows(store);
        let n = rows.len();
        match code {
            KeyCode::Esc => return PanelResult::Close,
            KeyCode::Enter => {
                return match rows.get(self.cursor) {
                    Some(r) if r.change_app => PanelResult::ChangeApp,
                    Some(r) => PanelResult::Switch(r.view),
                    None => PanelResult::Close,
                };
            }
            KeyCode::Down | KeyCode::Tab => {
                self.cursor = (self.cursor + 1).min(n.saturating_sub(1))
            }
            KeyCode::Char('n') if ctrl => self.cursor = (self.cursor + 1).min(n.saturating_sub(1)),
            KeyCode::Up | KeyCode::BackTab => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('p') if ctrl => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Left | KeyCode::Right => {
                let fold = code == KeyCode::Left;
                if let Some(r) = rows.get(self.cursor).filter(|r| !r.change_app) {
                    let p = match r.view {
                        View::Only(p) => Some(p),
                        View::Device(d) => Some(store.devices()[d as usize].platform),
                        View::All => None,
                    };
                    if let Some(p) = p {
                        self.folded[p as usize] = fold;
                        let os = View::Only(p);
                        if fold {
                            self.cursor = self
                                .rows(store)
                                .iter()
                                .position(|r| r.view == os)
                                .unwrap_or(0);
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.cursor = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                self.filter.push(c);
                self.cursor = 0;
            }
            _ => {}
        }
        let n = self.rows(store).len();
        self.cursor = self.cursor.min(n.saturating_sub(1));
        PanelResult::Open
    }

    pub fn draw(
        &self,
        f: &mut Frame,
        store: &Store,
        current: View,
        unseen: &dyn Fn(View) -> usize,
    ) {
        let rows = self.rows(store);
        let area = f.area();
        let w = 56.min(area.width);
        let h = (rows.len() as u16 + 3).max(5).min(area.height);
        let rect = Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height.saturating_sub(h)) / 3,
            w,
            h,
        );
        let inner_w = w.saturating_sub(2) as usize;
        let mut lines = vec![Line::from(vec![
            Span::styled(" ⌕ ", theme::dim()),
            Span::raw(self.filter.clone()),
            Span::styled("▏", theme::accent()),
            Span::styled(
                if self.filter.is_empty() {
                    "type to narrow"
                } else {
                    ""
                },
                theme::dim(),
            ),
        ])];
        let visible = h.saturating_sub(3) as usize;
        let start = self.cursor.saturating_sub(visible.saturating_sub(1));
        for (i, r) in rows.iter().enumerate().skip(start).take(visible) {
            lines.push(row_line(
                store,
                *r,
                i == self.cursor,
                r.view == current && !r.change_app,
                unseen(r.view),
                inner_w,
                self.folded,
            ));
        }
        if rows.is_empty() {
            lines.push(Line::styled("  no matches", theme::dim()));
        }
        f.render_widget(Clear, rect);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::bordered()
                    .border_type(theme::BORDER)
                    .border_style(theme::accent())
                    .title(Span::styled(" Switch target ", theme::accent_bold()))
                    .title_bottom(Line::styled(
                        " ↑↓ choose · ← → fold · enter switch · esc close ",
                        theme::dim(),
                    )),
            ),
            rect,
        );
    }
}

fn row_line(
    store: &Store,
    r: Row,
    sel: bool,
    cur: bool,
    unseen: usize,
    width: usize,
    folded: [bool; 2],
) -> Line<'static> {
    if r.change_app {
        let line = Line::from(vec![
            if sel {
                Span::styled("❯", theme::accent_bold())
            } else {
                Span::raw(" ")
            },
            Span::styled(
                " ⇄ Change app…",
                if sel {
                    theme::accent_bold()
                } else {
                    theme::dim()
                },
            ),
        ]);
        return if sel {
            line.style(theme::SELECTED)
        } else {
            line
        };
    }
    let mut left: Vec<Span<'static>> = vec![if sel {
        Span::styled("❯", theme::accent_bold())
    } else if cur {
        Span::styled("▌", theme::accent())
    } else {
        Span::raw(" ")
    }];
    let mut gone = false;
    match r.view {
        View::All => left.push(Span::raw(" ")),
        View::Only(p) => left.push(Span::styled(
            if folded[p as usize] { " ▸ " } else { " ▾ " },
            theme::dim(),
        )),
        View::Device(d) => {
            let dev = &store.devices()[d as usize];
            gone = !dev.connected;
            left.push(Span::styled(
                if r.last { "   └ " } else { "   ├ " },
                theme::dim(),
            ));
            left.push(if gone {
                Span::styled("○", theme::dim())
            } else {
                platform_dot(dev.platform)
            });
            left.push(Span::raw(" "));
        }
    }
    let label_style = match (cur, gone, r.view) {
        (true, ..) => theme::accent_bold(),
        (_, true, _) => theme::dim(),
        (_, _, View::Only(p)) => {
            crate::list::platform_style(p).add_modifier(ratatui::style::Modifier::BOLD)
        }
        _ => Style::new(),
    };
    left.push(Span::styled(name(store, r.view), label_style));
    if cur {
        left.push(Span::styled("  current", theme::accent()));
    }
    let mut right: Vec<Span<'static>> = Vec::new();
    if unseen > 0 && !cur {
        right.push(Span::styled(format!("✖{unseen} new  "), theme::crash()));
    }
    right.push(Span::styled(
        if gone {
            "disconnected ".to_owned()
        } else {
            format!("{:>5} lines ", compact(store.filtered_len(r.view)))
        },
        theme::dim(),
    ));
    let used: usize = left.iter().chain(&right).map(|s| s.width()).sum();
    left.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    left.extend(right);
    let line = Line::from(left);
    if sel {
        line.style(theme::SELECTED)
    } else {
        line
    }
}
