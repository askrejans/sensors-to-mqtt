//! Integration tests for sensors-to-mqtt.
//!
//! These tests exercise multiple modules together without real hardware.
//! All tests use the `synthetic` driver which has no OS dependencies.

use sensors_to_mqtt::config::{
    AppConfig, ConnectionConfig, GpioConnectionConfig, I2cConnectionConfig, SensorConfig,
    SerialConnectionConfig, TcpConnectionConfig,
};
use sensors_to_mqtt::models::{AppState, SensorHistory};
use sensors_to_mqtt::sensors::registry::create_sensor;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn synthetic_sensor_config(name: &str) -> SensorConfig {
    SensorConfig {
        name: name.to_string(),
        enabled: true,
        driver: "synthetic".to_string(),
        connection: ConnectionConfig::I2c(I2cConnectionConfig {
            device: "/dev/i2c-1".to_string(),
            address: 0x68,
        }),
        settings: None,
    }
}

fn tcp_sensor_config(name: &str, driver: &str, host: &str, port: u16, address: u16) -> SensorConfig {
    SensorConfig {
        name: name.to_string(),
        enabled: true,
        driver: driver.to_string(),
        connection: ConnectionConfig::Tcp(TcpConnectionConfig {
            host: host.to_string(),
            port,
            address: Some(address),
            framing: false,
        }),
        settings: None,
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[test]
fn test_registry_creates_synthetic_sensor() {
    let cfg = synthetic_sensor_config("IMU1");
    let sensor = create_sensor(&cfg).unwrap();
    assert_eq!(sensor.name(), "IMU1");
    assert_eq!(sensor.driver_name(), "synthetic");
}

#[test]
fn test_registry_rejects_unknown_driver() {
    let mut cfg = synthetic_sensor_config("bad");
    cfg.driver = "nonexistent_driver_xyz".to_string();
    let result = create_sensor(&cfg);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("nonexistent_driver_xyz"));
}

// ---------------------------------------------------------------------------
// TCP connection config
// ---------------------------------------------------------------------------

#[test]
fn test_tcp_config_parses_with_address() {
    use sensors_to_mqtt::config::load_configuration;
    use std::io::Write;

    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    write!(
        f,
        r#"
[[sensors]]
name    = "Remote IMU"
driver  = "mpu6500"
enabled = true
[sensors.connection]
type    = "tcp"
host    = "192.168.88.58"
port    = 9002
address = 0x68
"#
    )
    .unwrap();

    let cfg = load_configuration(Some(f.path().to_str().unwrap())).unwrap();
    assert_eq!(cfg.sensors.len(), 1);
    let sensor = &cfg.sensors[0];
    assert_eq!(sensor.driver, "mpu6500");
    match &sensor.connection {
        ConnectionConfig::Tcp(t) => {
            assert_eq!(t.host, "192.168.88.58");
            assert_eq!(t.port, 9002);
            assert_eq!(t.address, Some(0x68));
        }
        other => panic!("expected Tcp connection, got {:?}", other),
    }
}

#[test]
fn test_tcp_config_address_is_optional() {
    use sensors_to_mqtt::config::load_configuration;
    use std::io::Write;

    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    write!(
        f,
        r#"
[[sensors]]
name    = "Air Quality"
driver  = "sds011"
enabled = true
[sensors.connection]
type = "tcp"
host = "192.168.1.1"
port = 8880
"#
    )
    .unwrap();

    let cfg = load_configuration(Some(f.path().to_str().unwrap())).unwrap();
    match &cfg.sensors[0].connection {
        ConnectionConfig::Tcp(t) => {
            assert_eq!(t.address, None); // not required for serial-over-TCP
        }
        other => panic!("expected Tcp connection, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// open_i2c — connection type validation (no hardware needed)
// ---------------------------------------------------------------------------

#[test]
fn test_open_i2c_rejects_serial_connection() {
    use sensors_to_mqtt::transport::open_i2c;

    let cfg = SensorConfig {
        name: "bad".to_string(),
        enabled: true,
        driver: "bmp280".to_string(),
        connection: ConnectionConfig::Serial(SerialConnectionConfig {
            port: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
        }),
        settings: None,
    };
    let result = open_i2c(&cfg, 0x76);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("serial"), "unexpected error: {}", msg);
}

#[test]
fn test_open_i2c_rejects_gpio_connection() {
    use sensors_to_mqtt::transport::open_i2c;

    let cfg = SensorConfig {
        name: "bad".to_string(),
        enabled: true,
        driver: "bmp280".to_string(),
        connection: ConnectionConfig::Gpio(GpioConnectionConfig {
            pin: 17,
            active_low: false,
            debounce_ms: 50,
        }),
        settings: None,
    };
    let result = open_i2c(&cfg, 0x76);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("gpio"), "unexpected error: {}", msg);
}

#[test]
fn test_open_i2c_tcp_fails_without_server() {
    use sensors_to_mqtt::transport::open_i2c;

    // TCP connection to a non-existent server should fail at connect time,
    // not silently succeed — this validates the correct TCP code path is taken.
    let cfg = tcp_sensor_config("imu", "mpu6500", "127.0.0.1", 19999, 0x68);
    let result = open_i2c(&cfg, 0x68);
    assert!(result.is_err(), "expected connection failure to unreachable server");
}

// ---------------------------------------------------------------------------
// GPIO button — connection type validation
// ---------------------------------------------------------------------------

#[test]
fn test_gpio_button_rejects_i2c_connection() {
    use sensors_to_mqtt::sensors::gpio::button::GpioButton;

    let cfg = SensorConfig {
        name: "bad".to_string(),
        enabled: true,
        driver: "gpio_button".to_string(),
        connection: ConnectionConfig::I2c(I2cConnectionConfig {
            device: "/dev/i2c-1".to_string(),
            address: 0x40,
        }),
        settings: None,
    };
    let result = GpioButton::from_config(&cfg);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("i2c"), "unexpected error: {}", msg);
}

#[test]
fn test_gpio_button_rejects_serial_connection() {
    use sensors_to_mqtt::sensors::gpio::button::GpioButton;

    let cfg = SensorConfig {
        name: "bad".to_string(),
        enabled: true,
        driver: "gpio_button".to_string(),
        connection: ConnectionConfig::Serial(SerialConnectionConfig {
            port: "/dev/ttyUSB0".to_string(),
            baud_rate: 9600,
        }),
        settings: None,
    };
    let result = GpioButton::from_config(&cfg);
    assert!(result.is_err());
}

#[test]
fn test_gpio_button_tcp_fails_without_server() {
    use sensors_to_mqtt::sensors::gpio::button::GpioButton;

    let cfg = SensorConfig {
        name: "brake".to_string(),
        enabled: true,
        driver: "gpio_button".to_string(),
        connection: ConnectionConfig::Tcp(TcpConnectionConfig {
            host: "127.0.0.1".to_string(),
            port: 19999,
            address: None,
            framing: false,
        }),
        settings: None,
    };
    let result = GpioButton::from_config(&cfg);
    assert!(result.is_err(), "expected connection failure to unreachable server");
}

// ---------------------------------------------------------------------------
// Synthetic sensor — full data pipeline
// ---------------------------------------------------------------------------

#[test]
fn test_synthetic_full_pipeline() {
    let cfg = synthetic_sensor_config("test");
    let mut sensor = create_sensor(&cfg).unwrap();
    sensor.init().unwrap();

    let mut history = SensorHistory::new(100);

    for _ in 0..20 {
        let data = sensor.read().unwrap();
        assert!(!data.fields.is_empty());
        history.push(&data);
    }

    // History should have accumulated
    assert!(history.get("g_force_x").map(|b| b.len()).unwrap_or(0) == 20);

    // Stats should be valid
    let (min, max, avg) = history.stats("g_force_x").unwrap();
    assert!(
        min <= avg && avg <= max,
        "min={} avg={} max={}",
        min,
        avg,
        max
    );
}

// ---------------------------------------------------------------------------
// AppState + sensor registration
// ---------------------------------------------------------------------------

#[test]
fn test_register_sensors_populates_state() {
    use sensors_to_mqtt::service::register_sensors;

    let sensors = vec![
        synthetic_sensor_config("IMU"),
        synthetic_sensor_config("ENV"),
    ];
    let mut state = AppState::new("localhost:1883".into(), true, 100);
    register_sensors(&mut state, &sensors);

    assert_eq!(state.sensor_statuses.len(), 2);
    assert!(state.sensor_statuses.contains_key("IMU"));
    assert!(state.sensor_statuses.contains_key("ENV"));
    assert!(state.sensor_history.contains_key("IMU"));
}

// ---------------------------------------------------------------------------
// Config round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_config_defaults_are_sensible() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.mqtt.port, 1883);
    assert!(!cfg.mqtt.base_topic.is_empty());
    assert!(!cfg.log_level.is_empty());
    assert!(cfg.tui_refresh_rate_ms > 0);
}

