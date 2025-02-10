use anyhow::{Result, Context};
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use serde::Deserialize;
use super::I2CDevice;
use crate::sensors::{Sensor, SensorData};

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
}

impl Sensor for MPU6500 {
    fn init(&mut self) -> Result<()> {
        // Wake up the device
        self.i2c.write(self.address, &[0x6B, 0x00])?;
        
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
        
        self.i2c.write(self.address, &[ACCEL_CONFIG, accel_config])?;
        self.i2c.write(self.address, &[GYRO_CONFIG, gyro_config])?;
        
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let raw = self.read_raw()?;
        
        // Apply offsets and scaling
        let accel_scale = match self.settings.accel_range {
            16 => 2048.0,   // ±16g
            8 => 4096.0,    // ±8g
            4 => 8192.0,    // ±4g
            2 => 16384.0,   // ±2g
            _ => 2048.0,    // Default to ±16g
        };
        
        let gyro_scale = match self.settings.gyro_range {
            2000 => 16.4,   // ±2000°/s
            1000 => 32.8,   // ±1000°/s
            500 => 65.6,    // ±500°/s
            250 => 131.2,   // ±250°/s
            _ => 16.4,      // Default to ±2000°/s
        };
        
        // Update moving averages
        for i in 0..3 {
            self.averages.accel[i] = (self.averages.accel[i] * (self.settings.samples_avg - 1) + 
                (raw[i] as i32 - self.calibration.accel_offsets[i])) / self.settings.samples_avg;
            
            self.averages.gyro[i] = (self.averages.gyro[i] * (self.settings.samples_avg - 1) + 
                (raw[i + 3] as i32 - self.calibration.gyro_offsets[i])) / self.settings.samples_avg;
        }
        
        // Convert to scaled values
        let values = vec![
            ("accel_x".to_string(), self.averages.accel[0] as f64 / accel_scale),
            ("accel_y".to_string(), self.averages.accel[1] as f64 / accel_scale),
            ("accel_z".to_string(), self.averages.accel[2] as f64 / accel_scale),
            ("gyro_x".to_string(), self.averages.gyro[0] as f64 / gyro_scale),
            ("gyro_y".to_string(), self.averages.gyro[1] as f64 / gyro_scale),
            ("gyro_z".to_string(), self.averages.gyro[2] as f64 / gyro_scale),
        ];

        Ok(SensorData {
            timestamp: chrono::Utc::now().timestamp_millis(),
            values,
        })
    }

    fn get_info(&self) -> Result<String> {
        Ok(format!("MPU6500 IMU (addr: 0x{:02X}) - Accel: ±{}g, Gyro: ±{}°/s", 
            self.address,
            self.settings.accel_range,
            self.settings.gyro_range
        ))
    }
}