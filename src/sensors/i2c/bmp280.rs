//! BMP280 temperature + barometric pressure sensor (I2C, Linux only).
//!
//! Default I2C address: 0x76 (SDO → GND) or 0x77 (SDO → VCC).
//!
//! Derived fields: altitude (metres above sea level, std. atmosphere).

#![cfg(target_os = "linux")]

use anyhow::{Context, Result};
use chrono::Utc;
use embedded_hal::i2c::I2c;
use linux_embedded_hal::I2cdev;
use serde::Deserialize;
use std::collections::HashMap;

use crate::config::{ConnectionConfig, SensorConfig};
use crate::sensors::{FieldDescriptor, Sensor, SensorData, VizType};

// ---------------------------------------------------------------------------
// Register map (BMP280 datasheet §5.3)
// ---------------------------------------------------------------------------
const REG_ID: u8 = 0xD0;
const REG_RESET: u8 = 0xE0;
const REG_STATUS: u8 = 0xF3;
const REG_CTRL_MEAS: u8 = 0xF4;
const REG_CONFIG: u8 = 0xF5;
const REG_PRESS_MSB: u8 = 0xF7;
const REG_CALIB: u8 = 0x88; // 24 bytes of calibration data

const CHIP_ID: u8 = 0x58;

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct Bmp280Settings {
    #[serde(default = "default_sea_level_hpa")]
    pub sea_level_pressure_hpa: f64,
}

fn default_sea_level_hpa() -> f64 {
    1013.25
}

impl Default for Bmp280Settings {
    fn default() -> Self {
        Self {
            sea_level_pressure_hpa: default_sea_level_hpa(),
        }
    }
}

// ---------------------------------------------------------------------------
// Calibration coefficients
// ---------------------------------------------------------------------------

struct CalibCoeffs {
    dig_t1: u16,
    dig_t2: i16,
    dig_t3: i16,
    dig_p1: u16,
    dig_p2: i16,
    dig_p3: i16,
    dig_p4: i16,
    dig_p5: i16,
    dig_p6: i16,
    dig_p7: i16,
    dig_p8: i16,
    dig_p9: i16,
}

impl CalibCoeffs {
    fn from_bytes(raw: &[u8; 24]) -> Self {
        let u16le = |lo: usize| u16::from_le_bytes([raw[lo], raw[lo + 1]]);
        let i16le = |lo: usize| i16::from_le_bytes([raw[lo], raw[lo + 1]]);
        Self {
            dig_t1: u16le(0),
            dig_t2: i16le(2),
            dig_t3: i16le(4),
            dig_p1: u16le(6),
            dig_p2: i16le(8),
            dig_p3: i16le(10),
            dig_p4: i16le(12),
            dig_p5: i16le(14),
            dig_p6: i16le(16),
            dig_p7: i16le(18),
            dig_p8: i16le(20),
            dig_p9: i16le(22),
        }
    }

    /// Returns (temperature °C,  t_fine for pressure compensation)
    fn compensate_temperature(&self, adc_t: i32) -> (f64, i32) {
        let t1 = self.dig_t1 as i32;
        let t2 = self.dig_t2 as i32;
        let t3 = self.dig_t3 as i32;
        let var1 = (adc_t / 8 - t1 * 2) * t2 / 2048;
        let tmp = adc_t / 16 - t1;
        let var2 = tmp * tmp / 4096 * t3 / 16384;
        let t_fine = var1 + var2;
        let temp = (t_fine * 5 + 128) / 256;
        (temp as f64 / 100.0, t_fine)
    }

    /// Returns pressure in Pa
    fn compensate_pressure(&self, adc_p: i32, t_fine: i32) -> f64 {
        let p1 = self.dig_p1 as i64;
        let mut var1 = t_fine as i64 - 128000;
        let mut var2 = var1 * var1 * self.dig_p6 as i64;
        var2 += (var1 * self.dig_p5 as i64) * 131072;
        var2 += (self.dig_p4 as i64) * 34359738368;
        var1 = var1 * var1 * self.dig_p3 as i64 / 256 + var1 * self.dig_p2 as i64 * 4096;
        var1 = (140737488355328_i64 + var1) * p1 / 8589934592;
        if var1 == 0 {
            return 0.0;
        }
        let mut p = 1048576_i64 - adc_p as i64;
        p = ((p * 2147483648 - var2) * 3125) / var1;
        var1 = (self.dig_p9 as i64) * (p / 8192) * (p / 8192) / 33554432;
        var2 = (self.dig_p8 as i64) * p / 524288;
        p = p + (var1 + var2 + self.dig_p7 as i64) / 16;
        p as f64 / 100.0 // → hPa
    }
}

