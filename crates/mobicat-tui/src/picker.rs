//! Tag / subsystem picker (`t`): the most frequent values in view, filterable
//! by typing. Enter adds `tag:X`, `-` adds `-tag:X`.

use std::collections::HashMap;

use mobicat_core::{Source, Store, View};
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

const MAX_ITEMS: usize = 300;

pub struct Picker {
    /// (field, value, count), most frequent first.
    items: Vec<(&'static str, String, usize)>,
    filter: String,
    selected: usize,
}

pub enum PickerResult {
    Open,
    Close,
    /// A query term to add.
    Add(String),
}

impl Picker {
    pub fn from_store(store: &Store, view: View) -> Self {
        let mut counts: HashMap<(&'static str, String), usize> = HashMap::new();
        for i in 0..store.filtered_len(view) {
            let Some(e) = store.filtered_seq(view, i).and_then(|s| store.get(s)) else {
                continue;
            };
            if e.source == Source::System {
                continue;
            }
            *counts.entry(("tag", e.tag.to_string())).or_default() += 1;
            if let Some(s) = &e.subsystem {
                *counts.entry(("subsystem", s.to_string())).or_default() += 1;
            }
        }
        let mut items: Vec<_> = counts.into_iter().map(|((f, v), n)| (f, v, n)).collect();
        items.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
        items.truncate(MAX_ITEMS);
        Self {
            items,
            filter: String::new(),
            selected: 0,
        }
    }

    fn visible(&self) -> Vec<&(&'static str, String, usize)> {
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|(_, v, _)| f.is_empty() || v.to_lowercase().contains(&f))
            .collect()
    }

    pub fn key(&mut self, code: KeyCode) -> PickerResult {
        let n = self.visible().len();
        match code {
            KeyCode::Esc => return PickerResult::Close,
            KeyCode::Down => self.selected = (self.selected + 1).min(n.saturating_sub(1)),
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Enter | KeyCode::Char('-')
                if !(code == KeyCode::Char('-') && self.filter.ends_with('\\')) =>
            {
                let neg = if code == KeyCode::Char('-') { "-" } else { "" };
                return match self.visible().get(self.selected) {
                    Some((field, value, _)) => {
                        PickerResult::Add(format!("{neg}{field}:{}", quote(value)))
                    }
                    None => PickerResult::Close,
                };
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
            }
            _ => {}
        }
        PickerResult::Open
    }

    pub fn draw(&self, f: &mut Frame) {
        let area = f.area();
        let w = 64.min(area.width);
        let h = 20.min(area.height);
        let rect = Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        );
        let items = self.visible();
        let rows = h.saturating_sub(3) as usize;
        let start = self.selected.saturating_sub(rows.saturating_sub(1));
        let mut lines = vec![Line::from(vec![
            Span::styled(
                " type to narrow: ",
                Style::new().add_modifier(Modifier::DIM),
            ),
            Span::raw(self.filter.clone()),
            Span::raw("▏"),
        ])];
        for (i, (field, value, n)) in items.iter().enumerate().skip(start).take(rows) {
            let style = if i == self.selected {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new()
            };
            lines.push(Line::styled(format!(" {n:>7}  {field:<9} {value}"), style));
        }
        if items.is_empty() {
            lines.push(Line::raw(" no matches"));
        }
        f.render_widget(Clear, rect);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::bordered()
                    .title(" filter by  ↑↓ move  Enter include  - exclude  Esc close "),
            ),
            rect,
        );
    }
}

fn quote(v: &str) -> String {
    if v.is_empty() || v.chars().any(|c| c.is_whitespace() || "()|&\"".contains(c)) {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_owned()
    }
}
