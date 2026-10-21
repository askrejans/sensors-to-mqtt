//! sensors-to-mqtt — main entry point.
//!
//! TTY detection:
//!   - stdout is a terminal → TUI mode (interactive)
//!   - stdout is piped / systemd → daemon mode (structured logs to stdout)

use anyhow::Result;
use clap::Parser;
use std::collections::VecDeque;
use std::io::IsTerminal;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use sensors_to_mqtt::config::{
    AppConfig, ConnectionConfig, DRIVERS, I2cConnectionConfig, SensorConfig, load_configuration,
};
use sensors_to_mqtt::models::{Info, Shared};
use sensors_to_mqtt::{output, scan, service, tui};

/// Sensors (I2C, GPIO, serial, TCP bridge) to MQTT, HTTP and a web dashboard.
#[derive(Debug, Parser)]
#[command(version, about, after_help = drivers_help())]
struct Cli {
    /// Path to a TOML config file (default: ./config.toml, ./settings.toml, /etc/sensors-to-mqtt/)
    #[arg(short, long, value_name = "FILE")]
    config: Option<String>,

    /// Validate the configuration, print the effective settings and exit
    #[arg(long)]
    check_config: bool,

    /// Find the chips on a local I2C bus, suggest their configuration and exit
    #[arg(long)]
    scan: bool,

    /// I2C bus for --scan
    #[arg(
        long,
        value_name = "DEVICE",
        default_value = "/dev/i2c-1",
        requires = "scan"
    )]
    i2c_device: String,

    /// Replace the configured sensors with one synthetic sensor (no hardware needed)
    #[arg(long)]
    simulate: bool,

    /// Do not publish to MQTT
    #[arg(long)]
    no_mqtt: bool,

    /// Never start the terminal dashboard, even on a terminal
    #[arg(long)]
    headless: bool,

    /// Log level: trace, debug, info, warn or error
    #[arg(long, value_name = "LEVEL")]
    log_level: Option<String>,

    /// Query the local HTTP /health endpoint and exit 0 (healthy) or 1
    #[arg(long)]
    healthcheck: bool,
}

