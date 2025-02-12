/// MPU6500 is a struct representing the MPU6500 IMU sensor.
/// It provides methods to initialize, read, and calibrate the sensor, as well as to calculate angles from the sensor data.
///
/// # Fields
/// - `i2c`: The I2C device used to communicate with the sensor.
/// - `address`: The I2C address of the sensor.
/// - `name`: The name of the sensor.
/// - `settings`: The settings for the sensor, including accelerometer and gyroscope ranges, sample rate, and samples average.
/// - `calibration`: The calibration data for the sensor, including accelerometer and gyroscope offsets.
/// - `averages`: The moving average data for the sensor, including accelerometer and gyroscope values.
///
/// # Methods
/// - `new(bus: &str, device: I2CDevice) -> Result<Self>`: Creates a new MPU6500 instance and initializes the sensor.
/// - `read_sensor(&mut self, register: u8) -> Result<i16>`: Reads raw data from a specified register.
/// - `calibrate(&mut self) -> Result<()>`: Calibrates the sensor by taking multiple readings and calculating offsets.
/// - `read_raw(&mut self) -> Result<[i16; 6]>`: Reads raw accelerometer and gyroscope data from the sensor.
/// - `calculate_angles(&self, values: &[(String, f64)]) -> Option<(f64, f64)>`: Calculates lean and bank angles from accelerometer data.
/// - `init(&mut self) -> Result<()>`: Initializes the sensor by waking it up and configuring its settings.
/// - `read(&mut self) -> Result<SensorData>`: Reads and processes sensor data, applying offsets and scaling, and updating moving averages.
/// - `get_info(&self) -> Result<String>`: Returns a string with information about the sensor.
/// - `display_data(&self, data: &SensorData) -> Result<(u16, Option<String>)>`: Formats and returns sensor data for display.
use super::I2CDevice;
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
    pub fn new(bus: &str, device: I2CDevice) -> Result<Self> {
        let i2c = I2cdev::new(bus).context("Failed to open I2C device")?;
        let settings: MPU6500Settings = serde_yaml_ng::from_value(device.settings)?;

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
        let mut accel = [0.0; 3];
        let mut has_accel = false;

        for (key, value) in values {
            match key.as_str() {
                "accel_x" => {
                    accel[0] = *value;
                    has_accel = true;
                }
                "accel_y" => {
                    accel[1] = *value;
                }
                "accel_z" => {
                    accel[2] = *value;
                }
                _ => {}
            }
        }

        if !has_accel {
            return None;
        }

        let ax2 = accel[0] * accel[0];
        let az2 = accel[2] * accel[2];
        let lean_angle = (accel[1] / (ax2 + az2).sqrt()).atan() * 180.0 / std::f64::consts::PI;
        let bank_angle = (accel[0] / accel[2].abs()).atan() * 180.0 / std::f64::consts::PI;

        Some((lean_angle, bank_angle))
    }

    /// Publishes sensor data to MQTT topics
    pub fn publish_mqtt(&self, mqtt: &MqttHandler, data: &SensorData) -> Result<(), String> {
        // Calculate base topic for this IMU
        let base_topic = format!("IMU/{}", self.name);

        // 1. Publish sensor info
        let info_json = json!({
            "timestamp": data.timestamp,
            "device": data.device_name,
            "sample_rate": data.sample_rate,
            "accel_range": self.settings.accel_range,
            "gyro_range": self.settings.gyro_range
        });
        mqtt.publish_data(&format!("{}/INFO", base_topic), &info_json)?;

        // 2. Publish raw sensor data
        let mut raw_values = serde_json::Map::new();
        for (key, value) in &data.values {
            raw_values.insert(key.clone(), json!(value));
        }
        raw_values.insert("timestamp".to_string(), json!(data.timestamp));
        let raw_json = Value::Object(raw_values);
        mqtt.publish_data(&format!("{}/RAW", base_topic), &raw_json)?;

        // 3. Calculate and publish derived data
        let angles = self.calculate_angles(&data.values);
        let mut derived_data = serde_json::Map::new();

        // Add angles if available
        if let Some((lean, bank)) = angles {
            derived_data.insert("lean_angle".to_string(), json!(lean));
            derived_data.insert("bank_angle".to_string(), json!(bank));
        }

        // Map sensor values to meaningful names
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

/// Implementation of the `Sensor` trait for the `MPU6500` struct.
///
/// This implementation provides methods to initialize the sensor, read data from it,
/// retrieve sensor information, and display the sensor data in a formatted manner.
///
/// # Methods
///
/// - `init(&mut self) -> Result<()>`
///   - Initializes the MPU6500 sensor by waking it up, configuring the sample rate divider,
///     and setting the accelerometer and gyroscope ranges based on the provided settings.
///
/// - `read(&mut self) -> Result<SensorData>`
///   - Reads raw data from the sensor, applies offsets and scaling, updates moving averages,
///     and converts the raw data to scaled values. Returns the sensor data as a `SensorData` struct.
///
/// - `get_info(&self) -> Result<String>`
///   - Returns a string containing information about the sensor, including its name, address,
///     accelerometer range, and gyroscope range.
///
/// - `display_data(&self, data: &SensorData) -> Result<(u16, Option<String>)>`
///   - Formats the sensor data into a human-readable string, including G-forces, turn rates,
///     and angles. Returns the number of lines in the output and the formatted string.
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

        // Apply offsets and scaling
        let accel_scale = match self.settings.accel_range {
            16 => 2048.0, // ±16g
            8 => 4096.0,  // ±8g
            4 => 8192.0,  // ±4g
            2 => 16384.0, // ±2g
            _ => 2048.0,  // Default to ±16g
        };

        let gyro_scale = match self.settings.gyro_range {
            2000 => 16.4, // ±2000°/s
            1000 => 32.8, // ±1000°/s
            500 => 65.6,  // ±500°/s
            250 => 131.2, // ±250°/s
            _ => 16.4,    // Default to ±2000°/s
        };

        // Update moving averages
        for i in 0..3 {
            self.averages.accel[i] = (self.averages.accel[i] * (self.settings.samples_avg - 1)
                + (raw[i] as i32 - self.calibration.accel_offsets[i]))
                / self.settings.samples_avg;

            self.averages.gyro[i] = (self.averages.gyro[i] * (self.settings.samples_avg - 1)
                + (raw[i + 3] as i32 - self.calibration.gyro_offsets[i]))
                / self.settings.samples_avg;
        }

        // Convert to scaled values
        let values = vec![
            (
                "accel_x".to_string(),
                self.averages.accel[0] as f64 / accel_scale,
            ),
            (
                "accel_y".to_string(),
                self.averages.accel[1] as f64 / accel_scale,
            ),
            (
                "accel_z".to_string(),
                self.averages.accel[2] as f64 / accel_scale,
            ),
            (
                "gyro_x".to_string(),
                self.averages.gyro[0] as f64 / gyro_scale,
            ),
            (
                "gyro_y".to_string(),
                self.averages.gyro[1] as f64 / gyro_scale,
            ),
            (
                "gyro_z".to_string(),
                self.averages.gyro[2] as f64 / gyro_scale,
            ),
        ];

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
            "\n📊 {} Sensor Data @ {}\n",
            self.name,
            chrono::DateTime::from_timestamp_millis(data.timestamp)
                .unwrap()
                .format("%H:%M:%S.%3f")
        ));
        lines += 2;

        output.push_str("\n🎯 G-Forces:\n");
        lines += 2;
        for (key, value) in &data.values {
            match key.as_str() {
                "accel_x" => {
                    output.push_str(&format!("  Lateral: {:.2} G\n", value));
                    lines += 1;
                }
                "accel_y" => {
                    output.push_str(&format!("  Forward: {:.2} G\n", value));
                    lines += 1;
                }
                "accel_z" => {
                    output.push_str(&format!("  Vertical: {:.2} G\n", value));
                    lines += 1;
                }
                _ => {}
            }
        }

        output.push_str("\n🔄 Turn Rate (°/s):\n");
        lines += 2;
        for (key, value) in &data.values {
            match key.as_str() {
                "gyro_x" => {
                    output.push_str(&format!("  Roll: {:.2}\n", value));
                    lines += 1;
                }
                "gyro_y" => {
                    output.push_str(&format!("  Pitch: {:.2}\n", value));
                    lines += 1;
                }
                "gyro_z" => {
                    output.push_str(&format!("  Yaw: {:.2}\n", value));
                    lines += 1;
                }
                _ => {}
            }
        }

        if let Some((lean, bank)) = angles {
            output.push_str("\n📐 Angles:\n");
            output.push_str(&format!("  Lean: {:.2}°\n", lean));
            output.push_str(&format!("  Bank: {:.2}°\n", bank));
            lines += 4;
        }

        Ok((lines, Some(output)))
    }
}
