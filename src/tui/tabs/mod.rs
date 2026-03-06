//! Tab renderers and StateSnapshot.

use std::collections::VecDeque;
use std::sync::Arc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Context};
use ratatui::widgets::{
    Axis, Block, Borders, Chart, Dataset, GraphType, List, ListItem, Paragraph,
};

use crate::models::{AppState, MqttStatus, SensorHistory, SensorStatus};
use crate::sensors::SensorData;
use crate::tui::widgets::{data_row, section_line, status_dot};

// ---------------------------------------------------------------------------
// Immutable snapshot — built once per render tick, passed to all renderers
// ---------------------------------------------------------------------------

pub struct StateSnapshot {
    pub version: String,
    pub sensor_names: Vec<String>,
    pub sensor_statuses: Vec<SensorStatus>,
    pub sensor_data: Vec<Option<SensorData>>,
    pub sensor_history: Vec<Option<SensorHistory>>,
    pub mqtt_status: MqttStatus,
    pub messages_published: u64,
    pub mqtt_address: String,
    pub mqtt_enabled: bool,
    pub logs: Vec<String>,
    pub selected_tab: usize,
}

impl StateSnapshot {
    pub fn from(s: &AppState, log_buf: &Arc<std::sync::Mutex<VecDeque<String>>>) -> Self {
        let sensor_names = s.sensor_names();
        let sensor_statuses: Vec<SensorStatus> = sensor_names
            .iter()
            .map(|n| {
                s.sensor_statuses
                    .get(n)
                    .cloned()
                    .unwrap_or_else(|| SensorStatus {
                        name: n.clone(),
                        driver: String::new(),
                        connection_display: String::new(),
                        enabled: false,
                        connected: false,
                        last_error: None,
                    })
            })
            .collect();
        let sensor_data: Vec<Option<SensorData>> = sensor_names
            .iter()
            .map(|n| s.sensor_data.get(n).cloned())
            .collect();
        let sensor_history: Vec<Option<SensorHistory>> = sensor_names
            .iter()
            .map(|n| s.sensor_history.get(n).cloned())
            .collect();

        let logs = {
            let g = log_buf.lock().unwrap();
            g.iter().cloned().collect()
        };

        use std::sync::atomic::Ordering;
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            sensor_names,
            sensor_statuses,
            sensor_data,
            sensor_history,
            mqtt_status: s.mqtt_status.read().unwrap().clone(),
            messages_published: s.messages_published.load(Ordering::Relaxed),
            mqtt_address: s.mqtt_address.clone(),
            mqtt_enabled: s.mqtt_enabled,
            logs,
            selected_tab: s.selected_tab,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-sensor tab
// ---------------------------------------------------------------------------

pub fn render_sensor_tab(frame: &mut Frame, area: Rect, snap: &StateSnapshot, idx: usize) {
    let status = snap.sensor_statuses.get(idx);
    let data = snap.sensor_data.get(idx).and_then(|d| d.as_ref());
    let history = snap.sensor_history.get(idx).and_then(|h| h.as_ref());

    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    // Left: sensor info + field list
    render_sensor_info(frame, outer[0], status, data);

    // Right: visualisations
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(outer[1]);

    // Top-right: G-meter + G-ball
    let viz_top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(right[0]);

    render_g_meter(frame, viz_top[0], data);
    render_g_ball(frame, viz_top[1], data);

    // Bottom-right: sparklines + orientation compass
    let viz_bot = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(right[1]);

    render_sparklines(frame, viz_bot[0], history);
    render_orientation(frame, viz_bot[1], data);
}

// ---------------------------------------------------------------------------
// Info panel
// ---------------------------------------------------------------------------

fn render_sensor_info(
    frame: &mut Frame,
    area: Rect,
    status: Option<&SensorStatus>,
    data: Option<&SensorData>,
) {
    let block = Block::default()
        .title(" SENSOR INFO ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let mut lines: Vec<Line> = Vec::new();

    if let Some(st) = status {
        lines.push(data_row("Name", st.name.clone()));
        lines.push(data_row("Driver", st.driver.clone()));
        lines.push(data_row("Connection", st.connection_display.clone()));
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<14}", "Status"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            status_dot(st.connected),
        ]));
        if let Some(ref e) = st.last_error {
            lines.push(data_row("Error", e.clone()));
        }
        lines.push(section_line("READINGS"));
    }

    if let Some(d) = data {
        let ts_str = d.timestamp.format("%H:%M:%S.%3f").to_string();
        lines.push(data_row("Timestamp", ts_str));
        lines.push(section_line("G-FORCES"));
        for key in &[
            "g_force_x",
            "g_force_y",
            "g_force_z",
            "combined_g",
            "peak_g",
        ] {
            if let Some(&v) = d.fields.get(*key) {
                lines.push(data_row(key, format!("{:+.4} G", v)));
            }
        }
        lines.push(section_line("GYROSCOPE"));
        for key in &["roll_rate", "pitch_rate", "yaw_rate", "angular_velocity"] {
            if let Some(&v) = d.fields.get(*key) {
                lines.push(data_row(key, format!("{:+.2} °/s", v)));
            }
        }
        lines.push(section_line("ORIENTATION"));
        for key in &["lean_angle", "bank_angle", "tilt_angle"] {
            if let Some(&v) = d.fields.get(*key) {
                lines.push(data_row(key, format!("{:+.2}°", v)));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  Waiting for data…",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// ASCII G-meter
// ---------------------------------------------------------------------------

fn render_g_meter(frame: &mut Frame, area: Rect, data: Option<&SensorData>) {
    let block = Block::default()
        .title(" G-METER ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(d) = data {
        let bar_width = inner.width.saturating_sub(20) as usize;
        let max_g = 4.0_f64;
        let axes = [
            ("Lateral  X", "g_force_x"),
            ("Forward  Y", "g_force_y"),
            ("Vertical Z", "g_force_z"),
        ];

        let mut lines: Vec<Line> = Vec::new();
        for (label, key) in &axes {
            let val = d.fields.get(*key).copied().unwrap_or(0.0);
            let fill = ((val.abs() / max_g) * bar_width as f64).min(bar_width as f64) as usize;
            let empty = bar_width.saturating_sub(fill);
            let bar_color = g_color(val.abs());
            let bar_str = format!("█").repeat(fill) + &"░".repeat(empty);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<12} {:+.3}G ", label, val),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(bar_str, Style::default().fg(bar_color)),
            ]));
        }

        if let Some(&cg) = d.fields.get("combined_g") {
            lines.push(Line::from(Span::styled(
                format!("Combined      {:.3} G", cg),
                Style::default()
                    .fg(g_color(cg))
                    .add_modifier(Modifier::BOLD),
            )));
        }
        if let Some(&pg) = d.fields.get("peak_g") {
            lines.push(Line::from(Span::styled(
                format!("Peak          {:.3} G", pg),
                Style::default().fg(Color::Red),
            )));
        }

        let para = Paragraph::new(lines);
        frame.render_widget(para, inner);
    }
}

