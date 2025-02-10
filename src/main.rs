use std::{thread, time::Duration};
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use anyhow::{Result, Context};

const I2C_BUS: &str = "/dev/i2c-1";
const DEVICE_ADDR: u16 = 0x68;

// Register addresses
const ACCEL_CONFIG: u8 = 0x1C;
const GYRO_CONFIG: u8 = 0x1B;
const ACCEL_XOUT_H: u8 = 0x3B;
const ACCEL_YOUT_H: u8 = 0x3D;
const ACCEL_ZOUT_H: u8 = 0x3F;
const GYRO_XOUT_H: u8 = 0x43;
const GYRO_YOUT_H: u8 = 0x45;
const GYRO_ZOUT_H: u8 = 0x47;

// Constants
const ACCEL_SENSITIVITY: f64 = 2048.0; // ±16g
const GYRO_SENSITIVITY: f64 = 16.4;    // ±2000°/s
const SAMPLES: i32 = 10;
const CALIBRATION_SAMPLES: i32 = 300;

struct Imu {
    i2c: I2cdev,
    accel_offsets: [i32; 3],
    gyro_offsets: [i32; 3],
    accel_avg: [i32; 3],
    gyro_avg: [i32; 3],
}

impl Imu {
    fn new() -> Result<Self> {
        let i2c = I2cdev::new(I2C_BUS).context("Failed to open I2C device")?;
        Ok(Self {
            i2c,
            accel_offsets: [0; 3],
            gyro_offsets: [0; 3],
            accel_avg: [0; 3],
            gyro_avg: [0; 3],
        })
    }

    fn read_sensor(&mut self, register: u8) -> Result<i16> {
        let mut buf = [0u8; 2];
        self.i2c.write_read(DEVICE_ADDR, &[register], &mut buf)?;
        Ok(i16::from_be_bytes(buf))
    }

    fn init(&mut self) -> Result<()> {
        // Wake up the device
        self.i2c.write(DEVICE_ADDR, &[0x6B, 0x00])?;
        // Configure accelerometer and gyroscope
        self.i2c.write(DEVICE_ADDR, &[ACCEL_CONFIG, 0x18])?;
        self.i2c.write(DEVICE_ADDR, &[GYRO_CONFIG, 0x18])?;
        Ok(())
    }

    fn calibrate(&mut self) -> Result<()> {
        println!("🔧 Calibrating... Keep still");
        
        let mut accel_sums = [0i32; 3];
        let mut gyro_sums = [0i32; 3];

        for _ in 0..CALIBRATION_SAMPLES {
            let readings = self.read_all_sensors()?;
            for i in 0..3 {
                accel_sums[i] += readings[i] as i32;
                gyro_sums[i] += readings[i + 3] as i32;
            }
            thread::sleep(Duration::from_millis(10));
        }

        for i in 0..3 {
            self.accel_offsets[i] = accel_sums[i] / CALIBRATION_SAMPLES;
            self.gyro_offsets[i] = gyro_sums[i] / CALIBRATION_SAMPLES;
        }
        // Adjust Z acceleration offset
        self.accel_offsets[2] -= 2048;

        println!("✅ Calibration complete!");
        Ok(())
    }

    fn read_all_sensors(&mut self) -> Result<[i16; 6]> {
        Ok([
            self.read_sensor(ACCEL_XOUT_H)?,
            self.read_sensor(ACCEL_YOUT_H)?,
            self.read_sensor(ACCEL_ZOUT_H)?,
            self.read_sensor(GYRO_XOUT_H)?,
            self.read_sensor(GYRO_YOUT_H)?,
            self.read_sensor(GYRO_ZOUT_H)?,
        ])
    }

    fn update(&mut self) -> Result<()> {
        let raw = self.read_all_sensors()?;
        
        // Apply offsets
        let accel_raw = [
            raw[0] as i32 - self.accel_offsets[0], // X
            raw[1] as i32 - self.accel_offsets[1], // Y
            raw[2] as i32 - self.accel_offsets[2], // Z
        ];
        
        let gyro_raw = [
            raw[3] as i32 - self.gyro_offsets[0], // X
            raw[4] as i32 - self.gyro_offsets[1], // Y
            raw[5] as i32 - self.gyro_offsets[2], // Z
        ];

        // Update moving averages
        for i in 0..3 {
            self.accel_avg[i] = (self.accel_avg[i] * (SAMPLES - 1) + accel_raw[i]) / SAMPLES;
            self.gyro_avg[i] = (self.gyro_avg[i] * (SAMPLES - 1) + gyro_raw[i]) / SAMPLES;
        }

        self.display_data();
        Ok(())
    }

    fn display_data(&self) {
        // Convert to scaled values
        let accel = [
            self.accel_avg[0] as f64 / ACCEL_SENSITIVITY,
            self.accel_avg[1] as f64 / ACCEL_SENSITIVITY,
            self.accel_avg[2] as f64 / ACCEL_SENSITIVITY,
        ];
        
        let gyro = [
            self.gyro_avg[0] as f64 / GYRO_SENSITIVITY,
            self.gyro_avg[1] as f64 / GYRO_SENSITIVITY,
            self.gyro_avg[2] as f64 / GYRO_SENSITIVITY,
        ];

        // Calculate angles
        let ax2 = accel[0] * accel[0];
        let az2 = accel[2] * accel[2];
        let lean_angle = (accel[1] / (ax2 + az2).sqrt()).atan() * 180.0 / std::f64::consts::PI;
        let bank_angle = (accel[0] / accel[2].abs()).atan() * 180.0 / std::f64::consts::PI;

        print!("\x1B[2J\x1B[1;1H"); // Clear screen
        println!("G86Racing IMU");
        println!("==================");
        println!("📊 Raw Data:");
        println!("Accelerometer:");
        println!("  X: {}  Y: {}  Z: {}", self.accel_avg[0], self.accel_avg[1], self.accel_avg[2]);
        println!("Gyroscope:");
        println!("  X: {}  Y: {}  Z: {}", self.gyro_avg[0], self.gyro_avg[1], self.gyro_avg[2]);
        println!("");
        println!("🎯 G-Forces:");
        println!("  Lateral: {:.2} G", accel[0]);
        println!("  Forward: {:.2} G", accel[1]);
        println!("  Vertical: {:.2} G", accel[2]);
        println!("");
        println!("🔄 Turn Rate (°/s):");
        println!("  Roll: {:.2}", gyro[0]);
        println!("  Pitch: {:.2}", gyro[1]);
        println!("  Yaw: {:.2}", gyro[2]);
        println!("");
        println!("📐 Angles:");
        println!("  Lean: {:.2}°", lean_angle);
        println!("  Bank: {:.2}°", bank_angle);
        println!("==================");
    }
}

fn main() -> Result<()> {
    let mut imu = Imu::new()?;
    imu.init()?;
    imu.calibrate()?;

    loop {
        imu.update()?;
        thread::sleep(Duration::from_millis(10));
    }
}