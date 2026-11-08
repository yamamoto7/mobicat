//! Colors and shared styles. Only the 16 ANSI colors are used, so the
//! terminal's color scheme decides the exact shades.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::BorderType;

/// The one accent: current tab, selection bar, focused boxes, input.
pub const ACCENT: Color = Color::Cyan;
/// Background of selected rows.
pub const SELECTED: Style = Style::new().bg(Color::DarkGray);
pub const BORDER: BorderType = BorderType::Rounded;

pub fn accent() -> Style {
    Style::new().fg(ACCENT)
}

pub fn accent_bold() -> Style {
    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

pub fn crash() -> Style {
    Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
}

pub fn warn() -> Style {
    Style::new().fg(Color::Yellow)
}

pub fn ok() -> Style {
    Style::new().fg(Color::Green)
}
