//! Detail pane: every field of the selected entry and its full message.

use mobicat_core::{Clock, Entry, Platform, Source};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use crate::list::{level_style, platform_style};

pub struct Detail<'a> {
    pub entry: &'a Entry,
    pub clock: &'a Clock,
}

fn field(name: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{name:>10}  "),
            Style::new().add_modifier(Modifier::DIM),
        ),
        Span::raw(value),
    ])
}

impl Widget for Detail<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let e = self.entry;
        let platform = match e.platform {
            Platform::Android => "Android",
            Platform::Ios => "iOS",
        };
        let source = match e.source {
            Source::Log => "log",
            Source::Stdout => "stdout",
            Source::Stderr => "stderr",
            Source::System => "mobicat",
        };
        let mut lines = vec![
            Line::from(vec![
                Span::styled(format!(" {platform} "), platform_style(e.platform)),
                Span::raw(" "),
                Span::styled(format!(" {} ", e.level.letter()), level_style(e.level)),
                Span::raw(" "),
                Span::styled(e.tag.to_string(), Style::new().add_modifier(Modifier::BOLD)),
            ]),
            field("time", self.clock.rfc3339(e.time_ns)),
            field("device", e.device.to_string()),
        ];
        if let Some(app) = &e.app {
            lines.push(field("app", app.to_string()));
        }
        lines.push(field("pid / tid", format!("{} / {}", e.pid, e.tid)));
        for (name, v) in [
            ("subsystem", &e.subsystem),
            ("category", &e.category),
            ("library", &e.library),
        ] {
            if let Some(v) = v {
                lines.push(field(name, v.to_string()));
            }
        }
        let scope = if e.app_only {
            "app's own log"
        } else {
            "framework / system (hidden by app-only)"
        };
        lines.push(field("source", format!("{source} · {scope}")));
        lines.push(Line::raw(""));
        lines.extend(
            e.message
                .split('\n')
                .map(|l| Line::raw(l.replace('\t', "    "))),
        );

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(
                Block::new()
                    .borders(Borders::TOP)
                    .title(" detail  Enter/Esc close "),
            )
            .render(area, buf);
    }
}
