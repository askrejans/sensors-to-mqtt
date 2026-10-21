//! Canonical channels: driver fields mapped onto the names the G86 ecosystem
//! uses (ecu-to-mqtt and gps-to-mqtt use the same style: camelCase, unit in
//! the name). The `<base>/telemetry` envelope, the on-demand answer, the HTTP
//! API and Home Assistant discovery all speak these names.
//!
//! Per sensor, `[sensors.channels]` renames a field's channel or drops it:
//!
//! ```toml
//! [sensors.channels]
//! temperature = "cabinC"   # rename
//! altitude_m  = ""         # do not publish
//! ```

use crate::config::SensorConfig;
use std::collections::HashMap;

/// A driver field and the channel it feeds.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelSpec {
    pub field: String,
    pub channel: String,
    /// channel = field × scale
    pub scale: f64,
}

/// Catalogue entry: label, unit, group and a sensible range for dashboards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelInfo {
    pub label: &'static str,
    pub unit: &'static str,
    pub group: &'static str,
    pub min: f64,
    pub max: f64,
}

/// (channel, label, unit, group, min, max)
const CATALOG: &[(&str, &str, &str, &str, f64, f64)] = &[
    ("lateralG", "Lateral G", "g", "motion", -2.0, 2.0),
    ("longitudinalG", "Longitudinal G", "g", "motion", -2.0, 2.0),
    ("verticalG", "Vertical G", "g", "motion", -1.0, 3.0),
    ("combinedG", "Combined G", "g", "motion", 0.0, 3.0),
    ("peakG", "Peak G", "g", "motion", 0.0, 3.0),
    ("rollRateDegS", "Roll rate", "°/s", "motion", -250.0, 250.0),
    (
        "pitchRateDegS",
        "Pitch rate",
        "°/s",
        "motion",
        -250.0,
        250.0,
    ),
    ("yawRateDegS", "Yaw rate", "°/s", "motion", -250.0, 250.0),
    ("leanDeg", "Lean", "°", "motion", -60.0, 60.0),
    ("bankDeg", "Bank", "°", "motion", -60.0, 60.0),
    ("tiltDeg", "Tilt", "°", "motion", 0.0, 90.0),
    (
        "ambientC",
        "Ambient temperature",
        "°C",
        "environment",
        -30.0,
        60.0,
    ),
    ("humidityPct", "Humidity", "%", "environment", 0.0, 100.0),
    ("baroKpa", "Barometer", "kPa", "environment", 80.0, 110.0),
    (
        "baroAltitudeM",
        "Barometric altitude",
        "m",
        "environment",
        -100.0,
        3000.0,
    ),
    ("lightLux", "Light", "lx", "environment", 0.0, 65535.0),
    ("pm25", "PM2.5", "µg/m³", "air", 0.0, 500.0),
    ("pm10", "PM10", "µg/m³", "air", 0.0, 500.0),
    ("aqiPm25", "AQI (PM2.5)", "", "air", 0.0, 500.0),
    ("aqiPm10", "AQI (PM10)", "", "air", 0.0, 500.0),
    ("busV", "Bus voltage", "V", "power", 0.0, 26.0),
    ("currentA", "Current", "A", "power", -10.0, 10.0),
    ("powerW", "Power", "W", "power", 0.0, 100.0),
    ("batteryV", "Battery", "V", "power", 9.0, 16.0),
    ("batterySocPct", "Battery charge", "%", "power", 0.0, 100.0),
];

