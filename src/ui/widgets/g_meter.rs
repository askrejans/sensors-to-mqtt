//! G-meter widget for displaying current G-forces.

use crate::sensors::SensorData;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Render a G-meter widget showing current G-forces
pub fn render_g_meter(frame: &mut Frame, area: Rect, sensor_data: Option<&SensorData>, sensor_name: &str) {
    let block = Block::default()
        .title(format!(" G-Forces: {} ", sensor_name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    if let Some(data) = sensor_data {
        let g_x = data.data.get("g_force_x").copied().unwrap_or(0.0);
        let g_y = data.data.get("g_force_y").copied().unwrap_or(0.0);
        let g_z = data.data.get("g_force_z").copied().unwrap_or(0.0);

        // Calculate magnitude
        let magnitude = (g_x * g_x + g_y * g_y + g_z * g_z).sqrt();

        // Create visual representation
        let mut lines = vec![];

        // Title
        lines.push(Line::from(vec![
            Span::styled("G-Force Visualization", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(""));

        // Lateral (X) - horizontal bar
        lines.push(Line::from(vec![
            Span::raw("Lateral:  "),
            Span::styled(format!("{:6.3} G", g_x), get_g_color(g_x.abs())),
        ]));
        lines.push(Line::from(create_bar(g_x, 3.0, 40)));
        lines.push(Line::from(""));

        // Forward (Y) - horizontal bar
        lines.push(Line::from(vec![
            Span::raw("Forward:  "),
            Span::styled(format!("{:6.3} G", g_y), get_g_color(g_y.abs())),
        ]));
        lines.push(Line::from(create_bar(g_y, 3.0, 40)));
        lines.push(Line::from(""));

        // Vertical (Z) - horizontal bar
        lines.push(Line::from(vec![
            Span::raw("Vertical: "),
            Span::styled(format!("{:6.3} G", g_z), get_g_color(g_z.abs())),
        ]));
        lines.push(Line::from(create_bar(g_z, 3.0, 40)));
        lines.push(Line::from(""));

        // Magnitude
        lines.push(Line::from("─".repeat(40)));
        lines.push(Line::from(vec![
            Span::raw("Magnitude: "),
            Span::styled(
                format!("{:6.3} G", magnitude),
                get_g_color(magnitude)
            ),
        ]));

        // Angles if available
        if let (Some(lean), Some(bank)) = (data.data.get("lean_angle"), data.data.get("bank_angle")) {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("Lean:  "),
                Span::styled(format!("{:6.2}°", lean), Style::default().fg(Color::Cyan)),
            ]));
            lines.push(Line::from(vec![
                Span::raw("Bank:  "),
                Span::styled(format!("{:6.2}°", bank), Style::default().fg(Color::Cyan)),
            ]));
        }

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(paragraph, area);
    } else {
        let paragraph = Paragraph::new("No data available")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
    }
}

/// Get color based on G-force magnitude
fn get_g_color(g: f64) -> Style {
    let color = if g < 1.0 {
        Color::Green
    } else if g < 2.0 {
        Color::Yellow
    } else if g < 3.0 {
        Color::LightRed
    } else {
        Color::Red
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

/// Create a visual bar for G-force value
fn create_bar(value: f64, max: f64, width: usize) -> Vec<Span<'static>> {
    let center = width / 2;
    let scale = (width as f64 / 2.0) / max;
    let bar_length = ((value.abs() * scale) as usize).min(center);

    let mut spans = Vec::new();

    if value < 0.0 {
        // Negative value (left side)
        let empty_start = center - bar_length;
        spans.push(Span::raw(" ".repeat(empty_start)));
        spans.push(Span::styled(
            "█".repeat(bar_length),
            get_g_color(value.abs())
        ));
        spans.push(Span::styled("│", Style::default().fg(Color::White)));
        spans.push(Span::raw(" ".repeat(center)));
    } else {
        // Positive value (right side)
        spans.push(Span::raw(" ".repeat(center)));
        spans.push(Span::styled("│", Style::default().fg(Color::White)));
        spans.push(Span::styled(
            "█".repeat(bar_length),
            get_g_color(value.abs())
        ));
        spans.push(Span::raw(" ".repeat(center - bar_length)));
    }

    spans
}
