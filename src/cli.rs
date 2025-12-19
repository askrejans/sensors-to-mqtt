//! Command-line interface argument parsing.
//!
//! This module defines the CLI structure and parsing logic using clap,
//! supporting different run modes and configuration options.

use clap::{Parser, ValueEnum};
use std::path::PathBuf;

/// Run mode for the application
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunMode {
    /// Interactive terminal UI mode (default)
    Interactive,
    /// Background daemon mode (no UI, logs only)
    Daemon,
}

/// Log level for the application
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogLevel {
    /// Show all messages including trace
    Trace,
    /// Show debug messages and above
    Debug,
    /// Show info messages and above (default)
    Info,
    /// Show warnings and errors only
    Warn,
    /// Show errors only
    Error,
}

impl LogLevel {
    /// Convert LogLevel to env_logger filter string
    pub fn to_filter_string(&self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// Sensors-to-MQTT: Read sensor data and publish to MQTT broker
#[derive(Parser, Debug)]
#[command(name = "sensors-to-mqtt")]
#[command(author = "Sensors-to-MQTT Contributors")]
#[command(version)]
#[command(about = "Reads sensor data and publishes to MQTT broker", long_about = None)]
pub struct Cli {
    /// Run mode (interactive UI or background daemon)
    #[arg(short, long, value_enum, default_value = "interactive")]
    pub mode: RunMode,

    /// Path to configuration file
    #[arg(short, long, default_value = "config.yaml")]
    pub config: PathBuf,

    /// Log level
    #[arg(short, long, value_enum, default_value = "info")]
    pub log_level: LogLevel,

    /// Update interval in milliseconds (overrides config file)
    #[arg(short = 'i', long)]
    pub update_interval_ms: Option<u64>,

    /// Disable MQTT publishing (sensor reading only)
    #[arg(long)]
    pub no_mqtt: bool,

    /// MQTT broker host (overrides config file)
    #[arg(long)]
    pub mqtt_host: Option<String>,

    /// MQTT broker port (overrides config file)
    #[arg(long)]
    pub mqtt_port: Option<u16>,
}

impl Cli {
    /// Parse command-line arguments
    pub fn parse_args() -> Self {
        Self::parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_conversion() {
        assert_eq!(LogLevel::Info.to_filter_string(), "info");
        assert_eq!(LogLevel::Debug.to_filter_string(), "debug");
        assert_eq!(LogLevel::Error.to_filter_string(), "error");
    }

    #[test]
    fn test_default_values() {
        let cli = Cli::parse_from(&["sensors-to-mqtt"]);
        assert_eq!(cli.mode, RunMode::Interactive);
        assert_eq!(cli.config, PathBuf::from("config.yaml"));
        assert_eq!(cli.log_level, LogLevel::Info);
        assert_eq!(cli.no_mqtt, false);
    }
}
