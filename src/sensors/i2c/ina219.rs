//! INA219 — bi-directional current / power monitor (I2C, Linux only).
//! Texas Instruments INA219 datasheet SBOS448G.
//!
//! Default address: 0x40.  Measures bus voltage, shunt voltage, current,
//! and power.  Requires knowing the shunt resistor value (Ω) and the
//! maximum expected current (A) — both configured in TOML `[settings]`.
//!
//! Derived field: `state_of_charge_pct` when battery_min_v / battery_max_v
//! are provided.

#![cfg(target_os = "linux")]

use anyhow::{Context, Result};
use chrono::Utc;
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use serde::Deserialize;
use std::collections::HashMap;

use crate::config::{ConnectionConfig, SensorConfig};
use crate::sensors::{FieldDescriptor, Sensor, SensorData, VizType};

// Register addresses
const REG_CONFIGURATION: u8 = 0x00;
const REG_SHUNT_VOLTAGE: u8 = 0x01;
const REG_BUS_VOLTAGE: u8 = 0x02;
const REG_POWER: u8 = 0x03;
const REG_CURRENT: u8 = 0x04;
const REG_CALIBRATION: u8 = 0x05;

// Config register value: 32V range, ±320mV shunt, 12-bit×32 averaging (continuous)
const CONFIG_32V_320MV: u16 = 0x3FFF;

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct Ina219Settings {
    /// Shunt resistor value in ohms (e.g. 0.1 for a 100mΩ shunt)
    #[serde(default = "default_shunt")]
    pub shunt_ohms: f64,
    /// Maximum expected current in amps (used to set calibration register)
    #[serde(default = "default_max_current")]
    pub max_current_a: f64,
    /// Optional: battery min voltage for SoC estimation
    pub battery_min_v: Option<f64>,
    /// Optional: battery max voltage for SoC estimation
    pub battery_max_v: Option<f64>,
}

fn default_shunt() -> f64 {
    0.1
}
fn default_max_current() -> f64 {
    3.2
}

impl Default for Ina219Settings {
    fn default() -> Self {
        Self {
            shunt_ohms: default_shunt(),
            max_current_a: default_max_current(),
            battery_min_v: None,
            battery_max_v: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Field descriptors
// ---------------------------------------------------------------------------

static FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        key: "bus_voltage_v",
        label: "Bus Voltage",
        viz: VizType::Numeric { unit: "V" },
        group: Some("INA219"),
    },
    FieldDescriptor {
        key: "shunt_mv",
        label: "Shunt Voltage",
        viz: VizType::Numeric { unit: "mV" },
        group: None,
    },
    FieldDescriptor {
        key: "current_a",
        label: "Current",
        viz: VizType::Numeric { unit: "A" },
        group: None,
    },
    FieldDescriptor {
        key: "power_w",
        label: "Power",
        viz: VizType::Numeric { unit: "W" },
        group: None,
    },
    FieldDescriptor {
        key: "soc_pct",
        label: "State of Charge",
        viz: VizType::Numeric { unit: "%" },
        group: None,
    },
];

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

pub struct Ina219 {
    name: String,
    device: I2cdev,
    address: u8,
    settings: Ina219Settings,
    enabled: bool,
    current_lsb: f64, // amps per LSB from calibration
}

impl Ina219 {
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let conn = match &cfg.connection {
            ConnectionConfig::I2c(c) => c.clone(),
            _ => anyhow::bail!("INA219 requires an I2C connection"),
        };
        let settings: Ina219Settings = cfg
            .settings
            .as_ref()
            .map(|v| v.clone().try_into())
            .transpose()?
            .unwrap_or_default();
        let device = I2cdev::new(&conn.device).context("opening I2C device for INA219")?;
        Ok(Self {
            name: cfg.name.clone(),
            device,
            address: conn.address as u8,
            settings,
            enabled: cfg.enabled,
            current_lsb: 0.0,
        })
    }

    fn write_reg(&mut self, reg: u8, val: u16) -> Result<()> {
        let b = val.to_be_bytes();
        self.device.write(self.address, &[reg, b[0], b[1]])?;
        Ok(())
    }

    fn read_reg_u16(&mut self, reg: u8) -> Result<u16> {
        let mut b = [0u8; 2];
        self.device.write_read(self.address, &[reg], &mut b)?;
        Ok(u16::from_be_bytes(b))
    }

    fn read_reg_i16(&mut self, reg: u8) -> Result<i16> {
        Ok(self.read_reg_u16(reg)? as i16)
    }
}

impl Sensor for Ina219 {
    fn init(&mut self) -> Result<()> {
        // Calculate current LSB: max_current / 2^15
        self.current_lsb = self.settings.max_current_a / 32768.0;
        // Calibration register = 0.04096 / (current_lsb * shunt_ohms)
        let calib_val = (0.04096 / (self.current_lsb * self.settings.shunt_ohms)) as u16;

        self.write_reg(REG_CONFIGURATION, CONFIG_32V_320MV)?;
        self.write_reg(REG_CALIBRATION, calib_val)?;
        std::thread::sleep(std::time::Duration::from_millis(5));
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let shunt_raw = self.read_reg_i16(REG_SHUNT_VOLTAGE)?;
        let bus_raw = self.read_reg_u16(REG_BUS_VOLTAGE)?;
        let current_raw = self.read_reg_i16(REG_CURRENT)?;
        let power_raw = self.read_reg_u16(REG_POWER)?;

        // Bus voltage: bits [15:3] in 4mV LSBs
        let bus_voltage_v = ((bus_raw >> 3) as f64) * 0.004;
        // Shunt voltage: 10µV per LSB
        let shunt_mv = shunt_raw as f64 * 0.01;
        // Current: current_lsb per LSB
        let current_a = current_raw as f64 * self.current_lsb;
        // Power: 20 * current_lsb per LSB
        let power_w = power_raw as f64 * 20.0 * self.current_lsb;

        let soc_pct = match (self.settings.battery_min_v, self.settings.battery_max_v) {
            (Some(min), Some(max)) if max > min => {
                ((bus_voltage_v - min) / (max - min) * 100.0).clamp(0.0, 100.0)
            }
            _ => -1.0, // -1 = not configured
        };

        let mut fields = HashMap::new();
        fields.insert("bus_voltage_v".into(), bus_voltage_v);
        fields.insert("shunt_mv".into(), shunt_mv);
        fields.insert("current_a".into(), current_a);
        fields.insert("power_w".into(), power_w);
        fields.insert("soc_pct".into(), soc_pct);
        Ok(SensorData {
            timestamp: Utc::now(),
            fields,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
    fn driver_name(&self) -> &str {
        "ina219"
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
