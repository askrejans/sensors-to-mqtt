//! SHT31 — high-accuracy temperature + relative humidity (I2C, Linux only).
//! Sensirion SHT3x-DIS datasheet V5.
//!
//! Default address: 0x44 (ADDR pin low), 0x45 (ADDR pin high).
//! Uses one-shot mode with high repeatability.

#![cfg(target_os = "linux")]

use anyhow::{Context, Result};
use chrono::Utc;
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use std::collections::HashMap;

use crate::config::{ConnectionConfig, SensorConfig};
use crate::sensors::{FieldDescriptor, Sensor, SensorData, VizType};

// One-shot, high repeatability, clock-stretching disabled
const CMD_MEAS: [u8; 2] = [0x24, 0x00];

static FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        key: "temperature",
        label: "Temperature",
        viz: VizType::Numeric { unit: "°C" },
        group: Some("SHT31"),
    },
    FieldDescriptor {
        key: "humidity_pct",
        label: "Humidity",
        viz: VizType::Numeric { unit: "%" },
        group: None,
    },
];

/// CRC-8 (poly 0x31, init 0xFF) per Sensirion AN article.
fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc
}

pub struct Sht31 {
    name: String,
    device: I2cdev,
    address: u8,
    enabled: bool,
}

impl Sht31 {
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let conn = match &cfg.connection {
            ConnectionConfig::I2c(c) => c.clone(),
            _ => anyhow::bail!("SHT31 requires an I2C connection"),
        };
        let device = I2cdev::new(&conn.device).context("opening I2C device for SHT31")?;
        Ok(Self {
            name: cfg.name.clone(),
            device,
            address: conn.address as u8,
            enabled: cfg.enabled,
        })
    }
}

impl Sensor for Sht31 {
    fn init(&mut self) -> Result<()> {
        // Soft-reset
        self.device.write(self.address, &[0x30, 0xA2])?;
        std::thread::sleep(std::time::Duration::from_millis(2));
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        // Trigger measurement
        self.device.write(self.address, &CMD_MEAS)?;
        // SHT31 needs ≥15ms for high-repeatability measurement
        std::thread::sleep(std::time::Duration::from_millis(20));

        let mut raw = [0u8; 6];
        self.device.read(self.address, &mut raw)?;

        // CRC check
        if crc8(&raw[0..2]) != raw[2] {
            anyhow::bail!("SHT31: temperature CRC mismatch");
        }
        if crc8(&raw[3..5]) != raw[5] {
            anyhow::bail!("SHT31: humidity CRC mismatch");
        }

        let t_raw = u16::from_be_bytes([raw[0], raw[1]]) as f64;
        let h_raw = u16::from_be_bytes([raw[3], raw[4]]) as f64;

        // Formulae from Sensirion datasheet §4.13
        let temperature = -45.0 + 175.0 * t_raw / 65535.0;
        let humidity = 100.0 * h_raw / 65535.0;

        let mut fields = HashMap::new();
        fields.insert("temperature".into(), temperature);
        fields.insert("humidity_pct".into(), humidity);
        Ok(SensorData {
            timestamp: Utc::now(),
            fields,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
    fn driver_name(&self) -> &str {
        "sht31"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }
    fn set_enabled(&mut self, e: bool) {
        self.enabled = e;
    }
    fn field_descriptors(&self) -> &[FieldDescriptor] {
        FIELDS
    }
}
