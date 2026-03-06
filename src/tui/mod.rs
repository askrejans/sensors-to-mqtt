//! TUI entry point.
//!
//! Layout (always):
//!   header  (3 lines)
//!   tabs    (1 line)
//!   main    (Min 8) — content switches per selected tab
//!   log     (8 lines) — always visible, shows recent tracing log lines

pub mod tabs;
pub mod widgets;

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::Arc;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Tabs;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt::MakeWriter;

use crate::models::SharedState;

// ---------------------------------------------------------------------------
// TuiWriter — feeds tracing output into the log ring-buffer
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TuiWriter {
    buf: Arc<std::sync::Mutex<VecDeque<String>>>,
}

impl TuiWriter {
    pub fn new(buf: Arc<std::sync::Mutex<VecDeque<String>>>) -> Self {
        Self { buf }
    }
}

impl Write for TuiWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Ok(s) = std::str::from_utf8(data) {
            let line = s.trim_end_matches('\n').to_string();
            if !line.is_empty() {
                let mut g = self.buf.lock().unwrap();
                if g.len() >= 1000 {
                    g.pop_front();
                }
                g.push_back(line);
            }
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for TuiWriter {
    type Writer = TuiWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// TUI loop
// ---------------------------------------------------------------------------

pub async fn run_tui(
    state: SharedState,
    log_buf: Arc<std::sync::Mutex<VecDeque<String>>>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = tui_loop(&mut terminal, state, log_buf, cancel).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: SharedState,
    log_buf: Arc<std::sync::Mutex<VecDeque<String>>>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let refresh = std::time::Duration::from_millis(100);

    loop {
        // Take a snapshot to avoid holding the lock during rendering
        let snap = {
            let s = state.read().await;
            tabs::StateSnapshot::from(&*s, &log_buf)
        };

        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(1),
                    Constraint::Min(8),
                    Constraint::Length(8),
                ])
                .split(area);

            widgets::render_header(frame, chunks[0], snap.version.as_str());

            // Build tab titles
            let mut tab_titles: Vec<String> = snap
                .sensor_names
                .iter()
                .enumerate()
                .map(|(i, n)| format!(" {} ({}) ", n, i + 1))
                .collect();
            tab_titles.push(format!(" Connections ({}) ", snap.sensor_names.len() + 1));
            tab_titles.push(format!(" Logs ({}) ", snap.sensor_names.len() + 2));

            let tabs_widget = Tabs::new(tab_titles.iter().map(|s| s.as_str()).collect::<Vec<_>>())
                .select(snap.selected_tab)
                .style(Style::default().fg(Color::White))
                .highlight_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
                .divider(Span::raw("|"));
            frame.render_widget(tabs_widget, chunks[1]);

            // Main content
            let n_sensors = snap.sensor_names.len();
            if snap.selected_tab < n_sensors {
                tabs::render_sensor_tab(frame, chunks[2], &snap, snap.selected_tab);
            } else if snap.selected_tab == n_sensors {
                tabs::render_connections_tab(frame, chunks[2], &snap);
            } else {
                tabs::render_logs_tab(frame, chunks[2], &snap);
            }

            // Log panel (always visible)
            widgets::render_log_panel(frame, chunks[3], &snap.logs);
        })?;

        // Input with timeout
        if event::poll(refresh)? {
            if let Event::Key(key) = event::read()? {
                let n_sensors = {
                    let s = state.read().await;
                    s.sensor_names().len()
                };
                let tab_count = n_sensors + 2;

                let mut s = state.write().await;
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => {
                        break;
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        break;
                    }
                    (KeyCode::Left, _) => {
                        if s.selected_tab > 0 {
                            s.selected_tab -= 1;
                        }
                    }
                    (KeyCode::Right, _) => {
                        if s.selected_tab + 1 < tab_count {
                            s.selected_tab += 1;
                        }
                    }
                    (KeyCode::Char(c), _) if c.is_ascii_digit() => {
                        let idx = (c as usize).wrapping_sub('1' as usize);
                        if idx < tab_count {
                            s.selected_tab = idx;
                        }
                    }
                    _ => {}
                }
            }
        }

        if cancel.is_cancelled() {
            break;
        }
    }

    Ok(())
}
