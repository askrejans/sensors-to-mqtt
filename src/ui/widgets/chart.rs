//! Chart widget for displaying historical sensor data.

use crate::ui::app::SensorHistory;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
    Frame,
};

/// Render a chart widget showing historical G-force data
pub fn render_chart(frame: &mut Frame, area: Rect, history: Option<&SensorHistory>, sensor_name: &str) {
    let block = Block::default()
        .title(format!(" G-Force History: {} ", sensor_name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    if let Some(hist) = history {
        if hist.timestamps.is_empty() {
            let chart = Chart::new(vec![])
                .block(block)
                .x_axis(Axis::default().title("Time").bounds([0.0, 1.0]))
                .y_axis(Axis::default().title("G").bounds([-3.0, 3.0]));
            frame.render_widget(chart, area);
            return;
        }

        // Prepare data points
        let data_len = hist.g_force_x.len();
        let x_data: Vec<f64> = (0..data_len).map(|i| i as f64).collect();

        let x_dataset: Vec<(f64, f64)> = x_data.iter()
            .zip(hist.g_force_x.iter())
            .map(|(x, y)| (*x, *y))
            .collect();

        let y_dataset: Vec<(f64, f64)> = x_data.iter()
            .zip(hist.g_force_y.iter())
            .map(|(x, y)| (*x, *y))
            .collect();

        let z_dataset: Vec<(f64, f64)> = x_data.iter()
            .zip(hist.g_force_z.iter())
            .map(|(x, y)| (*x, *y))
            .collect();

        // Calculate bounds
        let stats = hist.get_stats();
        let y_min = stats.g_force_x.0
            .min(stats.g_force_y.0)
            .min(stats.g_force_z.0)
            .min(-0.5);
        let y_max = stats.g_force_x.1
            .max(stats.g_force_y.1)
            .max(stats.g_force_z.1)
            .max(0.5);

        let datasets = vec![
            Dataset::default()
                .name("Lateral (X)")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Red))
                .data(&x_dataset),
            Dataset::default()
                .name("Forward (Y)")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Green))
                .data(&y_dataset),
            Dataset::default()
                .name("Vertical (Z)")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Blue))
                .data(&z_dataset),
        ];

        let chart = Chart::new(datasets)
            .block(block)
            .x_axis(
                Axis::default()
                    .title("Samples")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([0.0, data_len as f64])
            )
            .y_axis(
                Axis::default()
                    .title("G-Force")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([y_min, y_max])
                    .labels(vec![
                        format!("{:.1}", y_min).into(),
                        "0.0".into(),
                        format!("{:.1}", y_max).into(),
                    ])
            );

        frame.render_widget(chart, area);
    } else {
        let chart = Chart::new(vec![])
            .block(block)
            .x_axis(Axis::default().title("Time").bounds([0.0, 1.0]))
            .y_axis(Axis::default().title("G").bounds([-3.0, 3.0]));
        frame.render_widget(chart, area);
    }
}
