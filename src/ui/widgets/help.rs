//! Help panel widget.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

/// Render the help panel
pub fn render_help(frame: &mut Frame, area: Rect) {
    // Clear the area first
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Keyboard Shortcuts ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Black));

    let help_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Navigation", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  ↑/k", Style::default().fg(Color::Cyan)),
            Span::raw("  - Select previous sensor"),
        ]),
        Line::from(vec![
            Span::styled("  ↓/j", Style::default().fg(Color::Cyan)),
            Span::raw("  - Select next sensor"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Controls", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Space/p", Style::default().fg(Color::Cyan)),
            Span::raw(" - Toggle measurement (start/pause)"),
        ]),
        Line::from(vec![
            Span::styled("  r", Style::default().fg(Color::Cyan)),
            Span::raw("       - Reload configuration"),
        ]),
        Line::from(vec![
            Span::styled("  d/Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" - Toggle selected sensor on/off"),
        ]),
        Line::from(vec![
            Span::styled("  c", Style::default().fg(Color::Cyan)),
            Span::raw("       - Clear chart history"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Other", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  ?/h/F1", Style::default().fg(Color::Cyan)),
            Span::raw("  - Toggle this help"),
        ]),
        Line::from(vec![
            Span::styled("  q/Esc", Style::default().fg(Color::Cyan)),
            Span::raw("   - Quit application"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Status Indicators", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  ✓", Style::default().fg(Color::Green)),
            Span::raw(" - Sensor enabled"),
        ]),
        Line::from(vec![
            Span::styled("  ✗", Style::default().fg(Color::DarkGray)),
            Span::raw(" - Sensor disabled"),
        ]),
        Line::from(vec![
            Span::styled("  ▶", Style::default().fg(Color::Yellow)),
            Span::raw(" - Selected sensor"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("Press "),
            Span::styled("?", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" to close this help"),
        ]),
    ];

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}
