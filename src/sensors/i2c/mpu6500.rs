use super::I2CDevice;
use crate::config::FilterConfig;
use crate::filters::kalman_1d::KalmanFilter1D;
use crate::sensors::{Sensor, SensorData};
use anyhow::{Context, Result};
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use serde::Deserialize;
use chrono::Utc;
use std::collections::HashMap;

const ACCEL_CONFIG: u8 = 0x1C;
const GYRO_CONFIG: u8 = 0x1B;
const ACCEL_XOUT_H: u8 = 0x3B;
const ACCEL_YOUT_H: u8 = 0x3D;
const ACCEL_ZOUT_H: u8 = 0x3F;
const GYRO_XOUT_H: u8 = 0x43;
const GYRO_YOUT_H: u8 = 0x45;
const GYRO_ZOUT_H: u8 = 0x47;

#[derive(Debug, Deserialize, Clone)]
pub struct MPU6500Settings {
    pub accel_range: u16,
    pub gyro_range: u16,
    pub sample_rate: u16,
    #[serde(default)]
    pub accel_filter: FilterConfig,
    #[serde(default)]
    pub accel_z_filter: FilterConfig,
    #[serde(default)]
    pub gyro_filter: FilterConfig,
}

pub struct MPU6500 {
    i2c: I2cdev,
    address: u16,
    name: String,
    enabled: bool,
    settings: MPU6500Settings,
    calibration: CalibrationData,
    accel_filters: [KalmanFilter1D; 3],
    gyro_filters: [KalmanFilter1D; 3],
}

struct CalibrationData {
    accel_offsets: [i32; 3],
    gyro_offsets: [i32; 3],
}

impl MPU6500 {
    fn remove_gravity(&self, raw_accel: [f64; 3]) -> [f64; 3] {
        // Estimate gravity components using complementary filter
        let gravity_magnitude = 1.0; // 1G
        let total_magnitude =
            (raw_accel[0].powi(2) + raw_accel[1].powi(2) + raw_accel[2].powi(2)).sqrt();

        // Scale factor to normalize gravity vector
        let scale = if total_magnitude != 0.0 {
            gravity_magnitude / total_magnitude
        } else {
            0.0
        };

        // Estimate gravity components
        let gravity = [
            raw_accel[0] * scale,
            raw_accel[1] * scale,
            raw_accel[2] * scale,
        ];

        // Remove gravity to get linear acceleration
        [
            raw_accel[0] - gravity[0],
            raw_accel[1] - gravity[1],
            raw_accel[2],
        ]
    }

    pub fn new(bus: &str, device: I2CDevice) -> Result<Self> {
        let i2c = I2cdev::new(bus).context("Failed to open I2C device")?;
        let settings: MPU6500Settings = serde_yaml_ng::from_value(device.settings)?;

        // Create Kalman filters using config parameters
        let accel_cfg = &settings.accel_filter;
        let accel_z_cfg = &settings.accel_z_filter;
        let gyro_cfg = &settings.gyro_filter;

        let accel_filters = [
            KalmanFilter1D::new(accel_cfg.process_noise, accel_cfg.measurement_noise)
                .with_dead_zone(accel_cfg.dead_zone),
            KalmanFilter1D::new(accel_cfg.process_noise, accel_cfg.measurement_noise)
                .with_dead_zone(accel_cfg.dead_zone),
            // Use tighter filtering for Z-axis to reduce jitter
            KalmanFilter1D::new(
                accel_z_cfg.process_noise * 0.5,
                accel_z_cfg.measurement_noise * 0.7
            ).with_dead_zone(accel_z_cfg.dead_zone * 0.5),
        ];

        let gyro_filters = [
            KalmanFilter1D::new(gyro_cfg.process_noise, gyro_cfg.measurement_noise)
                .with_dead_zone(gyro_cfg.dead_zone),
            KalmanFilter1D::new(gyro_cfg.process_noise, gyro_cfg.measurement_noise)
                .with_dead_zone(gyro_cfg.dead_zone),
            KalmanFilter1D::new(gyro_cfg.process_noise, gyro_cfg.measurement_noise)
                .with_dead_zone(gyro_cfg.dead_zone),
        ];

        let mut sensor = Self {
            i2c,
            address: device.address,
            name: device.name.clone(),
            enabled: device.enabled,
            settings,
            calibration: CalibrationData {
                accel_offsets: [0; 3],
                gyro_offsets: [0; 3],
            },
            accel_filters,
            gyro_filters,
        };

        sensor.init()?;
        sensor.calibrate()?;

        Ok(sensor)
    }

    fn read_sensor(&mut self, register: u8) -> Result<i16> {
        let mut buf = [0u8; 2];
        self.i2c.write_read(self.address, &[register], &mut buf)?;
        Ok(i16::from_be_bytes(buf))
    }