#[test]
fn test_config_load_with_multiple_sensors() {
    use sensors_to_mqtt::config::load_configuration;
    use std::io::Write;

    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    write!(
        f,
        r#"
[mqtt]
host       = "broker.example.com"
port       = 1884
base_topic = "/CAR"

[[sensors]]
name    = "IMU"
driver  = "synthetic"
enabled = true
[sensors.connection]
type    = "i2c"
device  = "/dev/i2c-1"
address = 0x68

[[sensors]]
name    = "Brake Switch"
driver  = "gpio_button"
enabled = false
[sensors.connection]
type        = "gpio"
pin         = 17
active_low  = true
debounce_ms = 30
"#
    )
    .unwrap();

    let cfg = load_configuration(Some(f.path().to_str().unwrap())).unwrap();
    assert_eq!(cfg.mqtt.host, "broker.example.com");
    assert_eq!(cfg.sensors.len(), 2);

    let imu = &cfg.sensors[0];
    assert_eq!(imu.driver, "synthetic");
    assert!(imu.enabled);

    let btn = &cfg.sensors[1];
    assert_eq!(btn.driver, "gpio_button");
    assert!(!btn.enabled);
    match &btn.connection {
        ConnectionConfig::Gpio(g) => {
            assert_eq!(g.pin, 17);
            assert!(g.active_low);
            assert_eq!(g.debounce_ms, 30);
        }
        other => panic!("expected Gpio connection, got {:?}", other),
    }
}