fn g_color(g: f64) -> Color {
    if g >= 3.0 {
        Color::Red
    } else if g >= 2.0 {
        Color::LightRed
    } else if g >= 1.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

// ---------------------------------------------------------------------------
// G-ball — 2-D dot on a circular canvas
// ---------------------------------------------------------------------------

fn render_g_ball(frame: &mut Frame, area: Rect, data: Option<&SensorData>) {
    let block = Block::default()
        .title(" G-BALL ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let gx = data
        .and_then(|d| d.fields.get("g_force_x"))
        .copied()
        .unwrap_or(0.0);
    let gy = data
        .and_then(|d| d.fields.get("g_force_y"))
        .copied()
        .unwrap_or(0.0);

    let canvas = Canvas::default()
        .block(block)
        .x_bounds([-2.0, 2.0])
        .y_bounds([-2.0, 2.0])
        .paint(move |ctx: &mut Context| {
            ctx.draw(&ratatui::widgets::canvas::Circle {
                x: 0.0,
                y: 0.0,
                radius: 2.0,
                color: Color::DarkGray,
            });
            // 1G ring
            ctx.draw(&ratatui::widgets::canvas::Circle {
                x: 0.0,
                y: 0.0,
                radius: 1.0,
                color: Color::DarkGray,
            });
            // Cross-hairs
            ctx.draw(&ratatui::widgets::canvas::Line {
                x1: -2.0,
                y1: 0.0,
                x2: 2.0,
                y2: 0.0,
                color: Color::DarkGray,
            });
            ctx.draw(&ratatui::widgets::canvas::Line {
                x1: 0.0,
                y1: -2.0,
                x2: 0.0,
                y2: 2.0,
                color: Color::DarkGray,
            });
            // Ball dot
            let dot_color = g_color((gx * gx + gy * gy).sqrt());
            ctx.draw(&ratatui::widgets::canvas::Circle {
                x: gx.clamp(-1.9, 1.9),
                y: gy.clamp(-1.9, 1.9),
                radius: 0.12,
                color: dot_color,
            });
        });

    frame.render_widget(canvas, area);
}

// ---------------------------------------------------------------------------
// Sparklines (time-series chart)
// ---------------------------------------------------------------------------

fn render_sparklines(frame: &mut Frame, area: Rect, history: Option<&SensorHistory>) {
    let block = Block::default()
        .title(" TIME SERIES ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    if let Some(hist) = history {
        // Show G-force X/Y/Z as a Chart
        let axes_info = [
            ("g_force_x", Color::Red),
            ("g_force_y", Color::Green),
            ("g_force_z", Color::Blue),
        ];

        let owned: Vec<(Vec<(f64, f64)>, Color)> = axes_info
            .iter()
            .map(|(key, color)| {
                let pts: Vec<(f64, f64)> = hist
                    .get(key)
                    .map(|buf| {
                        buf.iter()
                            .enumerate()
                            .map(|(i, &v)| (i as f64, v))
                            .collect()
                    })
                    .unwrap_or_default();
                (pts, *color)
            })
            .collect();

        let datasets: Vec<Dataset> = owned
            .iter()
            .zip(["Lat-X", "Fwd-Y", "Vrt-Z"])
            .map(|((pts, color), label)| {
                Dataset::default()
                    .name(label)
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(*color))
                    .data(pts)
            })
            .collect();

        let max_x = owned.iter().map(|(pts, _)| pts.len()).max().unwrap_or(1) as f64;

        let (y_min, y_max) = hist
            .stats("g_force_x")
            .map(|(min, max, _)| (min - 0.5, max + 0.5))
            .unwrap_or((-2.0, 2.0));

        let chart = Chart::new(datasets)
            .block(block)
            .x_axis(
                Axis::default()
                    .bounds([0.0, max_x])
                    .style(Style::default().fg(Color::DarkGray)),
            )
            .y_axis(
                Axis::default()
                    .bounds([y_min, y_max])
                    .labels(vec![
                        Span::raw(format!("{:.1}", y_min)),
                        Span::raw("0.0"),
                        Span::raw(format!("{:.1}", y_max)),
                    ])
                    .style(Style::default().fg(Color::DarkGray)),
            );

        frame.render_widget(chart, area);
    } else {
        let para = Paragraph::new("No history yet")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(para, area);
    }
}

// ---------------------------------------------------------------------------
// Orientation compass canvas
// ---------------------------------------------------------------------------

fn render_orientation(frame: &mut Frame, area: Rect, data: Option<&SensorData>) {
    let block = Block::default()
        .title(" ORIENTATION ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let lean = data
        .and_then(|d| d.fields.get("lean_angle"))
        .copied()
        .unwrap_or(0.0);
    let bank = data
        .and_then(|d| d.fields.get("bank_angle"))
        .copied()
        .unwrap_or(0.0);
    let _tilt = data
        .and_then(|d| d.fields.get("tilt_angle"))
        .copied()
        .unwrap_or(0.0);

    let canvas = Canvas::default()
        .block(block)
        .x_bounds([-100.0, 100.0])
        .y_bounds([-100.0, 100.0])
        .paint(move |ctx: &mut Context| {
            // Horizon circle
            ctx.draw(&ratatui::widgets::canvas::Circle {
                x: 0.0,
                y: 0.0,
                radius: 80.0,
                color: Color::DarkGray,
            });
            // Cross
            ctx.draw(&ratatui::widgets::canvas::Line {
                x1: -80.0,
                y1: 0.0,
                x2: 80.0,
                y2: 0.0,
                color: Color::DarkGray,
            });
            ctx.draw(&ratatui::widgets::canvas::Line {
                x1: 0.0,
                y1: -80.0,
                x2: 0.0,
                y2: 80.0,
                color: Color::DarkGray,
            });

            // Lean indicator (Y axis — side tilt)
            let lean_rad = lean.to_radians();
            let lx = lean_rad.sin() * 70.0;
            let ly = lean_rad.cos() * 70.0;
            ctx.draw(&ratatui::widgets::canvas::Line {
                x1: 0.0,
                y1: 0.0,
                x2: lx,
                y2: ly,
                color: Color::Yellow,
            });

            // Bank indicator (X axis)
            let bank_rad = bank.to_radians();
            let bx = bank_rad.sin() * 50.0;
            let by = -bank_rad.cos() * 50.0;
            ctx.draw(&ratatui::widgets::canvas::Line {
                x1: 0.0,
                y1: 0.0,
                x2: bx,
                y2: by,
                color: Color::Red,
            });
        });

    frame.render_widget(canvas, area);
}

// ---------------------------------------------------------------------------
// Connections tab
// ---------------------------------------------------------------------------

pub fn render_connections_tab(frame: &mut Frame, area: Rect, snap: &StateSnapshot) {
    let block = Block::default()
        .title(" CONNECTIONS ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(section_line("SENSORS"));

    for st in &snap.sensor_statuses {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<16}", st.name),
                Style::default().fg(Color::Cyan),
            ),
            status_dot(st.connected),
            Span::raw(format!("  {}", st.connection_display)),
        ]));
        if let Some(ref e) = st.last_error {
            lines.push(Line::from(Span::styled(
                format!("    Error: {}", e),
                Style::default().fg(Color::Red),
            )));
        }
    }

    lines.push(section_line("MQTT"));
    let mqtt_dot = snap.mqtt_status.is_connected();
    lines.push(Line::from(vec![
        Span::styled("  Broker          ", Style::default().fg(Color::Cyan)),
        status_dot(mqtt_dot),
        Span::raw(format!("  {}", snap.mqtt_address)),
    ]));
    lines.push(data_row(
        "  Published",
        format!("{}", snap.messages_published),
    ));

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

// ---------------------------------------------------------------------------
// Logs tab
// ---------------------------------------------------------------------------

pub fn render_logs_tab(frame: &mut Frame, area: Rect, snap: &StateSnapshot) {
    let block = Block::default()
        .title(" APPLICATION LOGS ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let height = area.height.saturating_sub(2) as usize;
    let start = snap.logs.len().saturating_sub(height);
    let items: Vec<ListItem> = snap.logs[start..]
        .iter()
        .map(|line| {
            let style = if line.contains("ERROR") {
                Style::default().fg(Color::Red)
            } else if line.contains("WARN") {
                Style::default().fg(Color::Yellow)
            } else if line.contains("INFO") {
                Style::default().fg(Color::Gray)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(Line::from(Span::styled(line.clone(), style)))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}
