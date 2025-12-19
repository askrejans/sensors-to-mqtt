//! Main entry point for the Sensors-to-MQTT system.
//!
//! This application reads sensor data (IMU, accelerometers, etc.) and publishes
//! it to an MQTT broker. It supports both interactive mode with a terminal UI
//! and daemon mode for running as a background service.

use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    Terminal,
};
use std::io;
use std::sync::Arc;
use std::time::Duration;

mod cli;
mod config;
mod error;
mod filters;
mod mqtt_handler;
mod publisher;
mod sensors;
mod service;
mod ui;

use cli::{Cli, RunMode};
use config::AppConfig;
use service::{SensorService, setup_signal_handler};
use ui::{App, InputAction, handle_input};

fn main() -> Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse_args();

    // Initialize logger
    let log_filter = cli.log_level.to_filter_string();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_filter))
        .init();

    log::info!("Starting Sensors-to-MQTT v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let mut config = AppConfig::from_file(&cli.config)?;
    config.apply_cli_overrides(&cli);

    log::info!("Configuration loaded from {:?}", cli.config);
    log::debug!("Config: {:?}", config);

    let config = Arc::new(config);

    // Run in appropriate mode
    match cli.mode {
        RunMode::Interactive => run_interactive(config, cli.no_mqtt),
        RunMode::Daemon => run_daemon(config, cli.no_mqtt),
    }
}

/// Run in interactive mode with TUI
fn run_interactive(config: Arc<AppConfig>, no_mqtt: bool) -> Result<()> {
    log::info!("Running in interactive mode");

    // Initialize service
    let mut service = SensorService::new(config.clone(), no_mqtt)?;
    let sensor_names = service.get_sensor_names();

    // Setup signal handler
    let stop_signal = service.get_stop_signal();
    setup_signal_handler(stop_signal.clone())?;

    // Initialize terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initialize app state
    let mut app = App::new(sensor_names);
    app.mqtt_connected = service.is_publisher_connected();

    // Main loop
    let update_interval = Duration::from_millis(config.service.update_interval_ms);
    let result = run_ui_loop(&mut terminal, &mut app, &mut service, update_interval);

    // Cleanup terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Main UI loop
fn run_ui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    service: &mut SensorService,
    update_interval: Duration,
) -> Result<()> {
    loop {
        // Draw UI
        terminal.draw(|frame| {
            render_ui(frame, app);
        })?;

        // Handle input
        match handle_input(Duration::from_millis(10))? {
            InputAction::Quit => {
                app.should_quit = true;
            }
            InputAction::ToggleMeasurement => {
                app.toggle_measuring();
            }
            InputAction::Reload => {
                if let Err(e) = service.reload_config() {
                    app.set_error(format!("Reload failed: {}", e));
                } else {
                    app.set_status("Configuration reloaded".to_string());
                }
            }
            InputAction::NextSensor => {
                app.next_sensor();
            }
            InputAction::PrevSensor => {
                app.prev_sensor();
            }
            InputAction::ToggleSensor => {
                app.toggle_selected_sensor();
                // Update sensor enabled state in service
                if let Some(name) = app.get_selected_sensor_name() {
                    let enabled = app.sensor_enabled.get(name).copied().unwrap_or(true);
                    if let Some(sensor) = service.get_sensor_mut(name) {
                        sensor.set_enabled(enabled);
                    }
                }
            }
            InputAction::ClearCharts => {
                app.clear_charts();
            }
            InputAction::ToggleHelp => {
                app.toggle_help();
            }
            InputAction::Calibrate => {
                if let Some(name) = app.get_selected_sensor_name() {
                    app.set_status(format!("Calibrating {} - Keep sensor still!", name));
                    if let Err(e) = service.recalibrate_sensor(name) {
                        app.set_error(format!("Calibration failed: {}", e));
                    } else {
                        app.set_status(format!("Calibration complete for {}", name));
                    }
                }
            }
            InputAction::None => {}
        }

        if app.should_quit {
            service.request_stop();
            break;
        }

        // Read and update sensor data if measuring
        if app.is_measuring {
            match service.read_sensors() {
                Ok(sensor_data) => {
                    for (name, data) in sensor_data {
                        app.update_sensor_data(&name, data.clone());
                        
                        // Publish to MQTT
                        if let Err(e) = service.publish(&name, &data) {
                            app.set_error(format!("Publish error: {}", e));
                        }
                    }
                    app.clear_error();
                }
                Err(e) => {
                    app.set_error(format!("Sensor read error: {}", e));
                }
            }

            // Update MQTT connection status
            app.mqtt_connected = service.is_publisher_connected();
        }

        std::thread::sleep(update_interval);
    }

    Ok(())
}

/// Render the UI
fn render_ui(frame: &mut ratatui::Frame, app: &App) {
    let size = frame.area();

    // Create main layout
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),      // Main content
            Constraint::Length(3),   // Status bar
        ])
        .split(size);

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25), // Sensor list
            Constraint::Percentage(75), // Charts and data
        ])
        .split(main_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50), // G-meter
            Constraint::Percentage(50), // Chart
        ])
        .split(content_chunks[1]);

    // Render sensor list
    ui::widgets::render_sensor_list(
        frame,
        content_chunks[0],
        &app.sensor_names,
        &app.sensor_enabled,
        app.selected_sensor,
    );

    // Render G-meter and chart for selected sensor
    if let Some(sensor_name) = app.get_selected_sensor_name() {
        let sensor_data = app.current_data.get(sensor_name);
        let sensor_history = app.sensor_history.get(sensor_name);

        ui::widgets::render_g_meter(frame, right_chunks[0], sensor_data, sensor_name);
        ui::widgets::render_chart(frame, right_chunks[1], sensor_history, sensor_name);
    }

    // Render status bar
    ui::widgets::render_status_bar(
        frame,
        main_chunks[1],
        app.is_measuring,
        app.mqtt_connected,
        app.status_message.as_deref(),
        app.error_message.as_deref(),
    );

    // Render help overlay if needed
    if app.show_help {
        let help_area = centered_rect(60, 80, size);
        ui::widgets::render_help(frame, help_area);
    }
}

/// Helper to create a centered rectangle
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Run in daemon mode (no UI)
fn run_daemon(config: Arc<AppConfig>, no_mqtt: bool) -> Result<()> {
    log::info!("Running in daemon mode");

    // Initialize service
    let mut service = SensorService::new(config, no_mqtt)?;

    // Setup signal handler
    let stop_signal = service.get_stop_signal();
    setup_signal_handler(stop_signal)?;

    // Run the service
    service.run_daemon()?;

    log::info!("Daemon mode exited");
    Ok(())
}