#[test]
fn test_config_load_tcp_sensors() {
    use sensors_to_mqtt::config::load_configuration;
    use std::io::Write;

    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    write!(
        f,
        r#"
[[sensors]]
name    = "Remote IMU"
driver  = "mpu6500"
enabled = true
[sensors.connection]
type    = "tcp"
host    = "192.168.88.58"
port    = 9002
address = 0x68

[[sensors]]
name    = "Remote Env"
driver  = "bme280"
enabled = true
[sensors.connection]
type    = "tcp"
host    = "192.168.88.58"
port    = 9003
address = 0x76

[[sensors]]
name    = "Remote Button"
driver  = "gpio_button"
enabled = true
[sensors.connection]
type = "tcp"
host = "192.168.88.58"
port = 9004

[[sensors]]
name    = "Air Quality"
driver  = "sds011"
enabled = true
[sensors.connection]
type = "tcp"
host = "192.168.88.58"
port = 8880
"#
    )
    .unwrap();

    let cfg = load_configuration(Some(f.path().to_str().unwrap())).unwrap();
    assert_eq!(cfg.sensors.len(), 4);

    // All drivers should be recognisable (no unknown driver errors at config level)
    let names: Vec<&str> = cfg.sensors.iter().map(|s| s.driver.as_str()).collect();
    assert!(names.contains(&"mpu6500"));
    assert!(names.contains(&"bme280"));
    assert!(names.contains(&"gpio_button"));
    assert!(names.contains(&"sds011"));

    // Check TCP with I2C address
    let imu = cfg.sensors.iter().find(|s| s.driver == "mpu6500").unwrap();
    match &imu.connection {
        ConnectionConfig::Tcp(t) => {
            assert_eq!(t.address, Some(0x68));
            assert_eq!(t.port, 9002);
        }
        other => panic!("expected Tcp, got {:?}", other),
    }

    // Check TCP without address (sds011 uses raw serial stream)
    let pm = cfg.sensors.iter().find(|s| s.driver == "sds011").unwrap();
    match &pm.connection {
        ConnectionConfig::Tcp(t) => assert_eq!(t.address, None),
        other => panic!("expected Tcp, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// TCP framing config option
// ---------------------------------------------------------------------------

#[test]
fn test_tcp_framing_defaults_to_false() {
    use sensors_to_mqtt::config::load_configuration;
    use std::io::Write;

    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    write!(
        f,
        r#"
[[sensors]]
name    = "Remote IMU"
driver  = "mpu6500"
enabled = true
[sensors.connection]
type    = "tcp"
host    = "192.168.1.1"
port    = 9002
address = 0x68
"#
    )
    .unwrap();

    let cfg = load_configuration(Some(f.path().to_str().unwrap())).unwrap();
    match &cfg.sensors[0].connection {
        ConnectionConfig::Tcp(t) => assert!(!t.framing, "framing should default to false"),
        other => panic!("expected Tcp, got {:?}", other),
    }
}

#[test]
fn test_tcp_framing_can_be_enabled() {
    use sensors_to_mqtt::config::load_configuration;
    use std::io::Write;

    let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    write!(
        f,
        r#"
[[sensors]]
name    = "Remote IMU"
driver  = "mpu6500"
enabled = true
[sensors.connection]
type    = "tcp"
host    = "192.168.1.1"
port    = 9002
address = 0x68
framing = true
"#
    )
    .unwrap();

    let cfg = load_configuration(Some(f.path().to_str().unwrap())).unwrap();
    match &cfg.sensors[0].connection {
        ConnectionConfig::Tcp(t) => assert!(t.framing, "framing should be true when set"),
        other => panic!("expected Tcp, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Kalman filter convergence
// ---------------------------------------------------------------------------

#[test]
fn test_kalman_converges_to_constant_input() {
    use sensors_to_mqtt::filters::kalman_1d::KalmanFilter1D;
    let mut filter = KalmanFilter1D::new(0.0001, 0.1);
    let target = 7.3;
    for _ in 0..500 {
        filter.update(target);
    }
    let out = filter.update(target);
    assert!(
        (out - target).abs() < 0.05,
        "filter did not converge: {}",
        out
    );
}

#[test]
fn test_kalman_tracks_slow_ramp() {
    use sensors_to_mqtt::filters::kalman_1d::KalmanFilter1D;
    let mut filter = KalmanFilter1D::new(0.01, 0.1);
    let mut last = 0.0_f64;
    for i in 0..100 {
        let input = i as f64 * 0.1;
        last = filter.update(input);
    }
    // Should roughly track to ~9.5 range
    assert!(last > 5.0, "filter lost track of ramp: {}", last);
}

// ---------------------------------------------------------------------------
// SensorHistory edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_history_get_nonexistent_field() {
    let h = SensorHistory::new(10);
    assert!(h.get("does_not_exist").is_none());
}

#[test]
fn test_history_empty_stats() {
    let h = SensorHistory::new(10);
    assert!(h.stats("x").is_none());
}

#[test]
fn test_history_single_point_stats() {
    use chrono::Utc;
    use sensors_to_mqtt::sensors::SensorData;
    let mut h = SensorHistory::new(10);
    h.push(&SensorData {
        timestamp: Utc::now(),
        fields: [("x".to_string(), 42.0)].into_iter().collect(),
    });
    let (min, max, avg) = h.stats("x").unwrap();
    assert!((min - 42.0).abs() < 1e-9);
    assert!((max - 42.0).abs() < 1e-9);
    assert!((avg - 42.0).abs() < 1e-9);
}
