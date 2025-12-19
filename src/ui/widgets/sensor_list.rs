//! Sensor list widget for displaying and selecting sensors.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};
use std::collections::HashMap;

/// Render a sensor list widget
pub fn render_sensor_list(
    frame: &mut Frame,
    area: Rect,
    sensor_names: &[String],
    sensor_enabled: &HashMap<String, bool>,
    selected: usize,
) {
    let block = Block::default()
        .title(" Sensors ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let items: Vec<ListItem> = sensor_names
        .iter()
        .map(|name| {
            let enabled = sensor_enabled.get(name).copied().unwrap_or(true);
            let status = if enabled { "✓" } else { "✗" };
            let style = if enabled {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            ListItem::new(Line::from(vec![
                ratatui::text::Span::styled(status, style),
                ratatui::text::Span::raw(" "),
                ratatui::text::Span::raw(name.as_str()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    frame.render_stateful_widget(list, area, &mut list_state);
}
