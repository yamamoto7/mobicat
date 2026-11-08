//! App picker: shown when mobicat starts without an app ID, and for
//! "Change app…". Sections: Recent, Running now, Installed. Typing narrows to
//! one flat list of matches.

use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::theme;

/// One app and where it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppItem {
    pub id: String,
    pub android: bool,
    pub ios: bool,
    /// A process of the app is running on some device.
    pub running: bool,
    /// Position in the recent list (0 = last used).
    pub recent: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Section {
    Recent,
    Running,
    Installed,
}

impl Section {
    fn of(a: &AppItem) -> Self {
        if a.recent.is_some() {
            Section::Recent
        } else if a.running {
            Section::Running
        } else {
            Section::Installed
        }
    }

    fn title(self) -> &'static str {
        match self {
            Section::Recent => "Recent",
            Section::Running => "Running now",
            Section::Installed => "Installed",
        }
    }
}

struct State {
    items: Vec<AppItem>,
    filter: String,
    selected: usize,
    quit_armed: Option<Instant>,
    /// Esc with an empty filter: "back" or "quit".
    esc_label: &'static str,
}

impl State {
    fn new(mut items: Vec<AppItem>, esc_label: &'static str) -> Self {
        items.sort_by(|a, b| {
            (Section::of(a), a.recent, &a.id).cmp(&(Section::of(b), b.recent, &b.id))
        });
        Self {
            items,
            filter: String::new(),
            selected: 0,
            quit_armed: None,
            esc_label,
        }
    }

    fn visible(&self) -> Vec<&AppItem> {
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|a| a.id.to_lowercase().contains(&f))
            .collect()
    }

    /// `Some(Some(id))` = chosen, `Some(None)` = cancelled, `None` = keep going.
    fn key(&mut self, code: KeyCode, ctrl: bool) -> Option<Option<String>> {
        let n = self.visible().len();
        if ctrl && matches!(code, KeyCode::Char('c') | KeyCode::Char('d')) {
            if self
                .quit_armed
                .is_some_and(|t| t.elapsed() <= Duration::from_secs(2))
            {
                return Some(None);
            }
            self.quit_armed = Some(Instant::now());
            return None;
        }
        match code {
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.selected = 0;
            }
            KeyCode::Esc => return Some(None),
            KeyCode::Enter => return Some(self.visible().get(self.selected).map(|a| a.id.clone())),
            KeyCode::Down | KeyCode::Tab => {
                self.selected = (self.selected + 1).min(n.saturating_sub(1));
            }
            KeyCode::Char('n') if ctrl => {
                self.selected = (self.selected + 1).min(n.saturating_sub(1))
            }
            KeyCode::Up | KeyCode::BackTab => self.selected = self.selected.saturating_sub(1),
            KeyCode::Char('p') if ctrl => self.selected = self.selected.saturating_sub(1),
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                self.filter.push(c);
                self.selected = 0;
            }
            _ => {}
        }
        None
    }

    fn row(&self, a: &AppItem, sel: bool, width: usize) -> Line<'static> {
        let f = self.filter.to_lowercase();
        let mut left = vec![if sel {
            Span::styled("❯ ", theme::accent_bold())
        } else {
            Span::raw("  ")
        }];
        // Underline the matched part.
        let id = a.id.clone();
        let base = if sel {
            theme::accent_bold()
        } else {
            ratatui::style::Style::new()
        };
        match (!f.is_empty())
            .then(|| id.to_lowercase().find(&f))
            .flatten()
        {
            Some(i) => {
                let j = i + f.len();
                left.push(Span::styled(id[..i].to_owned(), base));
                left.push(Span::styled(
                    id[i..j].to_owned(),
                    theme::accent_bold().underlined(),
                ));
                left.push(Span::styled(id[j..].to_owned(), base));
            }
            None => left.push(Span::styled(id, base)),
        }
        let mut right: Vec<Span<'static>> = Vec::new();
        if a.recent == Some(0) {
            right.push(Span::styled("last  ", theme::dim()));
        }
        if a.running {
            right.push(Span::styled("● running  ", theme::accent()));
        }
        right.push(if a.android {
            Span::styled(
                "Android",
                crate::list::platform_style(mobicat_core::Platform::Android),
            )
        } else {
            Span::raw("       ")
        });
        right.push(Span::raw(" "));
        right.push(if a.ios {
            Span::styled(
                "iOS",
                crate::list::platform_style(mobicat_core::Platform::Ios),
            )
        } else {
            Span::raw("   ")
        });
        right.push(Span::raw(" "));
        let used: usize = left.iter().chain(&right).map(Span::width).sum();
        left.push(Span::raw(" ".repeat(width.saturating_sub(used).max(1))));
        left.extend(right);
        let line = Line::from(left);
        if sel {
            line.style(theme::SELECTED)
        } else {
            line
        }
    }

    fn draw(&self, f: &mut ratatui::Frame) {
        let [top, input, list, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(f.area());
        let items = self.visible();
        let width = top.width as usize;
        let left = vec![
            Span::raw(" "),
            Span::styled("mobicat", theme::accent_bold()),
            Span::styled(" · choose an app", theme::dim()),
        ];
        let count = if self.filter.is_empty() {
            format!("{} apps ", self.items.len())
        } else {
            format!("{} / {} ", items.len(), self.items.len())
        };
        let used: usize = left.iter().map(Span::width).sum::<usize>() + count.chars().count();
        let mut head = left;
        head.push(Span::raw(" ".repeat(width.saturating_sub(used).max(1))));
        head.push(Span::styled(count, theme::dim()));
        f.render_widget(Paragraph::new(Line::from(head)), top);

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ⌕ ", theme::dim()),
                Span::raw(self.filter.clone()),
                Span::styled("▏", theme::accent()),
                Span::styled(
                    if self.filter.is_empty() {
                        "type to filter"
                    } else {
                        ""
                    },
                    theme::dim(),
                ),
            ]))
            .block(
                Block::bordered()
                    .border_type(theme::BORDER)
                    .border_style(theme::accent()),
            ),
            input,
        );

        // Rows: section headings (only without a filter) and apps.
        let mut rows: Vec<Option<usize>> = Vec::new();
        let mut headings: Vec<Section> = Vec::new();
        let mut last: Option<Section> = None;
        for (i, a) in items.iter().enumerate() {
            let s = Section::of(a);
            if self.filter.is_empty() && last != Some(s) {
                rows.push(None);
                headings.push(s);
                last = Some(s);
            }
            rows.push(Some(i));
        }
        let height = list.height as usize;
        let sel_row = rows
            .iter()
            .position(|r| *r == Some(self.selected))
            .unwrap_or(0);
        let start = sel_row.saturating_sub(height.saturating_sub(1));
        let mut h = headings.iter();
        let mut lines = Vec::new();
        for (ri, r) in rows.iter().enumerate() {
            match r {
                None => {
                    let s = h.next().copied().unwrap_or(Section::Installed);
                    if ri >= start {
                        let n = items.iter().filter(|a| Section::of(a) == s).count();
                        lines.push(Line::from(vec![
                            Span::styled(format!(" {}", s.title()), theme::dim().bold()),
                            Span::styled(format!(" {n}"), theme::dim()),
                        ]));
                    }
                }
                Some(i) if ri >= start => {
                    lines.push(self.row(items[*i], *i == self.selected, width))
                }
                Some(_) => {}
            }
            if lines.len() >= height {
                break;
            }
        }
        if items.is_empty() {
            lines.push(Line::styled("   no matching apps", theme::dim()));
        }
        f.render_widget(Paragraph::new(lines), list);

        let hint = if self
            .quit_armed
            .is_some_and(|t| t.elapsed() <= Duration::from_secs(2))
        {
            Paragraph::new(" Press Ctrl-C again to exit").style(theme::warn())
        } else if self.filter.is_empty() {
            Paragraph::new(format!(
                " ↑↓ choose · enter open · esc {} · ^c ^c quit",
                self.esc_label
            ))
            .style(theme::dim())
        } else {
            Paragraph::new(" ↑↓ choose · enter open · esc clear").style(theme::dim())
        };
        f.render_widget(hint, bottom);
    }
}