/// (driver, field, channel, scale). Fields not listed are kept in the
/// per-sensor state document but not published as channels.
const DEFAULTS: &[(&str, &str, &str, f64)] = &[
    // MPU-6500, calibrated stationary with Z up, X right and Y forward
    ("mpu6500", "accel_raw_x", "lateralG", 1.0),
    ("mpu6500", "accel_raw_y", "longitudinalG", 1.0),
    ("mpu6500", "accel_raw_z", "verticalG", 1.0),
    ("mpu6500", "combined_g", "combinedG", 1.0),
    ("mpu6500", "peak_g", "peakG", 1.0),
    ("mpu6500", "gyro_x", "rollRateDegS", 1.0),
    ("mpu6500", "gyro_y", "pitchRateDegS", 1.0),
    ("mpu6500", "gyro_z", "yawRateDegS", 1.0),
    ("mpu6500", "lean_angle", "leanDeg", 1.0),
    ("mpu6500", "bank_angle", "bankDeg", 1.0),
    ("mpu6500", "tilt_angle", "tiltDeg", 1.0),
    // synthetic test sensor: the same channels as the real ones (its rpm,
    // speed and throttle stay out, so they never clash with ecu-to-mqtt)
    ("synthetic", "g_force_x", "lateralG", 1.0),
    ("synthetic", "g_force_y", "longitudinalG", 1.0),
    ("synthetic", "g_force_z", "verticalG", 1.0),
    ("synthetic", "combined_g", "combinedG", 1.0),
    ("synthetic", "gyro_x", "rollRateDegS", 1.0),
    ("synthetic", "gyro_y", "pitchRateDegS", 1.0),
    ("synthetic", "gyro_z", "yawRateDegS", 1.0),
    ("synthetic", "tilt_angle", "tiltDeg", 1.0),
    ("synthetic", "temperature", "ambientC", 1.0),
    ("synthetic", "pressure", "baroKpa", 0.1),
    ("synthetic", "humidity", "humidityPct", 1.0),
    ("synthetic", "battery_voltage", "batteryV", 1.0),
    // environment
    ("bmp280", "temperature", "ambientC", 1.0),
    ("bmp280", "pressure_hpa", "baroKpa", 0.1),
    ("bmp280", "altitude_m", "baroAltitudeM", 1.0),
    ("bme280", "temperature", "ambientC", 1.0),
    ("bme280", "pressure_hpa", "baroKpa", 0.1),
    ("bme280", "altitude_m", "baroAltitudeM", 1.0),
    ("bme280", "humidity_pct", "humidityPct", 1.0),
    ("sht31", "temperature", "ambientC", 1.0),
    ("sht31", "humidity_pct", "humidityPct", 1.0),
    ("bh1750", "lux", "lightLux", 1.0),
    // power
    ("ina219", "bus_voltage_v", "busV", 1.0),
    ("ina219", "current_a", "currentA", 1.0),
    ("ina219", "power_w", "powerW", 1.0),
    ("ina219", "soc_pct", "batterySocPct", 1.0),
    // particulates
    ("sds011", "pm2_5", "pm25", 1.0),
    ("sds011", "pm10", "pm10", 1.0),
    ("sds011", "aqi_pm2_5", "aqiPm25", 1.0),
    ("sds011", "aqi_pm10", "aqiPm10", 1.0),
];

/// Label, unit and range of a channel; a generic entry for sensor-named ones.
pub fn info(channel: &str) -> ChannelInfo {
    CATALOG
        .iter()
        .find(|c| c.0 == channel)
        .map(|c| ChannelInfo {
            label: c.1,
            unit: c.2,
            group: c.3,
            min: c.4,
            max: c.5,
        })
        .unwrap_or(ChannelInfo {
            label: "",
            unit: "",
            group: "other",
            min: 0.0,
            max: 0.0,
        })
}

/// Every catalogued channel name (for the API).
pub fn catalog() -> impl Iterator<Item = (&'static str, ChannelInfo)> {
    CATALOG.iter().map(|c| (c.0, info(c.0)))
}

/// "pit button" + "Pressed" → "pitButtonPressed"
fn camel(name: &str, suffix: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper && !out.is_empty() {
                out.extend(ch.to_uppercase());
            } else if out.is_empty() {
                out.extend(ch.to_lowercase());
            } else {
                out.push(ch);
            }
            upper = false;
        } else {
            upper = true;
        }
    }
    out + suffix
}

