//! Shared widget renderers.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

pub fn render_header(frame: &mut Frame, area: Rect, version: &str) {
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("  sensors-to-mqtt v{}  ", version),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " ← → or 1-9: switch tabs   q: quit",
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(block);
    frame.render_widget(title, area);
}

// ---------------------------------------------------------------------------
// Log panel (always visible at bottom)
// ---------------------------------------------------------------------------

pub fn render_log_panel(frame: &mut Frame, area: Rect, logs: &[String]) {
    let block = Block::default()
        .title(" LOG (recent) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let height = area.height.saturating_sub(2) as usize;
    let start = logs.len().saturating_sub(height);
    let items: Vec<ListItem> = logs[start..]
        .iter()
        .map(|line| {
            let style = if line.contains("ERROR") {
                Style::default().fg(Color::Red)
            } else if line.contains("WARN") {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(Line::from(Span::styled(line.clone(), style)))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

// ---------------------------------------------------------------------------
// Helpers re-exported to tab renderers
// ---------------------------------------------------------------------------

pub fn section_line(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("── {} ──", title),
        Style::default().fg(Color::DarkGray),
    ))
}

pub fn data_row(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<14}", label),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(value, Style::default().fg(Color::White)),
    ])
}

pub fn status_dot(connected: bool) -> Span<'static> {
    if connected {
        Span::styled("● ONLINE ", Style::default().fg(Color::Green))
    } else {
        Span::styled("○ OFFLINE", Style::default().fg(Color::Red))
    }
}
