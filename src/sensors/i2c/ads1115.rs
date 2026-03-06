//! ADS1115 — 4-channel 16-bit precision ADC (I2C, Linux only).
//! Texas Instruments ADS1115 datasheet SBAS444E.
//!
//! Default address: 0x48 (ADDR→GND). Addresses: 0x48, 0x49, 0x4A, 0x4B.
//!
//! Reports all 4 single-ended input channels as voltages (V) plus
//! optional scaled/named derived values configurable via TOML settings.
//!
//! TOML settings example:
//! ```toml
//! [sensors.settings]
//! gain = 2.048              # PGA full-scale range (V): 6.144|4.096|2.048|1.024|0.512|0.256
//! sample_rate = 128         # SPS: 8|16|32|64|128|250|475|860
//!
//! # Optional per-channel linear mappings: value = (volts - offset) * scale
//! [[sensors.settings.channels]]
//! index  = 0
//! label  = "Battery Voltage"
//! unit   = "V"
//! scale  = 5.7        # voltage divider ratio (e.g. (R1+R2)/R2)
//! offset = 0.0
//!
//! [[sensors.settings.channels]]
//! index  = 1
//! label  = "Throttle"
//! unit   = "%"
//! scale  = 25.0      # 4V → 100%
//! offset = 0.0
//! ```

#![cfg(target_os = "linux")]

use anyhow::{Context, Result};
use chrono::Utc;
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use serde::Deserialize;
use std::collections::HashMap;

use crate::config::{ConnectionConfig, SensorConfig};
use crate::sensors::{FieldDescriptor, Sensor, SensorData, VizType};

// Registers
const REG_CONVERSION: u8 = 0x00;
const REG_CONFIG: u8 = 0x01;

// Config register bit fields (see Table 9 in datasheet)
// [15]   OS = 1 start single conversion
// [14:12] MUX  AIN0..3 vs GND = 100..111
// [11:9] PGA
// [8]    MODE = 1 (single-shot)
// [7:5]  DR data rate
// [4:0]  comparator settings (disabled)

fn pga_for(fsr: f64) -> (u16, f64) {
    match (fsr * 1000.0) as u32 {
        0..=256 => (0b111 << 9, 0.256),
        257..=512 => (0b110 << 9, 0.512),
        513..=1024 => (0b101 << 9, 1.024),
        1025..=2048 => (0b010 << 9, 2.048),
        2049..=4096 => (0b001 << 9, 4.096),
        _ => (0b000 << 9, 6.144),
    }
}

