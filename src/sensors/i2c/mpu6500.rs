use super::I2CDevice;
use crate::filters::kalman_1d::KalmanFilter1D;
use crate::mqtt_handler::MqttHandler;
use crate::sensors::{Sensor, SensorData};
use anyhow::{Context, Result};
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use serde::Deserialize;
use serde_json::{json, Value};

const ACCEL_CONFIG: u8 = 0x1C;
const GYRO_CONFIG: u8 = 0x1B;
const ACCEL_XOUT_H: u8 = 0x3B;
const ACCEL_YOUT_H: u8 = 0x3D;
const ACCEL_ZOUT_H: u8 = 0x3F;
const GYRO_XOUT_H: u8 = 0x43;
const GYRO_YOUT_H: u8 = 0x45;
const GYRO_ZOUT_H: u8 = 0x47;

#[derive(Debug, Deserialize)]
pub struct MPU6500Settings {
    pub accel_range: u16,
    pub gyro_range: u16,
    pub sample_rate: u16,
    pub samples_avg: i32,
}

pub struct MPU6500 {
    i2c: I2cdev,
    address: u16,
    name: String,
    settings: MPU6500Settings,
    calibration: CalibrationData,
    averages: AverageData,
    accel_filters: [KalmanFilter1D; 3],
    gyro_filters: [KalmanFilter1D; 3],
}

struct CalibrationData {
    accel_offsets: [i32; 3],
    gyro_offsets: [i32; 3],
}

struct AverageData {
    accel: [i32; 3],
    gyro: [i32; 3],
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

        // Create Kalman filters with appropriate noise parameters
        let accel_filters = [
            KalmanFilter1D::new(0.00001, 0.05).with_dead_zone(0.005), // Lateral (X)
            KalmanFilter1D::new(0.00001, 0.05).with_dead_zone(0.005), // Forward (Y)
            KalmanFilter1D::new(0.000005, 0.08).with_dead_zone(0.008), // Vertical (Z)
        ];

        let gyro_filters = [
            KalmanFilter1D::new(0.00005, 0.025).with_dead_zone(0.1), // Roll rate
            KalmanFilter1D::new(0.00005, 0.025).with_dead_zone(0.1), // Pitch rate
            KalmanFilter1D::new(0.00005, 0.025).with_dead_zone(0.1), // Yaw rate
        ];