// ---------------------------------------------------------------------------
// Driver struct
// ---------------------------------------------------------------------------

static FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        key: "temperature",
        label: "Temperature",
        viz: VizType::Numeric { unit: "°C" },
        group: Some("BMP280"),
    },
    FieldDescriptor {
        key: "pressure_hpa",
        label: "Pressure",
        viz: VizType::Numeric { unit: "hPa" },
        group: None,
    },
    FieldDescriptor {
        key: "altitude_m",
        label: "Altitude",
        viz: VizType::Numeric { unit: "m" },
        group: None,
    },
];

pub struct Bmp280 {
    name: String,
    device: I2cdev,
    address: u8,
    settings: Bmp280Settings,
    enabled: bool,
    calib: Option<CalibCoeffs>,
}

impl Bmp280 {
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let conn = match &cfg.connection {
            ConnectionConfig::I2c(c) => c.clone(),
            _ => anyhow::bail!("BMP280 requires an I2C connection"),
        };
        let settings: Bmp280Settings = cfg
            .settings
            .as_ref()
            .map(|v| v.clone().try_into())
            .transpose()?
            .unwrap_or_default();
        let device = I2cdev::new(&conn.device).context("opening I2C device for BMP280")?;
        Ok(Self {
            name: cfg.name.clone(),
            device,
            address: conn.address as u8,
            settings,
            enabled: cfg.enabled,
            calib: None,
        })
    }

    fn read_reg(&mut self, reg: u8) -> Result<u8> {
        let mut buf = [0u8; 1];
        self.device.write_read(self.address, &[reg], &mut buf)?;
        Ok(buf[0])
    }

    fn read_regs(&mut self, reg: u8, out: &mut [u8]) -> Result<()> {
        self.device.write_read(self.address, &[reg], out)?;
        Ok(())
    }

    fn write_reg(&mut self, reg: u8, val: u8) -> Result<()> {
        self.device.write(self.address, &[reg, val])?;
        Ok(())
    }
}

impl Sensor for Bmp280 {
    fn init(&mut self) -> Result<()> {
        let id = self.read_reg(REG_ID)?;
        if id != CHIP_ID {
            anyhow::bail!(
                "BMP280: unexpected chip id {:#04x} (expected {:#04x})",
                id,
                CHIP_ID
            );
        }
        // Soft-reset
        self.write_reg(REG_RESET, 0xB6)?;
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Read calibration (24 bytes from 0x88)
        let mut raw = [0u8; 24];
        self.read_regs(REG_CALIB, &mut raw)?;
        self.calib = Some(CalibCoeffs::from_bytes(&raw));

        // Normal mode, osrs_t×2, osrs_p×16
        self.write_reg(REG_CTRL_MEAS, 0b_101_101_11)?;
        // t_standby=500ms, filter×16, SPI 4-wire
        self.write_reg(REG_CONFIG, 0b_100_101_00)?;
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let calib = self
            .calib
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("BMP280 not initialised"))?;

        // Read 6 bytes: press MSB…LSB+xlsb, temp MSB…xlsb
        let mut raw = [0u8; 6];
        self.read_regs(REG_PRESS_MSB, &mut raw)?;

        let adc_p = (raw[0] as i32) << 12 | (raw[1] as i32) << 4 | (raw[2] as i32) >> 4;
        let adc_t = (raw[3] as i32) << 12 | (raw[4] as i32) << 4 | (raw[5] as i32) >> 4;

        let (temperature, t_fine) = calib.compensate_temperature(adc_t);
        let pressure = calib.compensate_pressure(adc_p, t_fine);
        let altitude =
            44330.0 * (1.0 - (pressure / self.settings.sea_level_pressure_hpa).powf(1.0 / 5.255));

        let mut fields = HashMap::new();
        fields.insert("temperature".into(), temperature);
        fields.insert("pressure_hpa".into(), pressure);
        fields.insert("altitude_m".into(), altitude);
        Ok(SensorData {
            timestamp: Utc::now(),
            fields,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
    fn driver_name(&self) -> &str {
        "bmp280"
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