    pub fn calibrate(&mut self) -> Result<()> {
        log::info!("Calibrating {} ... Keep sensor still", self.name);

        let mut accel_sums = [0i32; 3];
        let mut gyro_sums = [0i32; 3];
        const CALIBRATION_SAMPLES: i32 = 300;

        for _ in 0..CALIBRATION_SAMPLES {
            let readings = self.read_raw()?;
            for i in 0..3 {
                accel_sums[i] += readings[i] as i32;
                gyro_sums[i] += readings[i + 3] as i32;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        for i in 0..3 {
            self.calibration.accel_offsets[i] = accel_sums[i] / CALIBRATION_SAMPLES;
            self.calibration.gyro_offsets[i] = gyro_sums[i] / CALIBRATION_SAMPLES;
        }

        // Adjust Z acceleration offset for gravity
        self.calibration.accel_offsets[2] -= match self.settings.accel_range {
            16 => 2048,
            8 => 4096,
            4 => 8192,
            2 => 16384,
            _ => 2048,
        };

        log::info!("Calibration complete for {}", self.name);
        Ok(())
    }

    fn read_raw(&mut self) -> Result<[i16; 6]> {
        Ok([
            self.read_sensor(ACCEL_XOUT_H)?,
            self.read_sensor(ACCEL_YOUT_H)?,
            self.read_sensor(ACCEL_ZOUT_H)?,
            self.read_sensor(GYRO_XOUT_H)?,
            self.read_sensor(GYRO_YOUT_H)?,
            self.read_sensor(GYRO_ZOUT_H)?,
        ])
    }

    fn calculate_angles(&self, values: &HashMap<String, f64>) -> Option<(f64, f64)> {
        // Use the filtered "raw" values for angle calculations
        let accel_x = values.get("accel_raw_x").copied().unwrap_or(0.0);
        let accel_y = values.get("accel_raw_y").copied().unwrap_or(0.0);
        let accel_z = values.get("accel_raw_z").copied().unwrap_or(0.0);
        
        let accel = [accel_x, accel_y, accel_z];

        // If we didn’t find the raw accelerations, bail out
        if accel == [0.0, 0.0, 0.0] {
            return None;
        }

        // Same angle calculation as before
        let ax2 = accel[0] * accel[0];
        let az2 = accel[2] * accel[2];
        let lean_angle = (accel[1] / (ax2 + az2).sqrt()).atan().to_degrees();
        let bank_angle = (accel[0] / accel[2].abs()).atan().to_degrees();
        Some((lean_angle, bank_angle))
    }
}

impl Sensor for MPU6500 {
    fn init(&mut self) -> Result<()> {
        // Wake up the device
        self.i2c.write(self.address, &[0x6B, 0x00])?;

        // Configure sample rate divider
        let sample_rate_div = (1000 / self.settings.sample_rate as u32 - 1) as u8;
        self.i2c.write(self.address, &[0x19, sample_rate_div])?;

        // Configure accelerometer and gyroscope ranges
        let accel_config = match self.settings.accel_range {
            16 => 0x18, // ±16g
            8 => 0x10,  // ±8g
            4 => 0x08,  // ±4g
            2 => 0x00,  // ±2g
            _ => 0x18,  // Default to ±16g
        };

        let gyro_config = match self.settings.gyro_range {
            2000 => 0x18, // ±2000°/s
            1000 => 0x10, // ±1000°/s
            500 => 0x08,  // ±500°/s
            250 => 0x00,  // ±250°/s
            _ => 0x18,    // Default to ±2000°/s
        };

        self.i2c
            .write(self.address, &[ACCEL_CONFIG, accel_config])?;
        self.i2c.write(self.address, &[GYRO_CONFIG, gyro_config])?;

        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let raw = self.read_raw()?;

        // Scale factors
        let accel_scale = match self.settings.accel_range {
            16 => 2048.0,
            8 => 4096.0,
            4 => 8192.0,
            2 => 16384.0,
            _ => 2048.0,
        };
        let gyro_scale = match self.settings.gyro_range {
            2000 => 16.4,
            1000 => 32.8,
            500 => 65.6,
            250 => 131.2,
            _ => 16.4,
        };

        // Compute raw accelerations
        let mut raw_accel = [0.0; 3];
        for i in 0..3 {
            raw_accel[i] = (raw[i] as i32 - self.calibration.accel_offsets[i]) as f64 / accel_scale;
        }

        // Compute gravity-removed accelerations (for G-forces)
        let linear_accel = self.remove_gravity(raw_accel);

        // Create separate filter instances for linear acceleration to avoid state corruption
        let mut linear_filters = [
            KalmanFilter1D::new(self.settings.accel_filter.process_noise, self.settings.accel_filter.measurement_noise)
                .with_dead_zone(self.settings.accel_filter.dead_zone),
            KalmanFilter1D::new(self.settings.accel_filter.process_noise, self.settings.accel_filter.measurement_noise)
                .with_dead_zone(self.settings.accel_filter.dead_zone),
            KalmanFilter1D::new(self.settings.accel_z_filter.process_noise, self.settings.accel_z_filter.measurement_noise)
                .with_dead_zone(self.settings.accel_z_filter.dead_zone),
        ];

        let mut data = HashMap::new();
        
        // 1) Filtered raw accelerations (used for angle calculations)
        let axes = ["x", "y", "z"];
        for i in 0..3 {
            let filtered_raw = self.accel_filters[i].update(raw_accel[i]);
            data.insert(format!("accel_raw_{}", axes[i]), filtered_raw);
        }

        // 2) Filtered linear accelerations (used for g-forces)
        for i in 0..3 {
            let filtered_linear = linear_filters[i].update(linear_accel[i]);
            data.insert(format!("accel_{}", axes[i]), filtered_linear);
            data.insert(format!("g_force_{}", axes[i]), filtered_linear);
        }

        // Filtered gyro data
        for i in 0..3 {
            let raw_gyro =
                (raw[i + 3] as i32 - self.calibration.gyro_offsets[i]) as f64 / gyro_scale;
            let filtered_gyro = self.gyro_filters[i].update(raw_gyro);
            data.insert(format!("gyro_{}", axes[i]), filtered_gyro);
            
            // Add human-readable rate names
            let rate_name = match i {
                0 => "roll_rate",
                1 => "pitch_rate",
                2 => "yaw_rate",
                _ => unreachable!(),
            };
            data.insert(rate_name.to_string(), filtered_gyro);
        }

        // Calculate and add angles
        if let Some((lean, bank)) = self.calculate_angles(&data) {
            data.insert("lean_angle".to_string(), lean);
            data.insert("bank_angle".to_string(), bank);
        }

        // Return sensor data
        Ok(SensorData {
            timestamp: Utc::now(),
            data,
        })
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        log::info!("Sensor {} {}", self.name, if enabled { "enabled" } else { "disabled" });
    }

    fn get_info(&self) -> Result<String> {
        Ok(format!(
            "{} MPU6500 IMU (addr: 0x{:02X}) - Accel: ±{}g, Gyro: ±{}°/s",
            self.name, self.address, self.settings.accel_range, self.settings.gyro_range
        ))
    }

    fn display_data(&self, data: &SensorData) -> Result<(u16, Option<String>)> {
        let mut lines = 0;
        let mut output = String::new();

        output.push_str(&format!(
            "Device: {} @ {}\n",
            self.name,
            data.timestamp.format("%H:%M:%S.%3f")
        ));
        lines += 1;

        output.push_str("G-Forces          │ Turn Rates        \n");
        output.push_str("──────────────────┼───────────────────\n");
        lines += 2;

        // Extract G-forces
        let g_x = data.data.get("g_force_x").unwrap_or(&0.0);
        let g_y = data.data.get("g_force_y").unwrap_or(&0.0);
        let g_z = data.data.get("g_force_z").unwrap_or(&0.0);

        // Extract turn rates
        let roll = data.data.get("roll_rate").unwrap_or(&0.0);
        let pitch = data.data.get("pitch_rate").unwrap_or(&0.0);
        let yaw = data.data.get("yaw_rate").unwrap_or(&0.0);

        output.push_str(&format!("Lateral:  {:6.3} G │ Roll:  {:6.2}°/s\n", g_x, roll));
        output.push_str(&format!("Forward:  {:6.3} G │ Pitch: {:6.2}°/s\n", g_y, pitch));
        output.push_str(&format!("Vertical: {:6.3} G │ Yaw:   {:6.2}°/s\n", g_z, yaw));
        lines += 3;

        // Display angles if available
        if let (Some(lean), Some(bank)) = (data.data.get("lean_angle"), data.data.get("bank_angle")) {
            output.push_str("──────────────────┴───────────────────\n");
            output.push_str(&format!(
                "Lean Angle: {:6.2}°  Bank Angle: {:6.2}°\n",
                lean, bank
            ));
            lines += 2;
        }

        Ok((lines, Some(output)))
    }

    fn recalibrate(&mut self) -> Result<()> {
        // Reset filters before recalibration
        for filter in &mut self.accel_filters {
            filter.reset();
        }
        for filter in &mut self.gyro_filters {
            filter.reset();
        }
        
        // Recalibrate
        self.calibrate()?;
        log::info!("Sensor {} recalibrated", self.name);
        Ok(())
    }
}