        let mut sensor = Self {
            i2c,
            address: device.address,
            name: device.name,
            settings,
            calibration: CalibrationData {
                accel_offsets: [0; 3],
                gyro_offsets: [0; 3],
            },
            averages: AverageData {
                accel: [0; 3],
                gyro: [0; 3],
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

    fn calibrate(&mut self) -> Result<()> {
        println!("Calibrating MPU6500... Keep still");

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

        println!("Calibration complete!");
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

    fn calculate_angles(&self, values: &[(String, f64)]) -> Option<(f64, f64)> {
        // Use the filtered “raw” values for angle calculations
        let mut accel = [0.0; 3];
        for (key, value) in values {
            // We expect keys: "accel_raw_0", "accel_raw_1", "accel_raw_2"
            match key.as_str() {
                "accel_raw_0" => accel[0] = *value,
                "accel_raw_1" => accel[1] = *value,
                "accel_raw_2" => accel[2] = *value,
                _ => {}
            }
        }

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

    /// Publishes sensor data to MQTT topics
    pub fn publish_mqtt(&self, mqtt: &MqttHandler, data: &SensorData) -> Result<(), String> {
        let base_topic = format!("IMU/{}", self.name);

        // 1. Publish sensor info with filter parameters
        let info_json = json!({
            "timestamp": data.timestamp,
            "device": data.device_name,
            "sample_rate": data.sample_rate,
            "accel_range": self.settings.accel_range,
            "gyro_range": self.settings.gyro_range,
            "filter_info": {
                "accel_process_noise": 0.0001,
                "accel_measurement_noise": 0.0025,
                "gyro_process_noise": 0.0001,
                "gyro_measurement_noise": 0.003
            }
        });
        mqtt.publish_data(&format!("{}/INFO", base_topic), &info_json)?;

        // 2. Publish filtered sensor data
        let mut filtered_values = serde_json::Map::new();
        for (key, value) in &data.values {
            filtered_values.insert(key.clone(), json!(value));
        }
        filtered_values.insert("timestamp".to_string(), json!(data.timestamp));
        let filtered_json = Value::Object(filtered_values);
        mqtt.publish_data(&format!("{}/FILTERED", base_topic), &filtered_json)?;

        // 3. Calculate and publish derived data using filtered values
        let angles = self.calculate_angles(&data.values);
        let mut derived_data = serde_json::Map::new();

        if let Some((lean, bank)) = angles {
            derived_data.insert("lean_angle".to_string(), json!(lean));
            derived_data.insert("bank_angle".to_string(), json!(bank));
        }

        // Map filtered sensor values to meaningful names
        for (key, value) in &data.values {
            match key.as_str() {
                "accel_x" => derived_data.insert("lateral_g".to_string(), json!(value)),
                "accel_y" => derived_data.insert("forward_g".to_string(), json!(value)),
                "accel_z" => derived_data.insert("vertical_g".to_string(), json!(value)),
                "gyro_x" => derived_data.insert("roll_rate".to_string(), json!(value)),
                "gyro_y" => derived_data.insert("pitch_rate".to_string(), json!(value)),
                "gyro_z" => derived_data.insert("yaw_rate".to_string(), json!(value)),
                _ => None,
            };
        }

        derived_data.insert("timestamp".to_string(), json!(data.timestamp));
        let derived_json = Value::Object(derived_data);
        mqtt.publish_data(&format!("{}/DERIVED", base_topic), &derived_json)?;

        Ok(())
    }
}

impl Sensor for MPU6500 {
    fn as_mpu6500(&self) -> Option<&MPU6500> {
        Some(self)
    }

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
        let mut values = Vec::new();

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

        // Filter both raw and linear accelerations (store differently)
        // 1) Filtered raw accelerations (used for angle calculations)
        let mut filtered_raw_accel = [0.0; 3];
        for i in 0..3 {
            filtered_raw_accel[i] = self.accel_filters[i].update(raw_accel[i]);
            values.push((format!("accel_raw_{}", i), filtered_raw_accel[i]));
        }

        // 2) Filtered linear accelerations (used for g-forces)
        let mut filtered_linear_accel = [0.0; 3];
        for i in 0..3 {
            filtered_linear_accel[i] = self.accel_filters[i].update(linear_accel[i]);
            values.push((format!("accel_lin_{}", i), filtered_linear_accel[i]));
        }

        // Filtered gyro data
        for i in 0..3 {
            let raw_gyro =
                (raw[i + 3] as i32 - self.calibration.gyro_offsets[i]) as f64 / gyro_scale;
            let filtered_gyro = self.gyro_filters[i].update(raw_gyro);
            values.push((format!("gyro_{}", i), filtered_gyro));
        }

        // Return sensor data
        Ok(SensorData {
            timestamp: chrono::Utc::now().timestamp_millis(),
            device_name: self.name.clone(),
            sample_rate: self.settings.sample_rate,
            values,
        })
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
        let angles = self.calculate_angles(&data.values);

        output.push_str(&format!(
            "Device: {} @ {}\n",
            self.name,
            chrono::DateTime::from_timestamp_millis(data.timestamp)
                .unwrap()
                .format("%H:%M:%S.%3f")
        ));
        lines += 1;

        output.push_str("G-Forces          │ Turn Rates        \n");
        output.push_str("──────────────────┼───────────────────\n");
        lines += 2;

        let mut g_forces = Vec::new();
        let mut turn_rates = Vec::new();

        for (key, value) in &data.values {
            match key.as_str() {
                "accel_lin_0" => g_forces.push(format!("Right:   {:6.3} G", value)),
                "accel_lin_1" => g_forces.push(format!("Forward: {:6.3} G", value)),
                "accel_lin_2" => g_forces.push(format!("Up:      {:6.3} G", value)),
                "gyro_0" => turn_rates.push(format!("Roll:  {:6.2}°/s", value)),
                "gyro_1" => turn_rates.push(format!("Pitch: {:6.2}°/s", value)),
                "gyro_2" => turn_rates.push(format!("Yaw:   {:6.2}°/s", value)),
                _ => {}
            }
        }

        for i in 0..3 {
            if i < g_forces.len() && i < turn_rates.len() {
                output.push_str(&format!("{} │ {}\n", g_forces[i], turn_rates[i]));
                lines += 1;
            }
        }

        if let Some((lean, bank)) = angles {
            output.push_str("──────────────────┴───────────────────\n");
            output.push_str(&format!(
                "Lean Angle: {:6.2}°  Bank Angle: {:6.2}°\n",
                lean, bank
            ));
            lines += 2;
        }

        Ok((lines, Some(output)))
    }
}