fn dr_for(sps: u32) -> u16 {
    match sps {
        0..=8 => 0b000 << 5,
        9..=16 => 0b001 << 5,
        17..=32 => 0b010 << 5,
        33..=64 => 0b011 << 5,
        65..=128 => 0b100 << 5,
        129..=250 => 0b101 << 5,
        251..=475 => 0b110 << 5,
        _ => 0b111 << 5,
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ChannelConfig {
    pub index: u8,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub offset: f64,
}
fn default_scale() -> f64 {
    1.0
}

#[derive(Debug, Deserialize, Clone)]
pub struct Ads1115Settings {
    /// PGA full-scale range in volts (default 4.096 covers 0..4V on 3.3V supply)
    #[serde(default = "default_gain")]
    pub gain: f64,
    /// Output data rate in SPS (default 128)
    #[serde(default = "default_sps")]
    pub sample_rate: u32,
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,
}
fn default_gain() -> f64 {
    4.096
}
fn default_sps() -> u32 {
    128
}

impl Default for Ads1115Settings {
    fn default() -> Self {
        Self {
            gain: default_gain(),
            sample_rate: default_sps(),
            channels: vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

pub struct Ads1115 {
    name: String,
    device: I2cdev,
    address: u8,
    settings: Ads1115Settings,
    enabled: bool,
    fsr_v: f64,
    pga_bits: u16,
    dr_bits: u16,
    /// Dynamically-built field descriptors (allocated once in init)
    field_descs: Vec<FieldDescriptor>,
}

impl Ads1115 {
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let conn = match &cfg.connection {
            ConnectionConfig::I2c(c) => c.clone(),
            _ => anyhow::bail!("ADS1115 requires an I2C connection"),
        };
        let settings: Ads1115Settings = cfg
            .settings
            .as_ref()
            .map(|v| v.clone().try_into())
            .transpose()?
            .unwrap_or_default();
        let device = I2cdev::new(&conn.device).context("opening I2C device for ADS1115")?;

        let (pga_bits, fsr_v) = pga_for(settings.gain);
        let dr_bits = dr_for(settings.sample_rate);

        Ok(Self {
            name: cfg.name.clone(),
            device,
            address: conn.address as u8,
            settings,
            enabled: cfg.enabled,
            fsr_v,
            pga_bits,
            dr_bits,
            field_descs: Vec::new(),
        })
    }

    /// Read one channel in single-shot mode; returns raw i16 code.
    fn read_channel(&mut self, ch: u8) -> Result<i16> {
        let mux = ((ch as u16 & 0x3) + 4) << 12; // AIN0..3 vs GND
        let config: u16 = 0x8000         // OS: start conversion
            | mux
            | self.pga_bits
            | 0x0100                     // MODE: single-shot
            | self.dr_bits
            | 0x0003; // comparator disabled

        let b = config.to_be_bytes();
        self.device.write(self.address, &[REG_CONFIG, b[0], b[1]])?;

        // Wait for conversion (1/SPS * 1.1 headroom, ≥1ms)
        let wait_us = (1_100_000u64 / self.settings.sample_rate.max(1) as u64).max(1000);
        std::thread::sleep(std::time::Duration::from_micros(wait_us));

        let mut rb = [0u8; 2];
        self.device
            .write_read(self.address, &[REG_CONVERSION], &mut rb)?;
        Ok(i16::from_be_bytes(rb))
    }
}

// Use Box<str> for owned labels/units that can live as 'static-ish references.
// Since FieldDescriptor uses &'static str, we generate descriptors with
// leaked strings (safe: program lifetime, created once per sensor).
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

impl Sensor for Ads1115 {
    fn init(&mut self) -> Result<()> {
        // Build field descriptors (raw + mapped)
        let mut descs: Vec<FieldDescriptor> = Vec::new();

        for ch in 0..4u8 {
            let key = leak(format!("ch{}_v", ch));
            let label_raw = leak(format!("CH{} Voltage", ch));
            descs.push(FieldDescriptor {
                key,
                label: label_raw,
                viz: VizType::Numeric { unit: "V" },
                group: if ch == 0 { Some("ADS1115") } else { None },
            });

            // Mapped channel, if configured
            if let Some(cc) = self.settings.channels.iter().find(|c| c.index == ch) {
                let key_m = leak(format!("ch{}_mapped", ch));
                let label_m = leak(cc.label.clone());
                let unit_m: &'static str = leak(cc.unit.clone());
                descs.push(FieldDescriptor {
                    key: key_m,
                    label: label_m,
                    viz: VizType::Numeric { unit: unit_m },
                    group: None,
                });
            }
        }
        self.field_descs = descs;
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let mut fields = HashMap::new();
        let fsr = self.fsr_v;

        for ch in 0..4u8 {
            let raw = self.read_channel(ch)?;
            // Full-scale ±FSR maps to ±32767
            let volts = raw as f64 * fsr / 32767.0;
            fields.insert(format!("ch{}_v", ch), volts);

            if let Some(cc) = self.settings.channels.iter().find(|c| c.index == ch) {
                let mapped = (volts - cc.offset) * cc.scale;
                fields.insert(format!("ch{}_mapped", ch), mapped);
            }
        }

        Ok(SensorData {
            timestamp: Utc::now(),
            fields,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
    fn driver_name(&self) -> &str {
        "ads1115"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }
    fn set_enabled(&mut self, e: bool) {
        self.enabled = e;
    }
    fn field_descriptors(&self) -> &[FieldDescriptor] {
        &self.field_descs
    }
}
