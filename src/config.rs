//! Configuration management for sensors-to-mqtt.
//!
//! Load priority (highest → lowest):
//!   1. Environment: `SENSORS_TO_MQTT_MQTT_HOST`, `SENSORS_TO_MQTT_HTTP_PORT`,
//!      `SENSORS_TO_MQTT_LOG_LEVEL` … (one underscore, like the sibling
//!      gateways), or the nested form `SENSORS_TO_MQTT__MQTT__HOST`
//!   2. Explicitly specified file (`--config`)
//!   3. Default search paths: ./settings.toml, ./config.toml,
//!      /etc/sensors-to-mqtt/settings.toml, /etc/sensors-to-mqtt/config.toml
//!   4. Built-in defaults

use anyhow::{Context, Result, bail};
// Use absolute path to avoid ambiguity with the local `config` module name.
use ::config::{Config, Environment, File};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

pub const ENV_PREFIX: &str = "SENSORS_TO_MQTT";

// ---------------------------------------------------------------------------
// Top-level
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct AppConfig {
    pub log_level: String,
    pub log_json: bool,
    pub tui_refresh_rate_ms: u64,
    /// G86 car slug: without an explicit `mqtt.base_topic` the base becomes
    /// `/g86/<car>/sensors`. Unset uses the generic default (`/telemetry/sensors`).
    pub car: Option<String>,
    pub mqtt: MqttConfig,
    pub http: HttpConfig,
    pub sensors: Vec<SensorConfig>,
    /// File the configuration was loaded from (not a setting).
    #[serde(skip)]
    pub config_path: Option<String>,
    /// How the base topic was chosen (not a setting).
    #[serde(skip)]
    pub base_topic_origin: BaseTopicOrigin,
}

/// Where `mqtt.base_topic` came from, for the startup log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BaseTopicOrigin {
    /// Set in the file or the environment.
    Explicit,
    /// Derived from `car`: `/g86/<car>/sensors`.
    Car,
    /// Neither: the generic default (`/telemetry/sensors`).
    #[default]
    Default,
}

/// A G86 car slug: `^[a-z0-9][a-z0-9-]{0,31}$`.
pub fn valid_car(car: &str) -> bool {
    let b = car.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            log_json: false,
            tui_refresh_rate_ms: 100,
            car: None,
            mqtt: MqttConfig::default(),
            http: HttpConfig::default(),
            sensors: Vec::new(),
            config_path: None,
            base_topic_origin: BaseTopicOrigin::Default,
        }
    }
}

// ---------------------------------------------------------------------------
// MQTT
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MqttFormat {
    /// `<base>/<sensor>/telemetry`: the G86 canonical envelope, every reading
    Canonical,
    /// `<base>/<sensor>/state`: every field of one sensor (retained)
    Json,
    /// `<base>/IMU/<sensor>/{INFO,FILTERED,DERIVED}`: the topics of 0.5 and
    /// earlier, for consumers not yet moved to the canonical envelope
    Legacy,
    /// Home Assistant MQTT discovery for every channel
    Homeassistant,
}

