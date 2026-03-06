//! Synthetic test sensor that emits sine-wave data on all channels.
//!
//! Useful for TUI development, CI testing, and validating MQTT pipelines
//! without real hardware.  Configure with `driver = "synthetic"`.
//!
//! Each field is a different frequency / phase so the TUI charts all look
//! distinct:
//!
//! | field          | range       | notes                          |
//! |----------------|-------------|--------------------------------|
//! | g_force_x      | ±2 g        | 0.5 Hz sine                    |
//! | g_force_y      | ±2 g        | 0.7 Hz sine, 90° offset        |
//! | g_force_z      | ±2 g        | 1.0 Hz sine, 45° offset        |
//! | combined_g     | 0..3.46 g   | √(x²+y²+z²)                   |
//! | gyro_x         | ±180 °/s    | 0.3 Hz cosine                  |
//! | gyro_y         | ±180 °/s    | 0.4 Hz                         |
//! | gyro_z         | ±180 °/s    | 0.6 Hz, inverted               |
//! | temperature    | 15..35 °C   | slow 0.05 Hz drift             |
//! | pressure       | 950..1050 hPa | very slow drift              |
//! | humidity       | 30..70 %    | 0.02 Hz sine                   |
//! | battery_voltage| 11.5..14.5 V| slow 0.03 Hz                  |
//! | rpm            | 800..6500   | 0.1 Hz sawtooth                |
//! | speed_kmh      | 0..200 km/h | 0.08 Hz sine (abs)             |
//! | throttle_pct   | 0..100 %    | 0.15 Hz abs-sine               |
//! | tilt_angle     | ±45 °       | coupled to g_force_z           |

use anyhow::Result;
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::time::Instant;

use crate::config::SensorConfig;
use crate::sensors::{FieldDescriptor, Sensor, SensorData, VizType};

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct SyntheticSettings {
    /// Simulated sample rate in Hz (default 50).
    #[serde(default = "default_rate")]
    pub rate_hz: f64,
    /// Scale factor applied to all frequencies (>1 = faster, default 1.0).
    #[serde(default = "default_one")]
    pub speed: f64,
    /// Noise amplitude added to each reading (default 0.02).
    #[serde(default = "default_noise")]
    pub noise: f64,
}

fn default_rate() -> f64 {
    50.0
}
fn default_one() -> f64 {
    1.0
}
fn default_noise() -> f64 {
    0.02
}

impl Default for SyntheticSettings {
    fn default() -> Self {
        Self {
            rate_hz: default_rate(),
            speed: default_one(),
            noise: default_noise(),
        }
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

pub struct SyntheticSensor {
    name: String,
    settings: SyntheticSettings,
    enabled: bool,
    started: Instant,
    tick: u64,
    fields: &'static [FieldDescriptor],
}

static FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        key: "g_force_x",
        label: "G-Force X",
        viz: VizType::GForce,
        group: Some("Acceleration"),
    },
    FieldDescriptor {
        key: "g_force_y",
        label: "G-Force Y",
        viz: VizType::GForce,
        group: None,
    },
    FieldDescriptor {
        key: "g_force_z",
        label: "G-Force Z",
        viz: VizType::GForce,
        group: None,
    },
    FieldDescriptor {
        key: "combined_g",
        label: "Combined G",
        viz: VizType::GForce,
        group: None,
    },
    FieldDescriptor {
        key: "tilt_angle",
        label: "Tilt Angle",
        viz: VizType::Angle,
        group: Some("Orientation"),
    },
    FieldDescriptor {
        key: "gyro_x",
        label: "Gyro X",
        viz: VizType::AngularRate,
        group: Some("Gyroscope"),
    },
    FieldDescriptor {
        key: "gyro_y",
        label: "Gyro Y",
        viz: VizType::AngularRate,
        group: None,
    },
    FieldDescriptor {
        key: "gyro_z",
        label: "Gyro Z",
        viz: VizType::AngularRate,
        group: None,
    },
    FieldDescriptor {
        key: "temperature",
        label: "Temperature",
        viz: VizType::Numeric { unit: "°C" },
        group: Some("Environment"),
    },
    FieldDescriptor {
        key: "pressure",
        label: "Pressure",
        viz: VizType::Numeric { unit: "hPa" },
        group: None,
    },
    FieldDescriptor {
        key: "humidity",
        label: "Humidity",
        viz: VizType::Numeric { unit: "%" },
        group: None,
    },
    FieldDescriptor {
        key: "battery_voltage",
        label: "Battery",
        viz: VizType::Numeric { unit: "V" },
        group: Some("Electrical"),
    },
    FieldDescriptor {
        key: "rpm",
        label: "Engine RPM",
        viz: VizType::Numeric { unit: "rpm" },
        group: Some("Engine"),
    },
    FieldDescriptor {
        key: "speed_kmh",
        label: "Speed",
        viz: VizType::Numeric { unit: "km/h" },
        group: None,
    },
    FieldDescriptor {
        key: "throttle_pct",
        label: "Throttle",
        viz: VizType::Numeric { unit: "%" },
        group: None,
    },
];

