//! Configuration management for sensors-to-mqtt.
//!
//! Load priority (highest → lowest):
//!   1. `SENSORS_TO_MQTT__*` environment variables (double-underscore separator)
//!   2. Explicitly specified file (--config CLI flag)
//!   3. Default search paths: ./config.toml, /etc/sensors-to-mqtt/config.toml
//!   4. Built-in defaults

use anyhow::{Context, Result};
// Use absolute path to avoid ambiguity with the local `config` module name.
use ::config::{Config, Environment, File};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Top-level
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct AppConfig {
    pub log_level: String,
    pub log_json: bool,
    pub tui_refresh_rate_ms: u64,
    pub mqtt: MqttConfig,
    pub sensors: Vec<SensorConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            log_json: false,
            tui_refresh_rate_ms: 100,
            mqtt: MqttConfig::default(),
            sensors: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// MQTT
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct MqttConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub base_topic: String,
    pub client_id: String,
    pub keep_alive_secs: u64,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: "localhost".to_string(),
            port: 1883,
            base_topic: "/SENSORS".to_string(),
            client_id: "sensors-to-mqtt".to_string(),
            keep_alive_secs: 20,
            username: None,
            password: None,
        }
    }
}

impl MqttConfig {
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

// ---------------------------------------------------------------------------
// Sensors
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct SensorConfig {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub driver: String,
    pub connection: ConnectionConfig,
    /// Driver-specific settings stored as raw TOML so each driver can
    /// deserialize what it needs via `settings.clone().try_into()`.
    pub settings: Option<toml::Value>,
}

fn default_true() -> bool {
    true
}

/// Connection type — discriminated by the `type` field in TOML.
///
/// Example:
/// ```toml
/// [sensors.connection]
/// type = "i2c"
/// device = "/dev/i2c-1"
/// address = 0x68
/// ```
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ConnectionConfig {
    I2c(I2cConnectionConfig),
    Serial(SerialConnectionConfig),
    Tcp(TcpConnectionConfig),
    Gpio(GpioConnectionConfig),
}

#[derive(Debug, Deserialize, Clone)]
pub struct I2cConnectionConfig {
    #[serde(default = "default_i2c_device")]
    pub device: String,
    pub address: u16,
}

fn default_i2c_device() -> String {
    "/dev/i2c-1".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct SerialConnectionConfig {
    pub port: String,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
}

fn default_baud_rate() -> u32 {
    9600
}

#[derive(Debug, Deserialize, Clone)]
pub struct TcpConnectionConfig {
    pub host: String,
    #[serde(default = "default_tcp_port")]
    pub port: u16,
}

fn default_tcp_port() -> u16 {
    23
}

#[derive(Debug, Deserialize, Clone)]
pub struct GpioConnectionConfig {
    /// BCM GPIO pin number
    pub pin: u32,
    /// True = LOW means active/pressed
    #[serde(default)]
    pub active_low: bool,
    /// Software debounce window in milliseconds
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
}

fn default_debounce_ms() -> u64 {
    50
}

impl ConnectionConfig {
    /// Human-readable description for display in the TUI.
    pub fn to_display(&self) -> String {
        match self {
            ConnectionConfig::I2c(c) => format!("I2C {} @ {:#04x}", c.device, c.address),
            ConnectionConfig::Serial(c) => format!("Serial {} @ {} baud", c.port, c.baud_rate),
            ConnectionConfig::Tcp(c) => format!("TCP {}:{}", c.host, c.port),
            ConnectionConfig::Gpio(c) => format!(
                "GPIO pin {}{}",
                c.pin,
                if c.active_low { " (active-low)" } else { "" }
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Load and merge configuration from a TOML file and environment variables.
///
/// Environment variable format: `SENSORS_TO_MQTT__MQTT__HOST=broker.local`
/// (double-underscore maps to each level of TOML nesting).
pub fn load_configuration(path: Option<&str>) -> Result<AppConfig> {
    let mut builder = Config::builder();

    if let Some(p) = path {
        builder = builder.add_source(File::with_name(p).required(true));
    } else {
        builder = builder
            .add_source(File::with_name("settings.toml").required(false))
            .add_source(File::with_name("config.toml").required(false))
            .add_source(File::with_name("/etc/sensors-to-mqtt/config.toml").required(false));
    }

    builder = builder.add_source(
        Environment::with_prefix("SENSORS_TO_MQTT")
            .separator("__")
            .try_parsing(true),
    );

    let cfg: AppConfig = builder
        .build()
        .context("Failed to build configuration")?
        .try_deserialize()
        .context("Failed to deserialize configuration")?;

    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Default values ---

    #[test]
    fn test_default_mqtt_config() {
        let cfg = MqttConfig::default();
        assert_eq!(cfg.port, 1883);
        assert_eq!(cfg.host, "localhost");
        assert!(cfg.enabled);
        assert_eq!(cfg.keep_alive_secs, 20);
    }

    #[test]
    fn test_default_app_config() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.log_level, "info");
        assert!(!cfg.log_json);
        assert_eq!(cfg.tui_refresh_rate_ms, 100);
        assert!(cfg.sensors.is_empty());
    }

    // --- address() helper ---

    #[test]
    fn test_mqtt_address() {
        let mut cfg = MqttConfig::default();
        cfg.host = "broker.local".into();
        cfg.port = 8883;
        assert_eq!(cfg.address(), "broker.local:8883");
    }

    // --- ConnectionConfig to_display ---

    #[test]
    fn test_i2c_display() {
        let c = ConnectionConfig::I2c(I2cConnectionConfig {
            device: "/dev/i2c-1".into(),
            address: 0x68,
        });
        assert!(c.to_display().contains("0x68"));
        assert!(c.to_display().contains("i2c-1"));
    }

    #[test]
    fn test_serial_display() {
        let c = ConnectionConfig::Serial(SerialConnectionConfig {
            port: "/dev/ttyS0".into(),
            baud_rate: 115200,
        });
        assert!(c.to_display().contains("115200"));
    }

    #[test]
    fn test_tcp_display() {
        let c = ConnectionConfig::Tcp(TcpConnectionConfig {
            host: "192.168.1.100".into(),
            port: 3000,
        });
        assert!(c.to_display().contains("3000"));
    }

    #[test]
    fn test_gpio_display_active_low() {
        let c = ConnectionConfig::Gpio(GpioConnectionConfig {
            pin: 17,
            active_low: true,
            debounce_ms: 50,
        });
        assert!(c.to_display().contains("17"));
        assert!(c.to_display().contains("active-low"));
    }

    // --- TOML deserialization ---

    #[test]
    fn test_load_minimal_toml() {
        // Write a temp file and load it
        use std::io::Write;
        let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        write!(
            f,
            r#"
[mqtt]
host = "test-broker"
port = 1884
base_topic = "/TEST"
"#
        )
        .unwrap();
        let cfg = load_configuration(Some(f.path().to_str().unwrap())).unwrap();
        assert_eq!(cfg.mqtt.host, "test-broker");
        assert_eq!(cfg.mqtt.port, 1884);
    }

    #[test]
    fn test_load_sensor_toml() {
        use std::io::Write;
        let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        write!(
            f,
            r#"
[[sensors]]
name    = "Test IMU"
driver  = "synthetic"
enabled = true

[sensors.connection]
type = "i2c"
device  = "/dev/i2c-1"
address = 0x68
"#
        )
        .unwrap();
        let cfg = load_configuration(Some(f.path().to_str().unwrap())).unwrap();
        assert_eq!(cfg.sensors.len(), 1);
        assert_eq!(cfg.sensors[0].name, "Test IMU");
        assert_eq!(cfg.sensors[0].driver, "synthetic");
        assert!(cfg.sensors[0].enabled);
    }

    #[test]
    fn test_load_gpio_sensor_toml() {
        use std::io::Write;
        let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        write!(
            f,
            r#"
[[sensors]]
name   = "Brake Switch"
driver = "gpio_button"

[sensors.connection]
type       = "gpio"
pin        = 17
active_low = false
"#
        )
        .unwrap();
        let cfg = load_configuration(Some(f.path().to_str().unwrap())).unwrap();
        let sensor = &cfg.sensors[0];
        match &sensor.connection {
            ConnectionConfig::Gpio(g) => {
                assert_eq!(g.pin, 17);
                assert!(!g.active_low);
            }
            other => panic!("expected Gpio, got {:?}", other),
        }
    }

    #[test]
    fn test_load_nonexistent_required_file_fails() {
        let result = load_configuration(Some("/nonexistent/path/config.toml"));
        assert!(result.is_err());
    }
}