/// `--healthcheck`: minimal HTTP/1.0 GET so container images need no curl.
fn healthcheck(config: &AppConfig) -> Result<bool> {
    use std::io::{Read, Write};
    anyhow::ensure!(
        config.http.enabled,
        "http.enabled is false; nothing to probe"
    );
    let host = match config.http.bind.as_str() {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        h => h,
    };
    let addr: std::net::SocketAddr = format!("{host}:{}", config.http.port).parse()?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    let mut response = String::new();
    stream.take(4096).read_to_string(&mut response)?;
    println!("{}", response.lines().last().unwrap_or_default());
    Ok(response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200"))
}

fn drivers_help() -> String {
    format!(
        "Drivers: {}\n\nSettings can be overridden with SENSORS_TO_MQTT_<KEY>, e.g. SENSORS_TO_MQTT_MQTT_HOST, \
         SENSORS_TO_MQTT_HTTP_PORT, SENSORS_TO_MQTT_LOG_LEVEL.",
        DRIVERS.join(", ")
    )
}

fn init_logging(config: &AppConfig, writer: Option<tui::TuiWriter>) {
    let filter = EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    match writer {
        Some(w) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(w)
            .without_time()
            .init(),
        None if config.log_json => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init(),
        None => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(std::io::stdout().is_terminal())
            .init(),
    }
}

fn simulated_sensor() -> SensorConfig {
    SensorConfig {
        name: "sim".into(),
        enabled: true,
        driver: "synthetic".into(),
        connection: ConnectionConfig::I2c(I2cConnectionConfig {
            device: "simulated".into(),
            address: 0,
        }),
        settings: None,
        channels: None,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.scan {
        if let Err(e) = scan::run(&cli.i2c_device) {
            eprintln!("Scan failed: {e:#}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let mut config = match load_configuration(cli.config.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration error: {e:#}");
            std::process::exit(2);
        }
    };
    if cli.simulate {
        config.sensors = vec![simulated_sensor()];
    }
    if cli.no_mqtt {
        config.mqtt.enabled = false;
    }
    if let Some(level) = &cli.log_level {
        config.log_level = level.clone();
    }
    if let Err(e) = config.validate() {
        eprintln!("Configuration error: {e:#}");
        std::process::exit(2);
    }

    if cli.healthcheck {
        let healthy = healthcheck(&config).unwrap_or_else(|e| {
            eprintln!("healthcheck failed: {e:#}");
            false
        });
        std::process::exit(if healthy { 0 } else { 1 });
    }

    if cli.check_config {
        println!("{}", serde_json::to_string_pretty(&config.redacted())?);
        println!(
            "\nConfiguration OK ({})",
            config
                .config_path
                .as_deref()
                .unwrap_or("defaults + environment")
        );
        return Ok(());
    }

    let interactive = !cli.headless && std::io::stdout().is_terminal();
    let log_buf = Arc::new(Mutex::new(VecDeque::with_capacity(1000)));
    init_logging(
        &config,
        interactive.then(|| tui::TuiWriter::new(log_buf.clone())),
    );

    info!("sensors-to-mqtt v{} starting", env!("CARGO_PKG_VERSION"));
    match &config.config_path {
        Some(p) => info!("Configuration: {p}"),
        None => warn!("No configuration file found; using defaults and environment"),
    }
    if config.sensors.iter().all(|s| !s.enabled) {
        warn!(
            "No sensor is enabled; add [[sensors]] to the configuration, run --scan, or try --simulate"
        );
    }

    let outputs = output::describe(&config);
    for o in &outputs {
        info!("Output: {o}");
    }
    let shared = Shared::new(
        Info {
            version: env!("CARGO_PKG_VERSION"),
            mqtt: config.mqtt.enabled.then(|| config.mqtt.address()),
            outputs,
        },
        config.mqtt.enabled,
    );
    service::register_sensors(&mut shared.write(), &config.sensors);

    let config = Arc::new(config);
    let cancel = CancellationToken::new();
    let mut tasks = JoinSet::new();
    output::spawn_all(&config, &shared, &cancel, &mut tasks);
    let (hub, threads) = service::start(&shared, &config.sensors, &cancel);

    {
        let c = cancel.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            c.cancel();
        });
    }

    if interactive {
        let refresh = Duration::from_millis(config.tui_refresh_rate_ms.clamp(20, 2000));
        if let Err(e) = tui::run_tui(shared.clone(), log_buf, cancel.clone(), refresh).await {
            eprintln!("TUI error: {e:#}");
        }
        cancel.cancel();
    } else {
        info!("Running headless");
        cancel.cancelled().await;
    }

    info!("Shutting down");
    let _ = tokio::time::timeout(Duration::from_secs(3), async {
        while tasks.join_next().await.is_some() {}
        let _ = hub.await;
        // sensor threads notice the cancel within 50 ms, or after the read in progress
        let _ = tokio::task::spawn_blocking(move || {
            for t in threads {
                let _ = t.join();
            }
        })
        .await;
    })
    .await;
    info!("Shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => info!("Received Ctrl+C"),
            _ = term.recv() => info!("Received SIGTERM"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        info!("Received Ctrl+C");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
        let cli = Cli::parse_from([
            "sensors-to-mqtt",
            "-c",
            "x.toml",
            "--simulate",
            "--headless",
            "--no-mqtt",
        ]);
        assert_eq!(cli.config.as_deref(), Some("x.toml"));
        assert!(cli.simulate && cli.headless && cli.no_mqtt);
        assert!(
            Cli::try_parse_from(["sensors-to-mqtt", "--i2c-device", "/dev/i2c-0"]).is_err(),
            "only with --scan"
        );
        let cli = Cli::parse_from(["sensors-to-mqtt", "--scan"]);
        assert_eq!(cli.i2c_device, "/dev/i2c-1");
        assert!(drivers_help().contains("mpu6500"));
    }

    #[test]
    fn simulated_sensor_is_valid() {
        let c = AppConfig {
            sensors: vec![simulated_sensor()],
            ..AppConfig::default()
        };
        c.validate().unwrap();
    }
}
