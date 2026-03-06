//! BH1750 — ambient light sensor (I2C, Linux only).
//! ROHM Semiconductor BH1750FVI datasheet.
//!
//! Default address: 0x23 (ADDR pin low), 0x5C (ADDR pin high).
//! Reports illuminance in lux and derived categories.

#![cfg(target_os = "linux")]

use anyhow::{Context, Result};
use chrono::Utc;
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use std::collections::HashMap;

use crate::config::{ConnectionConfig, SensorConfig};
use crate::sensors::{FieldDescriptor, Sensor, SensorData, VizType};

// Measurement commands (datasheet §5)
const CMD_POWER_ON: u8 = 0x01;
const CMD_RESET: u8 = 0x07;
/// Continuously high-resolution mode: 1 lx resolution, 120 ms measurement time
const CMD_CONT_H_MEAS: u8 = 0x10;
/// One-time high-resolution mode (for sleeping between reads)
const _CMD_ONCE_H_MEAS: u8 = 0x20;

static FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        key: "lux",
        label: "Illuminance",
        viz: VizType::Numeric { unit: "lux" },
        group: Some("BH1750"),
    },
    FieldDescriptor {
        key: "lux_category",
        label: "Category",
        viz: VizType::Value,
        group: None,
    },
];

/// Coarse daylight category (0 = dark, 1 = dim, 2 = indoor, 3 = bright, 4 = direct sun)
fn lux_to_category(lux: f64) -> f64 {
    match lux as u32 {
        0..=10 => 0.0,       // dark / night
        11..=100 => 1.0,     // dim
        101..=1000 => 2.0,   // typical indoor
        1001..=10000 => 3.0, // bright / overcast outdoor
        _ => 4.0,            // direct sunlight
    }
}

pub struct Bh1750 {
    name: String,
    device: I2cdev,
    address: u8,
    enabled: bool,
}

impl Bh1750 {
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let conn = match &cfg.connection {
            ConnectionConfig::I2c(c) => c.clone(),
            _ => anyhow::bail!("BH1750 requires an I2C connection"),
        };
        let device = I2cdev::new(&conn.device).context("opening I2C device for BH1750")?;
        Ok(Self {
            name: cfg.name.clone(),
            device,
            address: conn.address as u8,
            enabled: cfg.enabled,
        })
    }
}

impl Sensor for Bh1750 {
    fn init(&mut self) -> Result<()> {
        self.device.write(self.address, &[CMD_POWER_ON])?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        self.device.write(self.address, &[CMD_RESET])?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        // Start continuous measurement
        self.device.write(self.address, &[CMD_CONT_H_MEAS])?;
        std::thread::sleep(std::time::Duration::from_millis(180));
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let mut raw = [0u8; 2];
        self.device.read(self.address, &mut raw)?;
        let raw_val = u16::from_be_bytes(raw) as f64;
        // BH1750 datasheet: lux = raw / 1.2
        let lux = raw_val / 1.2;

        let mut fields = HashMap::new();
        fields.insert("lux".into(), lux);
        fields.insert("lux_category".into(), lux_to_category(lux));
        Ok(SensorData {
            timestamp: Utc::now(),
            fields,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
    fn driver_name(&self) -> &str {
        "bh1750"
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
