//! Application configuration management.
//!
//! This module handles loading, parsing, and validating the application configuration
//! from YAML files with support for runtime overrides from CLI arguments.

use serde::Deserialize;
use std::path::Path;
use crate::error::{ConfigError, Result};

/// Top-level application configuration
#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub service: ServiceConfig,
    pub logging: LoggingConfig,
    pub mqtt: MqttConfig,
}

/// Service runtime configuration
#[derive(Debug, Deserialize, Clone)]
pub struct ServiceConfig {
    #[serde(default = "default_run_mode")]
    pub run_mode: String,
    #[serde(default = "default_update_interval")]
    pub update_interval_ms: u64,
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
    #[serde(default)]
    pub max_reconnect_attempts: u32,
    #[serde(default = "default_reconnect_delay")]
    pub reconnect_delay_ms: u64,
    #[serde(default = "default_max_reconnect_delay")]
    pub max_reconnect_delay_ms: u64,
}

/// Logging configuration
#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default = "default_true")]
    pub colored: bool,
}

/// MQTT broker configuration
#[derive(Debug, Deserialize, Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub base_topic: String,
    #[serde(default = "default_client_id")]
    pub client_id: String,
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
    #[serde(default = "default_true")]
    pub clean_session: bool,
    #[serde(default = "default_qos")]
    pub qos: i32,
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Filter configuration for Kalman filters
#[derive(Debug, Deserialize, Clone)]
pub struct FilterConfig {
    pub process_noise: f64,
    pub measurement_noise: f64,
    pub dead_zone: f64,
}

// Default value functions
fn default_run_mode() -> String {
    "interactive".to_string()
}

fn default_update_interval() -> u64 {
    10
}

fn default_true() -> bool {
    true
}

fn default_reconnect_delay() -> u64 {
    1000
}

fn default_max_reconnect_delay() -> u64 {
    60000
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_client_id() -> String {
    "sensors-to-mqtt".to_string()
}

fn default_keep_alive() -> u64 {
    20
}

fn default_qos() -> i32 {
    1
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            run_mode: default_run_mode(),
            update_interval_ms: default_update_interval(),
            auto_reconnect: true,
            max_reconnect_attempts: 0,
            reconnect_delay_ms: default_reconnect_delay(),
            max_reconnect_delay_ms: default_max_reconnect_delay(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            file: None,
            colored: true,
        }
    }
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 1883,
            base_topic: "/GOLF86/SENSORS".to_string(),
            client_id: default_client_id(),
            keep_alive_secs: default_keep_alive(),
            clean_session: true,
            qos: default_qos(),
            username: None,
            password: None,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            service: ServiceConfig::default(),
            logging: LoggingConfig::default(),
            mqtt: MqttConfig::default(),
        }
    }
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            process_noise: 0.00001,
            measurement_noise: 0.05,
            dead_zone: 0.005,
        }
    }
}

impl AppConfig {
    /// Load configuration from a YAML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| ConfigError::ReadError(e))?;
        
        let config: AppConfig = serde_yaml_ng::from_str(&content)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration values
    pub fn validate(&self) -> Result<()> {
        // Validate service config
        if self.service.update_interval_ms == 0 {
            return Err(ConfigError::InvalidValue {
                field: "service.update_interval_ms".to_string(),
                message: "must be greater than 0".to_string(),
            }.into());
        }

        // Validate MQTT config
        if self.mqtt.port == 0 {
            return Err(ConfigError::InvalidValue {
                field: "mqtt.port".to_string(),
                message: "must be greater than 0".to_string(),
            }.into());
        }

        if self.mqtt.qos < 0 || self.mqtt.qos > 2 {
            return Err(ConfigError::InvalidValue {
                field: "mqtt.qos".to_string(),
                message: "must be 0, 1, or 2".to_string(),
            }.into());
        }

        if self.mqtt.base_topic.is_empty() {
            return Err(ConfigError::InvalidValue {
                field: "mqtt.base_topic".to_string(),
                message: "cannot be empty".to_string(),
            }.into());
        }

        Ok(())
    }

    /// Apply CLI argument overrides to configuration
    pub fn apply_cli_overrides(&mut self, cli: &crate::cli::Cli) {
        if let Some(interval) = cli.update_interval_ms {
            self.service.update_interval_ms = interval;
        }

        if let Some(host) = &cli.mqtt_host {
            self.mqtt.host = host.clone();
        }

        if let Some(port) = cli.mqtt_port {
            self.mqtt.port = port;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.mqtt.host, "localhost");
        assert_eq!(config.mqtt.port, 1883);
        assert_eq!(config.service.update_interval_ms, 10);
    }

    #[test]
    fn test_validate_invalid_qos() {
        let mut config = AppConfig::default();
        config.mqtt.qos = 3;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_zero_update_interval() {
        let mut config = AppConfig::default();
        config.service.update_interval_ms = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_base_topic() {
        let mut config = AppConfig::default();
        config.mqtt.base_topic = String::new();
        assert!(config.validate().is_err());
    }
}
