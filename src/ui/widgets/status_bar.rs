//! Status bar widget.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Render the status bar
pub fn render_status_bar(
    frame: &mut Frame,
    area: Rect,
    is_measuring: bool,
    mqtt_connected: bool,
    status_message: Option<&str>,
    error_message: Option<&str>,
) {
    let mut spans = vec![];

    // Measurement status
    let measure_text = if is_measuring { "MEASURING" } else { "PAUSED" };
    let measure_color = if is_measuring { Color::Green } else { Color::Yellow };
    spans.push(Span::styled(
        measure_text,
        Style::default().fg(measure_color).add_modifier(Modifier::BOLD)
    ));

    spans.push(Span::raw(" │ "));

    // MQTT status
    let mqtt_text = if mqtt_connected { "MQTT ✓" } else { "MQTT ✗" };
    let mqtt_color = if mqtt_connected { Color::Green } else { Color::Red };
    spans.push(Span::styled(
        mqtt_text,
        Style::default().fg(mqtt_color)
    ));

    // Error message takes priority
    if let Some(error) = error_message {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            format!("ERROR: {}", error),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        ));
    } else if let Some(status) = status_message {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            status,
            Style::default().fg(Color::Gray)
        ));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    frame.render_widget(paragraph, area);
}