/// Let the user choose an app. `None` if cancelled. `can_go_back` changes the
/// Esc hint from "quit" to "back" (when switching apps mid-run).
pub fn pick_app(items: Vec<AppItem>, can_go_back: bool) -> Result<Option<String>> {
    let mut state = State::new(items, if can_go_back { "back" } else { "quit" });
    let mut terminal = ratatui::init();
    let result = (|| -> Result<Option<String>> {
        loop {
            terminal.draw(|f| state.draw(f))?;
            // Wake up now and then so "Press Ctrl-C again" goes away.
            if !event::poll(Duration::from_millis(500))? {
                continue;
            }
            if let Event::Key(k) = event::read()?
                && k.kind != KeyEventKind::Release
                && let Some(done) = state.key(k.code, k.modifiers.contains(KeyModifiers::CONTROL))
            {
                return Ok(done);
            }
        }
    })();
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn items() -> Vec<AppItem> {
        let app = |id: &str, android, ios, running, recent| AppItem {
            id: id.into(),
            android,
            ios,
            running,
            recent,
        };
        vec![
            app("net.acme.notes", true, false, false, None),
            app("com.example.other", true, false, false, None),
            app("org.sample.shop", true, true, true, None),
            app("com.example.widgets", false, true, false, None),
            app("com.example.app", true, true, true, Some(0)),
        ]
    }

    #[test]
    fn sections_then_narrowing_and_choosing() {
        let mut s = State::new(items(), "quit");
        let mut t = Terminal::new(TestBackend::new(70, 14)).unwrap();
        t.draw(|f| s.draw(f)).unwrap();
        insta::assert_snapshot!(t.backend().to_string());
        assert_eq!(
            s.key(KeyCode::Enter, false),
            Some(Some("com.example.app".into()))
        );
        for c in "other".chars() {
            assert!(s.key(KeyCode::Char(c), false).is_none());
        }
        assert_eq!(
            s.key(KeyCode::Enter, false),
            Some(Some("com.example.other".into()))
        );
        assert_eq!(
            s.key(KeyCode::Esc, false),
            None,
            "Esc clears the filter first"
        );
        assert_eq!(s.key(KeyCode::Esc, false), Some(None));
    }

    #[test]
    fn ctrl_c_twice_cancels() {
        let mut s = State::new(items(), "quit");
        assert_eq!(s.key(KeyCode::Char('c'), true), None);
        assert_eq!(s.key(KeyCode::Char('c'), true), Some(None));
    }
}