impl SyntheticSensor {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            settings: SyntheticSettings::default(),
            enabled: true,
            started: Instant::now(),
            tick: 0,
            fields: FIELDS,
        }
    }

    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let settings: SyntheticSettings = cfg
            .settings
            .as_ref()
            .map(|v| v.clone().try_into())
            .transpose()?
            .unwrap_or_default();

        Ok(Self {
            name: cfg.name.clone(),
            settings,
            enabled: cfg.enabled,
            started: Instant::now(),
            tick: 0,
            fields: FIELDS,
        })
    }

    /// Tiny deterministic pseudo-noise based on tick + seed.
    fn noise(&self, seed: u64) -> f64 {
        let v = (self.tick.wrapping_mul(2654435761).wrapping_add(seed)) as f64;
        ((v * 1e-10).sin() * 2.0 - 1.0) * self.settings.noise
    }
}

impl Sensor for SyntheticSensor {
    fn init(&mut self) -> Result<()> {
        self.started = Instant::now();
        self.tick = 0;
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        self.tick += 1;
        let t = self.started.elapsed().as_secs_f64() * self.settings.speed;
        let n = |seed| self.noise(seed);

        let gx = 2.0 * (2.0 * PI * 0.5 * t).sin() + n(1);
        let gy = 2.0 * (2.0 * PI * 0.7 * t + PI / 2.0).sin() + n(2);
        let gz = 2.0 * (2.0 * PI * 1.0 * t + PI / 4.0).sin() + n(3);
        let combined_g = (gx * gx + gy * gy + gz * gz).sqrt();
        let tilt = gz.atan2((gx * gx + gy * gy).sqrt()).to_degrees();

        let gyro_x = 180.0 * (2.0 * PI * 0.3 * t).cos() + n(4);
        let gyro_y = 180.0 * (2.0 * PI * 0.4 * t).sin() + n(5);
        let gyro_z = -180.0 * (2.0 * PI * 0.6 * t).sin() + n(6);

        let temperature = 25.0 + 10.0 * (2.0 * PI * 0.05 * t).sin() + n(7);
        let pressure = 1013.25 + 50.0 * (2.0 * PI * 0.01 * t).sin() + n(8);
        let humidity = 50.0 + 20.0 * (2.0 * PI * 0.02 * t).sin() + n(9);

        let battery_voltage = 13.0 + 1.5 * (2.0 * PI * 0.03 * t).sin() + n(10);

        // RPM: sawtooth 800..6500
        let rpm_period = 10.0 / self.settings.speed;
        let rpm_phase = (t % rpm_period) / rpm_period;
        let rpm = 800.0 + 5700.0 * rpm_phase + n(11) * 50.0;

        let speed_kmh = 100.0 + 100.0 * (2.0 * PI * 0.08 * t).sin().abs() + n(12) * 5.0;
        let throttle_pct =
            (100.0 * (2.0 * PI * 0.15 * t).sin().abs() + n(13) * 3.0).clamp(0.0, 100.0);

        let mut fields = HashMap::new();
        fields.insert("g_force_x".into(), gx);
        fields.insert("g_force_y".into(), gy);
        fields.insert("g_force_z".into(), gz);
        fields.insert("combined_g".into(), combined_g);
        fields.insert("tilt_angle".into(), tilt);
        fields.insert("gyro_x".into(), gyro_x);
        fields.insert("gyro_y".into(), gyro_y);
        fields.insert("gyro_z".into(), gyro_z);
        fields.insert("temperature".into(), temperature);
        fields.insert("pressure".into(), pressure);
        fields.insert("humidity".into(), humidity);
        fields.insert("battery_voltage".into(), battery_voltage);
        fields.insert("rpm".into(), rpm);
        fields.insert("speed_kmh".into(), speed_kmh);
        fields.insert("throttle_pct".into(), throttle_pct);

        Ok(SensorData {
            timestamp: Utc::now(),
            fields,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
    fn driver_name(&self) -> &str {
        "synthetic"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    fn recalibrate(&mut self) -> Result<()> {
        self.started = Instant::now();
        self.tick = 0;
        Ok(())
    }
    fn field_descriptors(&self) -> &[FieldDescriptor] {
        self.fields
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sensor() -> SyntheticSensor {
        let mut s = SyntheticSensor::new("test");
        s.init().unwrap();
        s
    }

    #[test]
    fn test_produces_all_declared_fields() {
        let mut s = make_sensor();
        let data = s.read().unwrap();
        for fd in s.field_descriptors() {
            assert!(
                data.fields.contains_key(fd.key),
                "missing field: {}",
                fd.key
            );
        }
    }

    #[test]
    fn test_g_force_bounds() {
        let mut s = make_sensor();
        for _ in 0..100 {
            let data = s.read().unwrap();
            let gx = data.fields["g_force_x"];
            let gy = data.fields["g_force_y"];
            let gz = data.fields["g_force_z"];
            // Sine amplitude 2 + max noise 0.02
            assert!(gx.abs() <= 2.1, "gx out of range: {}", gx);
            assert!(gy.abs() <= 2.1, "gy out of range: {}", gy);
            assert!(gz.abs() <= 2.1, "gz out of range: {}", gz);
        }
    }

    #[test]
    fn test_combined_g_consistent() {
        let mut s = make_sensor();
        let data = s.read().unwrap();
        let gx = data.fields["g_force_x"];
        let gy = data.fields["g_force_y"];
        let gz = data.fields["g_force_z"];
        let cg = data.fields["combined_g"];
        let expected = (gx * gx + gy * gy + gz * gz).sqrt();
        assert!((cg - expected).abs() < 1e-10);
    }

    #[test]
    fn test_temperature_range() {
        let mut s = make_sensor();
        for _ in 0..200 {
            let v = s.read().unwrap().fields["temperature"];
            assert!(
                v > 10.0 && v < 40.0,
                "temperature {} out of plausible range",
                v
            );
        }
    }

    #[test]
    fn test_rpm_range() {
        let mut s = make_sensor();
        for _ in 0..50 {
            let v = s.read().unwrap().fields["rpm"];
            assert!(v >= 700.0 && v <= 6600.0, "rpm {} out of range", v);
        }
    }

    #[test]
    fn test_throttle_clamped_0_100() {
        let mut s = make_sensor();
        for _ in 0..200 {
            let v = s.read().unwrap().fields["throttle_pct"];
            assert!(v >= 0.0 && v <= 100.0, "throttle {} out of [0,100]", v);
        }
    }

    #[test]
    fn test_data_changes_between_reads() {
        let mut s = make_sensor();
        let d1 = s.read().unwrap();
        // Advance time by forcing tick difference
        std::thread::sleep(std::time::Duration::from_millis(20));
        let d2 = s.read().unwrap();
        // At least one field should differ
        let any_diff = d1
            .fields
            .keys()
            .any(|k| (d1.fields[k] - d2.fields[k]).abs() > 1e-12);
        assert!(any_diff, "all fields identical in consecutive reads");
    }

    #[test]
    fn test_recalibrate_resets_tick() {
        let mut s = make_sensor();
        s.tick = 9999;
        s.recalibrate().unwrap();
        assert_eq!(s.tick, 0);
    }

    #[test]
    fn test_driver_name() {
        let s = SyntheticSensor::new("x");
        assert_eq!(s.driver_name(), "synthetic");
    }

    #[test]
    fn test_enable_disable() {
        let mut s = SyntheticSensor::new("x");
        assert!(s.is_enabled());
        s.set_enabled(false);
        assert!(!s.is_enabled());
        s.set_enabled(true);
        assert!(s.is_enabled());
    }
}
