//! MPU6500 6-axis IMU driver (Linux only — requires linux-embedded-hal).
//!
//! Provides calibrated, Kalman-filtered accelerometer and gyroscope data.
//! Extra derived quantities: combined_g, tilt_angle, angular_velocity_magnitude,
//! and a rolling peak_g (cleared on recalibrate).

#![cfg(target_os = "linux")]

use anyhow::{Context, Result};
use chrono::Utc;
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use serde::Deserialize;
use std::collections::HashMap;

use crate::config::{ConnectionConfig, SensorConfig};
use crate::filters::kalman_1d::KalmanFilter1D;
use crate::sensors::{FieldDescriptor, Sensor, SensorData, VizType};

// ---------------------------------------------------------------------------
// Register map
// ---------------------------------------------------------------------------
const PWR_MGMT_1: u8 = 0x6B;
const SMPLRT_DIV: u8 = 0x19;
const ACCEL_CONFIG: u8 = 0x1C;
const GYRO_CONFIG: u8 = 0x1B;
const ACCEL_XOUT_H: u8 = 0x3B;
const GYRO_XOUT_H: u8 = 0x43;

// ---------------------------------------------------------------------------
// Settings (deserialised from config.toml [sensors.settings])
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct FilterConfig {
    pub process_noise: f64,
    pub measurement_noise: f64,
    pub dead_zone: f64,
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

#[derive(Debug, Deserialize, Clone)]
pub struct MPU6500Settings {
    #[serde(default = "default_accel_range")]
    pub accel_range: u16,
    #[serde(default = "default_gyro_range")]
    pub gyro_range: u16,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u16,
    #[serde(default = "default_history_size")]
    pub history_size: usize,
    #[serde(default)]
    pub accel_filter: FilterConfig,
    #[serde(default)]
    pub accel_z_filter: FilterConfig,
    #[serde(default)]
    pub gyro_filter: FilterConfig,
}

fn default_accel_range() -> u16 {
    16
}
fn default_gyro_range() -> u16 {
    2000
}
fn default_sample_rate() -> u16 {
    100
}
fn default_history_size() -> usize {
    600
}

// ---------------------------------------------------------------------------
// Calibration
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct CalibrationData {
    accel_offsets: [i32; 3],
    gyro_offsets: [i32; 3],
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

pub struct MPU6500 {
    i2c: I2cdev,
    address: u8,
    sensor_name: String,
    enabled: bool,
    settings: MPU6500Settings,
    calibration: CalibrationData,
    /// Stateful Kalman filters for raw accel (persistent across reads)
    accel_filters: [KalmanFilter1D; 3],
    /// Stateful Kalman filters for linear (gravity-removed) accel
    linear_filters: [KalmanFilter1D; 3],
    /// Stateful Kalman filters for gyroscope
    gyro_filters: [KalmanFilter1D; 3],
    /// Rolling peak combined-G (reset on recalibrate)
    peak_g: f64,
    /// Field descriptors (built once)
    descriptors: Vec<FieldDescriptor>,
}

impl MPU6500 {
    /// Construct from `SensorConfig` using the new config model.
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let (device, address) = match &cfg.connection {
            ConnectionConfig::I2c(c) => (c.device.clone(), c.address as u8),
            _ => anyhow::bail!("MPU6500 requires an I2C connection"),
        };

        let settings: MPU6500Settings = cfg
            .settings
            .as_ref()
            .map(|v| v.clone().try_into())
            .transpose()
            .map_err(|e: toml::de::Error| anyhow::anyhow!("MPU6500 settings: {}", e))?
            .unwrap_or_default();

        let i2c = I2cdev::new(&device).context("Failed to open I2C device")?;

        let accel_filters = Self::build_accel_filters(&settings);
        let linear_filters = Self::build_linear_filters(&settings);
        let gyro_filters = Self::build_gyro_filters(&settings);
        let descriptors = Self::build_descriptors();

        let mut sensor = Self {
            i2c,
            address,
            sensor_name: cfg.name.clone(),
            enabled: cfg.enabled,
            settings,
            calibration: CalibrationData::default(),
            accel_filters,
            linear_filters,
            gyro_filters,
            peak_g: 0.0,
            descriptors,
        };

        sensor.init()?;
        sensor.do_calibrate()?;
        Ok(sensor)
    }

    fn build_accel_filters(s: &MPU6500Settings) -> [KalmanFilter1D; 3] {
        let a = &s.accel_filter;
        let z = &s.accel_z_filter;
        [
            KalmanFilter1D::new(a.process_noise, a.measurement_noise).with_dead_zone(a.dead_zone),
            KalmanFilter1D::new(a.process_noise, a.measurement_noise).with_dead_zone(a.dead_zone),
            KalmanFilter1D::new(z.process_noise, z.measurement_noise).with_dead_zone(z.dead_zone),
        ]
    }

    fn build_linear_filters(s: &MPU6500Settings) -> [KalmanFilter1D; 3] {
        let a = &s.accel_filter;
        let z = &s.accel_z_filter;
        [
            KalmanFilter1D::new(a.process_noise, a.measurement_noise).with_dead_zone(a.dead_zone),
            KalmanFilter1D::new(a.process_noise, a.measurement_noise).with_dead_zone(a.dead_zone),
            KalmanFilter1D::new(z.process_noise, z.measurement_noise).with_dead_zone(z.dead_zone),
        ]
    }

    fn build_gyro_filters(s: &MPU6500Settings) -> [KalmanFilter1D; 3] {
        let g = &s.gyro_filter;
        [
            KalmanFilter1D::new(g.process_noise, g.measurement_noise).with_dead_zone(g.dead_zone),
            KalmanFilter1D::new(g.process_noise, g.measurement_noise).with_dead_zone(g.dead_zone),
            KalmanFilter1D::new(g.process_noise, g.measurement_noise).with_dead_zone(g.dead_zone),
        ]
    }

    fn build_descriptors() -> Vec<FieldDescriptor> {
        vec![
            // Accelerometer
            FieldDescriptor {
                key: "accel_x",
                label: "Accel X",
                viz: VizType::GForce,
                group: Some("ACCELEROMETER"),
            },
            FieldDescriptor {
                key: "accel_y",
                label: "Accel Y",
                viz: VizType::GForce,
                group: None,
            },
            FieldDescriptor {
                key: "accel_z",
                label: "Accel Z",
                viz: VizType::GForce,
                group: None,
            },
            // G-forces
            FieldDescriptor {
                key: "g_force_x",
                label: "G Lateral",
                viz: VizType::GForce,
                group: Some("G-FORCES"),
            },
            FieldDescriptor {
                key: "g_force_y",
                label: "G Forward",
                viz: VizType::GForce,
                group: None,
            },
            FieldDescriptor {
                key: "g_force_z",
                label: "G Vertical",
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
                key: "peak_g",
                label: "Peak G",
                viz: VizType::GForce,
                group: None,
            },
            // Gyroscope
            FieldDescriptor {
                key: "roll_rate",
                label: "Roll Rate",
                viz: VizType::AngularRate,
                group: Some("GYROSCOPE"),
            },
            FieldDescriptor {
                key: "pitch_rate",
                label: "Pitch Rate",
                viz: VizType::AngularRate,
                group: None,
            },
            FieldDescriptor {
                key: "yaw_rate",
                label: "Yaw Rate",
                viz: VizType::AngularRate,
                group: None,
            },
            FieldDescriptor {
                key: "angular_velocity",
                label: "Angular Vel",
                viz: VizType::AngularRate,
                group: None,
            },
            // Orientation
            FieldDescriptor {
                key: "lean_angle",
                label: "Lean Angle",
                viz: VizType::Angle,
                group: Some("ORIENTATION"),
            },
            FieldDescriptor {
                key: "bank_angle",
                label: "Bank Angle",
                viz: VizType::Angle,
                group: None,
            },
            FieldDescriptor {
                key: "tilt_angle",
                label: "Tilt Angle",
                viz: VizType::Angle,
                group: None,
            },
        ]
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn read_register_i16(&mut self, reg: u8) -> Result<i16> {
        let mut buf = [0u8; 2];
        self.i2c.write_read(self.address, &[reg], &mut buf)?;
        Ok(i16::from_be_bytes(buf))
    }

    fn read_raw_6(&mut self) -> Result<[i16; 6]> {
        Ok([
            self.read_register_i16(ACCEL_XOUT_H)?,
            self.read_register_i16(ACCEL_XOUT_H + 2)?,
            self.read_register_i16(ACCEL_XOUT_H + 4)?,
            self.read_register_i16(GYRO_XOUT_H)?,
            self.read_register_i16(GYRO_XOUT_H + 2)?,
            self.read_register_i16(GYRO_XOUT_H + 4)?,
        ])
    }

    fn accel_scale(&self) -> f64 {
        match self.settings.accel_range {
            2 => 16384.0,
            4 => 8192.0,
            8 => 4096.0,
            16 => 2048.0,
            _ => 2048.0,
        }
    }

    fn gyro_scale(&self) -> f64 {
        match self.settings.gyro_range {
            250 => 131.2,
            500 => 65.6,
            1000 => 32.8,
            2000 => 16.4,
            _ => 16.4,
        }
    }

    fn gravity_for_range(&self) -> i32 {
        match self.settings.accel_range {
            2 => 16384,
            4 => 8192,
            8 => 4096,
            16 => 2048,
            _ => 2048,
        }
    }

    /// Remove the static gravity component from accelerometer readings.
    /// X and Y get gravity subtracted proportionally; Z is returned raw
    /// (gravity is mostly along Z when the sensor is horizontal).
    fn remove_gravity(raw: [f64; 3]) -> [f64; 3] {
        let mag = (raw[0].powi(2) + raw[1].powi(2) + raw[2].powi(2)).sqrt();
        if mag < 1e-9 {
            return raw;
        }
        let scale = 1.0 / mag;
        let gravity = [raw[0] * scale, raw[1] * scale, raw[2] * scale];
        [raw[0] - gravity[0], raw[1] - gravity[1], raw[2]]
    }

    /// Perform calibration: 300 samples @ 10 ms, average, subtract 1G from Z.
    pub fn do_calibrate(&mut self) -> Result<()> {
        const N: i32 = 300;
        let mut sums = [0i64; 6];
        for _ in 0..N {
            let raw = self.read_raw_6()?;
            for i in 0..6 {
                sums[i] += raw[i] as i64;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for i in 0..3 {
            self.calibration.accel_offsets[i] = (sums[i] / N as i64) as i32;
            self.calibration.gyro_offsets[i] = (sums[i + 3] / N as i64) as i32;
        }
        // Remove 1G from Z
        self.calibration.accel_offsets[2] -= self.gravity_for_range();
        self.peak_g = 0.0;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Sensor trait implementation
// ---------------------------------------------------------------------------

impl Sensor for MPU6500 {
    fn init(&mut self) -> Result<()> {
        // Wake up
        self.i2c.write(self.address, &[PWR_MGMT_1, 0x00])?;
        // Sample rate divider
        let div = ((1000u32 / self.settings.sample_rate as u32).saturating_sub(1)) as u8;
        self.i2c.write(self.address, &[SMPLRT_DIV, div])?;
        // Accel config
        let accel_cfg: u8 = match self.settings.accel_range {
            2 => 0x00,
            4 => 0x08,
            8 => 0x10,
            16 => 0x18,
            _ => 0x18,
        };
        self.i2c.write(self.address, &[ACCEL_CONFIG, accel_cfg])?;
        // Gyro config
        let gyro_cfg: u8 = match self.settings.gyro_range {
            250 => 0x00,
            500 => 0x08,
            1000 => 0x10,
            2000 => 0x18,
            _ => 0x18,
        };
        self.i2c.write(self.address, &[GYRO_CONFIG, gyro_cfg])?;
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let raw = self.read_raw_6()?;
        let a_scale = self.accel_scale();
        let g_scale = self.gyro_scale();

        // Apply calibration and scale
        let raw_accel: [f64; 3] = [
            (raw[0] as i32 - self.calibration.accel_offsets[0]) as f64 / a_scale,
            (raw[1] as i32 - self.calibration.accel_offsets[1]) as f64 / a_scale,
            (raw[2] as i32 - self.calibration.accel_offsets[2]) as f64 / a_scale,
        ];

        let linear_accel = Self::remove_gravity(raw_accel);

        // Filter raw accel (stateful)
        let filt_raw: [f64; 3] = [
            self.accel_filters[0].update(raw_accel[0]),
            self.accel_filters[1].update(raw_accel[1]),
            self.accel_filters[2].update(raw_accel[2]),
        ];

        // Filter linear/G-force accel (stateful — bug fixed vs old code)
        let filt_lin: [f64; 3] = [
            self.linear_filters[0].update(linear_accel[0]),
            self.linear_filters[1].update(linear_accel[1]),
            self.linear_filters[2].update(linear_accel[2]),
        ];

        // Gyro
        let raw_gyro: [f64; 3] = [
            (raw[3] as i32 - self.calibration.gyro_offsets[0]) as f64 / g_scale,
            (raw[4] as i32 - self.calibration.gyro_offsets[1]) as f64 / g_scale,
            (raw[5] as i32 - self.calibration.gyro_offsets[2]) as f64 / g_scale,
        ];
        let filt_gyro: [f64; 3] = [
            self.gyro_filters[0].update(raw_gyro[0]),
            self.gyro_filters[1].update(raw_gyro[1]),
            self.gyro_filters[2].update(raw_gyro[2]),
        ];

        // Derived quantities
        let combined_g = (filt_lin[0].powi(2) + filt_lin[1].powi(2) + filt_lin[2].powi(2)).sqrt();
        if combined_g > self.peak_g {
            self.peak_g = combined_g;
        }

        let tilt_angle = {
            let ax2 = filt_raw[0].powi(2);
            let ay2 = filt_raw[1].powi(2);
            let az = filt_raw[2];
            ((ax2 + ay2).sqrt() / az.abs().max(1e-9))
                .atan()
                .to_degrees()
        };
        let lean_angle = (filt_raw[1]
            / (filt_raw[0].powi(2) + filt_raw[2].powi(2)).sqrt().max(1e-9))
        .atan()
        .to_degrees();
        let bank_angle = (filt_raw[0] / filt_raw[2].abs().max(1e-9))
            .atan()
            .to_degrees();
        let angular_velocity =
            (filt_gyro[0].powi(2) + filt_gyro[1].powi(2) + filt_gyro[2].powi(2)).sqrt();

        let mut fields = HashMap::new();
        // Raw accel
        fields.insert("accel_raw_x".to_string(), filt_raw[0]);
        fields.insert("accel_raw_y".to_string(), filt_raw[1]);
        fields.insert("accel_raw_z".to_string(), filt_raw[2]);
        // Linear accel / G-forces
        fields.insert("accel_x".to_string(), filt_lin[0]);
        fields.insert("accel_y".to_string(), filt_lin[1]);
        fields.insert("accel_z".to_string(), filt_lin[2]);
        fields.insert("g_force_x".to_string(), filt_lin[0]);
        fields.insert("g_force_y".to_string(), filt_lin[1]);
        fields.insert("g_force_z".to_string(), filt_lin[2]);
        fields.insert("combined_g".to_string(), combined_g);
        fields.insert("peak_g".to_string(), self.peak_g);
        // Gyro
        fields.insert("gyro_x".to_string(), filt_gyro[0]);
        fields.insert("gyro_y".to_string(), filt_gyro[1]);
        fields.insert("gyro_z".to_string(), filt_gyro[2]);
        fields.insert("roll_rate".to_string(), filt_gyro[0]);
        fields.insert("pitch_rate".to_string(), filt_gyro[1]);
        fields.insert("yaw_rate".to_string(), filt_gyro[2]);
        fields.insert("angular_velocity".to_string(), angular_velocity);
        // Orientation
        fields.insert("lean_angle".to_string(), lean_angle);
        fields.insert("bank_angle".to_string(), bank_angle);
        fields.insert("tilt_angle".to_string(), tilt_angle);

        Ok(SensorData {
            timestamp: Utc::now(),
            fields,
        })
    }

    fn name(&self) -> &str {
        &self.sensor_name
    }
    fn driver_name(&self) -> &str {
        "MPU6500"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }
    fn set_enabled(&mut self, v: bool) {
        self.enabled = v;
    }

    fn recalibrate(&mut self) -> Result<()> {
        for f in &mut self.accel_filters {
            f.reset();
        }
        for f in &mut self.linear_filters {
            f.reset();
        }
        for f in &mut self.gyro_filters {
            f.reset();
        }
        self.do_calibrate()
    }

    fn field_descriptors(&self) -> &[FieldDescriptor] {
        &self.descriptors
    }
}
