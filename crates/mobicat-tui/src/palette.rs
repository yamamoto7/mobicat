//! `:` actions: every action by name, with its key, so nothing has to be
//! memorized.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Same as pressing the key.
    Key(KeyCode, KeyModifiers),
    Quit,
    ChangeApp,
}

const N: KeyModifiers = KeyModifiers::NONE;
const C: KeyModifiers = KeyModifiers::CONTROL;

/// (name, key shown, action)
pub const ACTIONS: &[(&str, &str, Act)] = &[
    ("Filter (query)…", "/", Act::Key(KeyCode::Char('/'), N)),
    ("Search…", "f", Act::Key(KeyCode::Char('f'), N)),
    ("Next crash", "]c", Act::Key(KeyCode::Char('c'), N)),
    ("Previous crash", "[c", Act::Key(KeyCode::Char('c'), N)),
    ("Next error", "]e", Act::Key(KeyCode::Char('e'), N)),
    ("Next warning", "]w", Act::Key(KeyCode::Char('w'), N)),
    ("Pause / resume", "Space", Act::Key(KeyCode::Char(' '), N)),
    ("Follow the newest", "G", Act::Key(KeyCode::Char('G'), N)),
    ("Go to the oldest", "g", Act::Key(KeyCode::Char('g'), N)),
    ("Switch target…", "d", Act::Key(KeyCode::Char('d'), N)),
    ("Next target", "Tab", Act::Key(KeyCode::Tab, N)),
    (
        "Previous target",
        "Shift-Tab",
        Act::Key(KeyCode::BackTab, N),
    ),
    ("Change app…", "", Act::ChangeApp),
    (
        "Pick a tag or subsystem…",
        "t",
        Act::Key(KeyCode::Char('t'), N),
    ),
    ("App-only on / off", "a", Act::Key(KeyCode::Char('a'), N)),
    (
        "Raise the minimum level",
        "+",
        Act::Key(KeyCode::Char('+'), N),
    ),
    (
        "Lower the minimum level",
        "-",
        Act::Key(KeyCode::Char('-'), N),
    ),
    (
        "Query hides / highlights",
        "x",
        Act::Key(KeyCode::Char('x'), N),
    ),
    ("Detail pane", "Enter", Act::Key(KeyCode::Enter, N)),
    ("Open / close block", "o", Act::Key(KeyCode::Char('o'), N)),
    ("Wrap long lines", "^w", Act::Key(KeyCode::Char('w'), C)),
    (
        "Time column: clock / delta / off",
        "T",
        Act::Key(KeyCode::Char('T'), N),
    ),
    ("Copy entry", "y", Act::Key(KeyCode::Char('y'), N)),
    ("Select a range", "V", Act::Key(KeyCode::Char('V'), N)),
    (
        "Copy everything shown",
        "Y",
        Act::Key(KeyCode::Char('Y'), N),
    ),
    (
        "Save everything shown to a file",
        "^s",
        Act::Key(KeyCode::Char('s'), C),
    ),
    (
        "Clear the screen and buffer…",
        "c",
        Act::Key(KeyCode::Char('c'), N),
    ),
    ("Keys", "?", Act::Key(KeyCode::Char('?'), N)),
    ("Quit", "^c ^c", Act::Quit),
];

/// The chord prefix an action needs before its key, if any.
pub fn chord(i: usize) -> Option<char> {
    let keys = ACTIONS[i].1;
    (keys.len() == 2 && (keys.starts_with(']') || keys.starts_with('[')))
        .then(|| keys.chars().next().unwrap_or(']'))
}

pub struct Palette {
    filter: String,
    cursor: usize,
    /// Show "Change app…" (only when mobicat can restart with another app).
    can_change_app: bool,
}

pub enum PaletteResult {
    Open,
    Close,
    Run(usize),
}

impl Palette {
    pub fn new(can_change_app: bool) -> Self {
        Self {
            filter: String::new(),
            cursor: 0,
            can_change_app,
        }
    }

    fn visible(&self) -> Vec<usize> {
        let words: Vec<String> = self
            .filter
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        (0..ACTIONS.len())
            .filter(|&i| self.can_change_app || ACTIONS[i].2 != Act::ChangeApp)
            .filter(|&i| {
                let name = ACTIONS[i].0.to_lowercase();
                words.iter().all(|w| name.contains(w.as_str()))
            })
            .collect()
    }

    pub fn key(&mut self, code: KeyCode, ctrl: bool) -> PaletteResult {
        let items = self.visible();
        let n = items.len();
        match code {
            KeyCode::Esc => return PaletteResult::Close,
            KeyCode::Enter => {
                return items
                    .get(self.cursor)
                    .map_or(PaletteResult::Close, |&i| PaletteResult::Run(i));
            }
            KeyCode::Down | KeyCode::Tab => {
                self.cursor = (self.cursor + 1).min(n.saturating_sub(1))
            }
            KeyCode::Char('n') if ctrl => self.cursor = (self.cursor + 1).min(n.saturating_sub(1)),
            KeyCode::Up | KeyCode::BackTab => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('p') if ctrl => self.cursor = self.cursor.saturating_sub(1),
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
        PaletteResult::Open
    }

    pub fn draw(&self, f: &mut Frame) {
        let items = self.visible();
        let area = f.area();
        let w = 56.min(area.width);
        let h = (items.len() as u16 + 3).clamp(5, 18).min(area.height);
        let rect = Rect::new(
            area.x + (area.width - w) / 2,
            area.y + area.height.saturating_sub(h) / 3,
            w,
            h,
        );
        let inner_w = w.saturating_sub(2) as usize;
        let mut lines = vec![Line::from(vec![
            Span::styled(" : ", theme::accent_bold()),
            Span::raw(self.filter.clone()),
            Span::styled("▏", theme::accent()),
            Span::styled(
                if self.filter.is_empty() {
                    "type to find an action"
                } else {
                    ""
                },
                theme::dim(),
            ),
        ])];
        let rows = h.saturating_sub(3) as usize;
        let start = self.cursor.saturating_sub(rows.saturating_sub(1));
        for (pos, &i) in items.iter().enumerate().skip(start).take(rows) {
            let (name, keys, _) = ACTIONS[i];
            let sel = pos == self.cursor;
            let left = format!("{} {name}", if sel { "❯" } else { " " });
            let right = format!("{keys} ");
            let pad = inner_w.saturating_sub(left.chars().count() + right.chars().count());
            let line = Line::from(vec![
                Span::styled(
                    left,
                    if sel {
                        theme::accent_bold()
                    } else {
                        ratatui::style::Style::new()
                    },
                ),
                Span::raw(" ".repeat(pad)),
                Span::styled(right, theme::dim()),
            ]);
            lines.push(if sel {
                line.style(theme::SELECTED)
            } else {
                line
            });
        }
        if items.is_empty() {
            lines.push(Line::styled("  no matching action", theme::dim()));
        }
        f.render_widget(Clear, rect);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::bordered()
                    .border_type(theme::BORDER)
                    .border_style(theme::accent())
                    .title(Span::styled(" Actions ", theme::accent_bold()))
                    .title_bottom(Line::styled(
                        " ↑↓ choose · enter run · esc close ",
                        theme::dim(),
                    )),
            ),
            rect,
        );
    }
}