/// A valid channel name: camelCase letters and digits, starting with a letter.
pub fn valid_channel(name: &str) -> bool {
    name.len() <= 40
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The channels a configured sensor publishes, overrides applied.
pub fn channels_for(cfg: &SensorConfig) -> Vec<ChannelSpec> {
    let mut specs: Vec<ChannelSpec> = DEFAULTS
        .iter()
        .filter(|d| d.0 == cfg.driver)
        .map(|d| ChannelSpec {
            field: d.1.into(),
            channel: d.2.into(),
            scale: d.3,
        })
        .collect();
    // drivers without fixed semantics: channels named after the sensor
    match cfg.driver.as_str() {
        "gpio_button" => {
            specs.push(ChannelSpec {
                field: "state".into(),
                channel: camel(&cfg.name, "Pressed"),
                scale: 1.0,
            });
            specs.push(ChannelSpec {
                field: "press_count".into(),
                channel: camel(&cfg.name, "Presses"),
                scale: 1.0,
            });
        }
        "ads1115" => {
            for ch in 0..4 {
                specs.push(ChannelSpec {
                    field: format!("ch{ch}_mapped"),
                    channel: camel(&cfg.name, &format!("Ch{ch}")),
                    scale: 1.0,
                });
            }
        }
        _ => {}
    }
    // [sensors.channels]: field = "newName" renames, field = "" drops, an
    // unknown field adds a channel
    if let Some(table) = cfg.channels.as_ref() {
        for (field, name) in table {
            specs.retain(|s| &s.field != field);
            if !name.is_empty() {
                specs.push(ChannelSpec {
                    field: field.clone(),
                    channel: name.clone(),
                    scale: 1.0,
                });
            }
        }
    }
    specs
}

/// The channel values of one reading.
pub fn apply(specs: &[ChannelSpec], fields: &HashMap<String, f64>) -> Vec<(String, f64)> {
    specs
        .iter()
        .filter_map(|s| {
            let v = fields.get(&s.field)? * s.scale;
            v.is_finite().then(|| (s.channel.clone(), v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConnectionConfig, I2cConnectionConfig};

    fn sensor(driver: &str, name: &str) -> SensorConfig {
        SensorConfig {
            name: name.into(),
            enabled: true,
            driver: driver.into(),
            connection: ConnectionConfig::I2c(I2cConnectionConfig {
                device: "/dev/i2c-1".into(),
                address: 0x68,
            }),
            settings: None,
            channels: None,
        }
    }

    #[test]
    fn imu_fields_become_g86_channels() {
        let specs = channels_for(&sensor("mpu6500", "imu"));
        let fields: HashMap<String, f64> = [
            ("accel_raw_x".into(), 0.6),
            ("accel_raw_y".into(), -0.4),
            ("accel_raw_z".into(), 1.0),
            ("gyro_z".into(), 15.0),
            ("accel_x".into(), 9.9), // not a channel
        ]
        .into();
        let ch: HashMap<String, f64> = apply(&specs, &fields).into_iter().collect();
        assert_eq!(ch["lateralG"], 0.6);
        assert_eq!(ch["longitudinalG"], -0.4);
        assert_eq!(ch["verticalG"], 1.0);
        assert_eq!(ch["yawRateDegS"], 15.0);
        assert_eq!(ch.len(), 4);
    }

    #[test]
    fn units_are_converted_and_overrides_apply() {
        let mut s = sensor("bme280", "climate");
        s.channels = Some(
            [
                ("temperature".into(), "cabinC".into()),
                ("altitude_m".into(), String::new()),
            ]
            .into(),
        );
        let specs = channels_for(&s);
        let fields: HashMap<String, f64> = [
            ("temperature".into(), 21.5),
            ("pressure_hpa".into(), 1013.0),
            ("altitude_m".into(), 30.0),
        ]
        .into();
        let ch: HashMap<String, f64> = apply(&specs, &fields).into_iter().collect();
        assert_eq!(ch["cabinC"], 21.5);
        assert!((ch["baroKpa"] - 101.3).abs() < 1e-9); // hPa → kPa
        assert!(!ch.contains_key("ambientC"));
        assert!(!ch.contains_key("baroAltitudeM"));
    }

    #[test]
    fn sensor_named_channels() {
        let ch: Vec<String> = channels_for(&sensor("gpio_button", "pit button"))
            .into_iter()
            .map(|s| s.channel)
            .collect();
        assert_eq!(ch, ["pitButtonPressed", "pitButtonPresses"]);
        let ch: Vec<String> = channels_for(&sensor("ads1115", "oil"))
            .into_iter()
            .map(|s| s.channel)
            .collect();
        assert_eq!(ch, ["oilCh0", "oilCh1", "oilCh2", "oilCh3"]);
        assert!(
            valid_channel("pitButtonPressed") && !valid_channel("Bad name") && !valid_channel("")
        );
    }

    #[test]
    fn catalogue_covers_every_default_channel() {
        for d in DEFAULTS {
            assert!(
                !info(d.2).label.is_empty(),
                "{} has no catalogue entry",
                d.2
            );
            assert!(valid_channel(d.2));
        }
        assert_eq!(info("unknown").group, "other");
    }

    #[test]
    fn non_finite_values_are_dropped() {
        let specs = channels_for(&sensor("sht31", "s"));
        let fields: HashMap<String, f64> = [
            ("temperature".into(), f64::NAN),
            ("humidity_pct".into(), 40.0),
        ]
        .into();
        assert_eq!(
            apply(&specs, &fields),
            vec![("humidityPct".to_string(), 40.0)]
        );
    }
}