fn default_mqtt_formats() -> Vec<MqttFormat> {
    vec![MqttFormat::Canonical, MqttFormat::Json]
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrList {
    One(String),
    Many(Vec<String>),
}

/// `["canonical", "json"]` in TOML, `canonical,json` in the environment.
fn list_formats<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<MqttFormat>, D::Error> {
    let items = match StringOrList::deserialize(d)? {
        StringOrList::One(s) => s
            .split([',', ';', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        StringOrList::Many(v) => v,
    };
    items
        .into_iter()
        .map(|s| {
            MqttFormat::deserialize(serde::de::value::StringDeserializer::<D::Error>::new(
                s.to_lowercase(),
            ))
        })
        .collect()
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct MqttConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub base_topic: String,
    /// Auto-generated (unique per process) when unset.
    pub client_id: Option<String>,
    pub keep_alive_secs: u64,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Read the password from a file (Docker/systemd secrets).
    pub password_file: Option<String>,
    /// QoS of live data (0, 1 or 2). Status and discovery always use 1.
    pub qos: u8,
    pub use_tls: bool,
    pub ca_cert_path: Option<String>,
    pub client_cert_path: Option<String>,
    pub client_key_path: Option<String>,
    #[serde(deserialize_with = "list_formats")]
    pub formats: Vec<MqttFormat>,
    /// Cap for the json and legacy topics, per sensor (Hz); 0 = every reading.
    /// The canonical envelope is never throttled.
    pub max_rate_hz: f64,
    pub homeassistant_prefix: String,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: "localhost".to_string(),
            port: 1883,
            base_topic: "/telemetry/sensors".to_string(),
            client_id: None,
            keep_alive_secs: 30,
            username: None,
            password: None,
            password_file: None,
            qos: 0,
            use_tls: false,
            ca_cert_path: None,
            client_cert_path: None,
            client_key_path: None,
            formats: default_mqtt_formats(),
            max_rate_hz: 10.0,
            homeassistant_prefix: "homeassistant".to_string(),
        }
    }
}

impl MqttConfig {
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn wants(&self, format: MqttFormat) -> bool {
        self.formats.contains(&format)
    }

    pub fn password(&self) -> Result<Option<String>> {
        match &self.password_file {
            Some(path) => Ok(Some(
                std::fs::read_to_string(path)
                    .with_context(|| format!("reading mqtt password_file {path}"))?
                    .trim_end_matches(['\r', '\n'])
                    .to_string(),
            )),
            None => Ok(self.password.clone()),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP API / web dashboard
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct HttpConfig {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
    /// Bearer token required for `/api/*` (health and metrics stay open).
    pub token: Option<String>,
    /// `Access-Control-Allow-Origin` for browser dashboards (unset = no CORS).
    pub cors_origin: Option<String>,
    /// Serve the built-in web dashboard at `/`.
    pub ui: bool,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: "127.0.0.1".to_string(),
            // gps-to-mqtt 9090, ecu-to-mqtt 9091
            port: 9092,
            token: None,
            cors_origin: None,
            ui: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Sensors
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SensorConfig {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub driver: String,
    pub connection: ConnectionConfig,
    /// Driver-specific settings stored as raw TOML so each driver can
    /// deserialize what it needs via `settings.clone().try_into()`.
    pub settings: Option<toml::Value>,
    /// Canonical channel overrides: `field = "channelName"`, `""` drops one.
    #[serde(default)]
    pub channels: Option<BTreeMap<String, String>>,
}

impl SensorConfig {
    /// Time between reads. `settings.interval_ms`, else a per-driver default
    /// matched to how fast the part produces new values.
    pub fn interval_ms(&self) -> u64 {
        self.settings
            .as_ref()
            .and_then(|v| v.get("interval_ms"))
            .and_then(|v| v.as_integer())
            .map(|v| v.clamp(1, 60_000) as u64)
            .unwrap_or(match self.driver.as_str() {
                "mpu6500" | "synthetic" => 20,
                "gpio_button" => 10,
                "sds011" => 1000,
                "bh1750" => 200,
                _ => 100,
            })
    }
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
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ConnectionConfig {
    I2c(I2cConnectionConfig),
    Serial(SerialConnectionConfig),
    Tcp(TcpConnectionConfig),
    Gpio(GpioConnectionConfig),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct I2cConnectionConfig {
    #[serde(default = "default_i2c_device")]
    pub device: String,
    pub address: u16,
}

fn default_i2c_device() -> String {
    "/dev/i2c-1".to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SerialConnectionConfig {
    pub port: String,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
}

fn default_baud_rate() -> u32 {
    9600
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TcpConnectionConfig {
    pub host: String,
    #[serde(default = "default_tcp_port")]
    pub port: u16,
    /// I2C device address on the remote bridge (e.g. 0x68 for MPU-6500).
    /// Only required when the driver communicates with an I2C sensor via TCP.
    pub address: Option<u16>,
    /// When `true`, every response from the server is prefixed with a 2-byte
    /// big-endian payload length.  Set this to match the bridge configuration.
    /// Defaults to `false` (raw byte stream).
    #[serde(default)]
    pub framing: bool,
}

fn default_tcp_port() -> u16 {
    23
}

#[derive(Debug, Deserialize, Serialize, Clone)]
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

pub const DRIVERS: &[&str] = &[
    "synthetic",
    "mpu6500",
    "bmp280",
    "bme280",
    "sht31",
    "bh1750",
    "ina219",
    "ads1115",
    "gpio_button",
    "sds011",
];

// ---------------------------------------------------------------------------
// Validation and helpers
// ---------------------------------------------------------------------------

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        let levels = ["trace", "debug", "info", "warn", "error"];
        if !levels.contains(&self.log_level.to_lowercase().as_str()) {
            bail!("log_level must be one of {levels:?}");
        }
        if let Some(car) = &self.car
            && !valid_car(car)
        {
            bail!("car {car:?} must be a slug: lowercase letters, digits and '-', at most 32");
        }
        let m = &self.mqtt;
        if m.enabled {
            if m.host.trim().is_empty() {
                bail!("mqtt.host must not be empty");
            }
            if m.port == 0 {
                bail!("mqtt.port must be greater than 0");
            }
            if m.qos > 2 {
                bail!("mqtt.qos must be 0, 1 or 2");
            }
            if m.formats.is_empty() {
                bail!("mqtt.formats must name at least one format");
            }
            if !(0.0..=1000.0).contains(&m.max_rate_hz) {
                bail!("mqtt.max_rate_hz must be between 0 and 1000");
            }
            if m.base_topic.trim().is_empty() || m.base_topic.contains(['+', '#']) {
                bail!("mqtt.base_topic must be a topic without wildcards");
            }
            if m.client_cert_path.is_some() != m.client_key_path.is_some() {
                bail!("mqtt.client_cert_path and mqtt.client_key_path go together");
            }
        }
        if self.http.enabled {
            if self.http.port == 0 {
                bail!("http.port must be greater than 0");
            }
            format!("{}:{}", self.http.bind, self.http.port)
                .parse::<std::net::SocketAddr>()
                .with_context(|| format!("http.bind {:?} is not an IP address", self.http.bind))?;
        }
        let mut names = std::collections::HashSet::new();
        let mut channels: BTreeMap<String, String> = BTreeMap::new();
        for s in &self.sensors {
            if s.name.trim().is_empty() {
                bail!("every sensor needs a name");
            }
            if s.name.contains(['/', '+', '#']) {
                bail!(
                    "sensor name {:?} must not contain '/', '+' or '#' (it is part of a topic)",
                    s.name
                );
            }
            if !names.insert(s.name.as_str()) {
                bail!("sensor name {:?} is used twice", s.name);
            }
            if !DRIVERS.contains(&s.driver.as_str()) {
                bail!(
                    "sensor {:?}: unknown driver {:?} (available: {})",
                    s.name,
                    s.driver,
                    DRIVERS.join(", ")
                );
            }
            if let Some(map) = &s.channels {
                for name in map.values().filter(|n| !n.is_empty()) {
                    if !crate::channels::valid_channel(name) {
                        bail!(
                            "sensor {:?}: channel name {name:?} must be camelCase letters and digits",
                            s.name
                        );
                    }
                }
            }
            if !s.enabled {
                continue;
            }
            // two enabled sensors feeding one channel would overwrite each other
            for spec in crate::channels::channels_for(s) {
                if let Some(other) = channels.insert(spec.channel.clone(), s.name.clone())
                    && other != s.name
                {
                    bail!(
                        "channel {:?} comes from both {other:?} and {:?}; rename one in [sensors.channels]",
                        spec.channel,
                        s.name
                    );
                }
            }
        }
        Ok(())
    }

    /// The effective configuration with secrets masked.
    pub fn redacted(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        for (section, key) in [("mqtt", "password"), ("http", "token")] {
            if v[section].get(key).is_some_and(|x| !x.is_null()) {
                v[section][key] = "***".into();
            }
        }
        v
    }
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// `SENSORS_TO_MQTT_MQTT_HOST` → `mqtt.host`, `SENSORS_TO_MQTT_LOG_LEVEL` →
/// `log_level`. The nested `SENSORS_TO_MQTT__…` form is left to the
/// `Environment` source.
fn flat_env_key(var: &str) -> Option<String> {
    let rest = var.strip_prefix(ENV_PREFIX)?.strip_prefix('_')?;
    if rest.is_empty() || rest.starts_with('_') {
        return None;
    }
    let key = rest.to_lowercase();
    for section in ["mqtt_", "http_"] {
        if let Some(field) = key.strip_prefix(section) {
            return Some(format!("{}.{field}", &section[..4]));
        }
    }
    Some(key)
}

fn candidate_paths() -> Vec<String> {
    [
        "/etc/sensors-to-mqtt/config.toml",
        "/etc/sensors-to-mqtt/settings.toml",
        "config.toml",
        "settings.toml",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Load and merge configuration from a TOML file and environment variables.
pub fn load_configuration(path: Option<&str>) -> Result<AppConfig> {
    load_with_env(path, std::env::vars())
}

/// [`load_configuration`] with an explicit environment (for tests).
pub fn load_with_env(
    path: Option<&str>,
    env: impl IntoIterator<Item = (String, String)>,
) -> Result<AppConfig> {
    let mut builder = Config::builder();
    let mut used = None;

    if let Some(p) = path {
        builder = builder.add_source(File::with_name(p).required(true));
        used = Some(p.to_string());
    } else {
        // lowest priority first; later files override earlier ones
        for p in candidate_paths() {
            if std::path::Path::new(&p).is_file() {
                builder = builder.add_source(File::with_name(&p).required(true));
                used = Some(p);
            }
        }
    }

    let env: Vec<(String, String)> = env.into_iter().collect();
    builder = builder.add_source(
        Environment::with_prefix(ENV_PREFIX)
            .separator("__")
            .try_parsing(true)
            .source(Some(env.iter().cloned().collect())),
    );
    for (var, value) in &env {
        if let Some(key) = flat_env_key(var) {
            builder = builder
                .set_override(key, value.clone())
                .with_context(|| format!("environment variable {var}"))?;
        }
    }

    let built = builder.build().context("Failed to build configuration")?;
    let explicit_base = built.get_string("mqtt.base_topic").is_ok();
    let mut cfg: AppConfig = built
        .try_deserialize()
        .context("Failed to deserialize configuration")?;
    cfg.config_path = used;
    cfg.car = cfg.car.take().filter(|c| !c.trim().is_empty());
    cfg.base_topic_origin = if explicit_base {
        BaseTopicOrigin::Explicit
    } else if let Some(car) = cfg.car.as_ref().filter(|c| valid_car(c)) {
        cfg.mqtt.base_topic = format!("/g86/{car}/sensors");
        BaseTopicOrigin::Car
    } else {
        BaseTopicOrigin::Default
    };
    cfg.validate()?;
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn load_str(toml: &str, env: &[(&str, &str)]) -> Result<AppConfig> {
        let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        write!(f, "{toml}").unwrap();
        load_with_env(
            Some(f.path().to_str().unwrap()),
            env.iter().map(|(k, v)| (k.to_string(), v.to_string())),
        )
    }

    // --- Default values ---

    #[test]
    fn test_default_mqtt_config() {
        let cfg = MqttConfig::default();
        assert_eq!(cfg.port, 1883);
        assert_eq!(cfg.host, "localhost");
        assert!(cfg.enabled);
        assert_eq!(cfg.base_topic, "/telemetry/sensors");
        assert_eq!(cfg.formats, [MqttFormat::Canonical, MqttFormat::Json]);
    }

    #[test]
    fn test_default_app_config() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.log_level, "info");
        assert!(!cfg.log_json);
        assert_eq!(cfg.tui_refresh_rate_ms, 100);
        assert!(cfg.sensors.is_empty());
        assert_eq!(cfg.http.port, 9092);
        assert_eq!(cfg.http.bind, "127.0.0.1");
        cfg.validate().unwrap();
    }

    // --- address() helper ---

    #[test]
    fn test_mqtt_address() {
        let cfg = MqttConfig {
            host: "broker.local".into(),
            port: 8883,
            ..MqttConfig::default()
        };
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
            address: None,
            framing: false,
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
        let cfg = load_str(
            r#"
[mqtt]
host = "test-broker"
port = 1884
base_topic = "/TEST"
formats = ["canonical", "legacy"]
"#,
            &[],
        )
        .unwrap();
        assert_eq!(cfg.mqtt.host, "test-broker");
        assert_eq!(cfg.mqtt.port, 1884);
        assert!(cfg.mqtt.wants(MqttFormat::Legacy) && !cfg.mqtt.wants(MqttFormat::Json));
        assert!(cfg.config_path.is_some());
    }

    #[test]
    fn test_load_sensor_toml() {
        let cfg = load_str(
            r#"
[[sensors]]
name    = "Test IMU"
driver  = "synthetic"
enabled = true

[sensors.connection]
type = "i2c"
device  = "/dev/i2c-1"
address = 0x68

[sensors.settings]
interval_ms = 50

[sensors.channels]
temperature = "cabinC"
"#,
            &[],
        )
        .unwrap();
        assert_eq!(cfg.sensors.len(), 1);
        assert_eq!(cfg.sensors[0].name, "Test IMU");
        assert_eq!(cfg.sensors[0].driver, "synthetic");
        assert!(cfg.sensors[0].enabled);
        assert_eq!(cfg.sensors[0].interval_ms(), 50);
        assert_eq!(
            cfg.sensors[0].channels.as_ref().unwrap()["temperature"],
            "cabinC"
        );
    }

    #[test]
    fn test_load_gpio_sensor_toml() {
        let cfg = load_str(
            r#"
[[sensors]]
name   = "Brake Switch"
driver = "gpio_button"

[sensors.connection]
type       = "gpio"
pin        = 17
active_low = false
"#,
            &[],
        )
        .unwrap();
        let sensor = &cfg.sensors[0];
        assert_eq!(sensor.interval_ms(), 10);
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

    // --- Environment ---

    #[test]
    fn env_overrides_both_forms() {
        let cfg = load_str(
            "[mqtt]\nhost = \"file\"\n",
            &[
                ("SENSORS_TO_MQTT_MQTT_HOST", "192.168.88.15"),
                ("SENSORS_TO_MQTT_MQTT_FORMATS", "canonical,homeassistant"),
                ("SENSORS_TO_MQTT_HTTP_PORT", "9200"),
                ("SENSORS_TO_MQTT_HTTP_ENABLED", "false"),
                ("SENSORS_TO_MQTT_LOG_LEVEL", "debug"),
                ("SENSORS_TO_MQTT__MQTT__PORT", "1884"),
                ("UNRELATED", "x"),
            ],
        )
        .unwrap();
        assert_eq!(cfg.mqtt.host, "192.168.88.15");
        assert_eq!(cfg.mqtt.port, 1884);
        assert_eq!(
            cfg.mqtt.formats,
            [MqttFormat::Canonical, MqttFormat::Homeassistant]
        );
        assert_eq!(cfg.http.port, 9200);
        assert!(!cfg.http.enabled);
        assert_eq!(cfg.log_level, "debug");
    }

    #[test]
    fn flat_env_keys_map_to_sections() {
        assert_eq!(
            flat_env_key("SENSORS_TO_MQTT_MQTT_BASE_TOPIC").as_deref(),
            Some("mqtt.base_topic")
        );
        assert_eq!(
            flat_env_key("SENSORS_TO_MQTT_HTTP_TOKEN").as_deref(),
            Some("http.token")
        );
        assert_eq!(
            flat_env_key("SENSORS_TO_MQTT_LOG_JSON").as_deref(),
            Some("log_json")
        );
        assert_eq!(flat_env_key("SENSORS_TO_MQTT__MQTT__HOST"), None);
        assert_eq!(flat_env_key("SENSORS_TO_MQTTX"), None);
    }

    // --- Validation ---

    #[test]
    fn validation_rejects_bad_settings() {
        let sensor = |name: &str, driver: &str| {
            format!(
                "[[sensors]]\nname = \"{name}\"\ndriver = \"{driver}\"\n[sensors.connection]\ntype = \"i2c\"\naddress = 0x68\n"
            )
        };
        assert!(load_str("log_level = \"loud\"\n", &[]).is_err());
        assert!(load_str("[mqtt]\nqos = 3\n", &[]).is_err());
        assert!(load_str("[mqtt]\nformats = [\"xml\"]\n", &[]).is_err());
        assert!(load_str("[http]\nbind = \"not-an-ip\"\n", &[]).is_err());
        assert!(load_str(&sensor("a/b", "synthetic"), &[]).is_err());
        assert!(load_str(&sensor("x", "nope"), &[]).is_err());
        assert!(
            load_str(
                &format!("{}{}", sensor("x", "synthetic"), sensor("x", "bmp280")),
                &[]
            )
            .is_err()
        );
        // two IMUs on the same channel names clash …
        let err = load_str(
            &format!("{}{}", sensor("a", "mpu6500"), sensor("b", "mpu6500")),
            &[],
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("lateralG"), "{err:#}");
        // … unless one is disabled
        let two = format!(
            "{}{}",
            sensor("a", "mpu6500"),
            sensor("b", "mpu6500").replace("driver", "enabled = false\ndriver")
        );
        assert!(load_str(&two, &[]).is_ok());
    }

    #[test]
    fn car_sets_the_g86_namespace_unless_the_base_is_explicit() {
        let cfg = load_str("log_level = \"info\"\n", &[]).unwrap();
        assert_eq!(cfg.mqtt.base_topic, "/telemetry/sensors");
        assert_eq!(cfg.base_topic_origin, BaseTopicOrigin::Default);

        let old = load_str("[mqtt]\nbase_topic = \"/SENSORS\"\n", &[]).unwrap();
        assert_eq!(old.mqtt.base_topic, "/SENSORS");
        assert_eq!(old.base_topic_origin, BaseTopicOrigin::Explicit);

        let cfg = load_str("car = \"golf86\"\n", &[]).unwrap();
        assert_eq!(cfg.mqtt.base_topic, "/g86/golf86/sensors");
        assert_eq!(cfg.base_topic_origin, BaseTopicOrigin::Car);

        let cfg = load_str("", &[("SENSORS_TO_MQTT_CAR", "qa-car")]).unwrap();
        assert_eq!(cfg.mqtt.base_topic, "/g86/qa-car/sensors");

        let cfg = load_str(
            "car = \"golf86\"\n[mqtt]\nbase_topic = \"/GOLF86/SENSORS\"\n",
            &[],
        )
        .unwrap();
        assert_eq!(cfg.mqtt.base_topic, "/GOLF86/SENSORS");
        assert_eq!(cfg.base_topic_origin, BaseTopicOrigin::Explicit);

        let cfg = load_str(
            "car = \"golf86\"\n",
            &[("SENSORS_TO_MQTT_MQTT_BASE_TOPIC", "/X")],
        )
        .unwrap();
        assert_eq!(cfg.mqtt.base_topic, "/X");

        for bad in ["Golf86", "-x", "a_b", "a/b", &"a".repeat(33)] {
            assert!(
                load_str(&format!("car = \"{bad}\"\n"), &[]).is_err(),
                "{bad}"
            );
        }
        assert!(valid_car("0") && valid_car(&"a".repeat(32)));
    }

    #[test]
    fn redacted_masks_secrets() {
        let mut cfg = AppConfig::default();
        cfg.mqtt.password = Some("secret".into());
        cfg.http.token = Some("tok".into());
        let v = cfg.redacted();
        assert_eq!(v["mqtt"]["password"], "***");
        assert_eq!(v["http"]["token"], "***");
        assert!(AppConfig::default().redacted()["mqtt"]["password"].is_null());
    }
}
