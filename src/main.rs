//! sensors-to-mqtt — main entry point.
//!
//! TTY detection:
//!   - stdout is a terminal → TUI mode (interactive)
//!   - stdout is piped / systemd → daemon mode (structured logs to stdout)

use anyhow::Result;
use gumdrop::Options as _;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use sensors_to_mqtt::config::load_configuration;
use sensors_to_mqtt::models::{AppState, SharedState};
use sensors_to_mqtt::service::{register_sensors, spawn_sensor_task};
use sensors_to_mqtt::{mqtt_handler, tui};

// ---------------------------------------------------------------------------
// CLI  (gumdrop)
// ---------------------------------------------------------------------------

#[derive(Debug, gumdrop::Options)]
struct Opts {
    #[options(help = "print help")]
    help: bool,

    #[options(short = "c", help = "path to config file (default: config.toml)")]
    config: Option<String>,

    #[options(long = "no-mqtt", help = "disable MQTT publishing")]
    no_mqtt: bool,

    #[options(long = "log-level", help = "log level: trace|debug|info|warn|error")]
    log_level: Option<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let opts = Opts::parse_args_default_or_exit();

    let cfg = load_configuration(opts.config.as_deref())?;

    let is_tty = atty::is(atty::Stream::Stdout);

    // Log buffer shared with TUI writer
    let log_buf: Arc<std::sync::Mutex<VecDeque<String>>> =
        Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(1000)));

    // Initialise logging
    let log_level = opts
        .log_level
        .as_deref()
        .unwrap_or(cfg.log_level.as_str())
        .to_string();

    if is_tty {
        let tui_writer = tui::TuiWriter::new(Arc::clone(&log_buf));
        let filter = EnvFilter::try_new(&log_level).unwrap_or_else(|_| EnvFilter::new("info"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(tui_writer)
            .without_time()
            .init();
    } else {
        let filter = EnvFilter::try_new(&log_level).unwrap_or_else(|_| EnvFilter::new("info"));
        if cfg.log_json {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .init();
        } else {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }

    tracing::info!("Starting sensors-to-mqtt v{}", env!("CARGO_PKG_VERSION"));

    let cancel = CancellationToken::new();

    // MQTT
    let mqtt_handle = if cfg.mqtt.enabled && !opts.no_mqtt {
        Some(mqtt_handler::spawn_mqtt_task(&cfg.mqtt))
    } else {
        None
    };

    let mqtt_address = if cfg.mqtt.enabled && !opts.no_mqtt {
        cfg.mqtt.address()
    } else {
        "disabled".to_string()
    };

    // Shared application state
    let state: SharedState = Arc::new(RwLock::new(AppState::new(
        mqtt_address,
        cfg.mqtt.enabled && !opts.no_mqtt,
        1000,
    )));

    // Register all configured sensors in the state map
    {
        let mut s = state.write().await;
        register_sensors(&mut s, &cfg.sensors);
        if let Some(ref h) = mqtt_handle {
            s.messages_published = Arc::clone(&h.counter);
            // Share the exact same Arc so TUI always reflects live MQTT state
            s.mqtt_status = Arc::clone(&h.status);
        }
    }

    // Spawn a task per enabled sensor
    for sensor_cfg in &cfg.sensors {
        if !sensor_cfg.enabled {
            tracing::info!("Sensor '{}' is disabled, skipping", sensor_cfg.name);
            continue;
        }
        spawn_sensor_task(
            sensor_cfg.clone(),
            Arc::clone(&state),
            mqtt_handle.clone(),
            cancel.clone(),
            cfg.mqtt.base_topic.clone(),
        );
    }

    // Signal handler
    {
        let c = cancel.clone();
        tokio::spawn(async move {
            let mut signals = signal_hook_tokio::Signals::new(&[
                signal_hook::consts::SIGTERM,
                signal_hook::consts::SIGINT,
            ])
            .expect("signal handler");
            use futures_util::StreamExt;
            if signals.next().await.is_some() {
                tracing::info!("Shutdown signal received");
                c.cancel();
            }
        });
    }

    // Run TUI or wait for cancel
    if is_tty {
        tui::run_tui(Arc::clone(&state), Arc::clone(&log_buf), cancel.clone()).await?;
        cancel.cancel();
    } else {
        tracing::info!("Running in daemon mode (no TUI)");
        cancel.cancelled().await;
    }

    tracing::info!("Shutting down");
    Ok(())
}
