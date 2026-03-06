//! BME280 — temperature, barometric pressure, and humidity (I2C, Linux only).
//! Bosch Sensortec datasheet BST-BME280-DS002.
//!
//! Chip-ID: 0x60.  Default I2C addresses: 0x76 (SDO→GND) or 0x77 (SDO→VCC).
//!
//! Derived fields: altitude_m (barometric, std atmosphere).

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
// Register map
// ---------------------------------------------------------------------------
const REG_ID: u8 = 0xD0;
const REG_RESET: u8 = 0xE0;
const REG_CTRL_HUM: u8 = 0x72;
const REG_CTRL_MEAS: u8 = 0xF4;
const REG_CONFIG: u8 = 0xF5;
const REG_PRESS_MSB: u8 = 0xF7; // 8 bytes: P(3) T(3) H(2)
const REG_CALIB_TP: u8 = 0x88; // 26 bytes T+P calibration
const REG_CALIB_H1: u8 = 0xA1; // 1 byte  dig_H1
const REG_CALIB_H2: u8 = 0xE1; // 7 bytes dig_H2..H6

const CHIP_ID: u8 = 0x60;

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct Bme280Settings {
    #[serde(default = "default_sea_level_hpa")]
    pub sea_level_pressure_hpa: f64,
}
fn default_sea_level_hpa() -> f64 {
    1013.25
}
impl Default for Bme280Settings {
    fn default() -> Self {
        Self {
            sea_level_pressure_hpa: default_sea_level_hpa(),
        }
    }
}

// ---------------------------------------------------------------------------
// Calibration
// ---------------------------------------------------------------------------

struct Calib {
    t1: u16,
    t2: i16,
    t3: i16,
    p1: u16,
    p2: i16,
    p3: i16,
    p4: i16,
    p5: i16,
    p6: i16,
    p7: i16,
    p8: i16,
    p9: i16,
    h1: u8,
    h2: i16,
    h3: u8,
    h4: i16,
    h5: i16,
    h6: i8,
}

impl Calib {
    fn from_bytes(tp: &[u8; 26], h1: u8, hx: &[u8; 7]) -> Self {
        let u16l = |o: usize| u16::from_le_bytes([tp[o], tp[o + 1]]);
        let i16l = |o: usize| i16::from_le_bytes([tp[o], tp[o + 1]]);
        Self {
            t1: u16l(0),
            t2: i16l(2),
            t3: i16l(4),
            p1: u16l(6),
            p2: i16l(8),
            p3: i16l(10),
            p4: i16l(12),
            p5: i16l(14),
            p6: i16l(16),
            p7: i16l(18),
            p8: i16l(20),
            p9: i16l(22),
            h1,
            h2: i16::from_le_bytes([hx[0], hx[1]]),
            h3: hx[2],
            h4: (hx[3] as i16) << 4 | (hx[4] as i16 & 0x0F),
            h5: ((hx[4] as i16 & 0xF0) >> 4) | (hx[5] as i16) << 4,
            h6: hx[6] as i8,
        }
    }

    /// Returns (temperature °C, t_fine)
    fn comp_temp(&self, adc_t: i32) -> (f64, i64) {
        let t1 = self.t1 as i64;
        let t2 = self.t2 as i64;
        let t3 = self.t3 as i64;
        let var1 = ((adc_t as i64 >> 3) - (t1 << 1)) * t2 >> 11;
        let v2t = (adc_t as i64 >> 4) - t1;
        let var2 = v2t * v2t / 4096 * t3 / 16384;
        let t_fine = var1 + var2;
        ((t_fine * 5 + 128) as f64 / 25600.0, t_fine)
    }

    /// Returns pressure in hPa
    fn comp_press(&self, adc_p: i32, t_fine: i64) -> f64 {
        let p1 = self.p1 as i64;
        let mut v1: i64 = t_fine - 128000;
        let mut v2: i64 = v1 * v1 * self.p6 as i64;
        v2 += (v1 * self.p5 as i64) << 17;
        v2 += (self.p4 as i64) << 35;
        v1 = v1 * v1 * self.p3 as i64 / 256 + v1 * self.p2 as i64 * 4096;
        v1 = (((1_i64) << 47) + v1) * p1 >> 33;
        if v1 == 0 {
            return 0.0;
        }
        let mut p = 1048576_i64 - adc_p as i64;
        p = ((p << 31) - v2) * 3125 / v1;
        v1 = (self.p9 as i64) * (p >> 13) * (p >> 13) >> 25;
        v2 = (self.p8 as i64) * p >> 19;
        p = (p + v1 + v2) / 256 + ((self.p7 as i64) << 4);
        p as f64 / 25600.0 // → hPa
    }

    /// Returns relative humidity %
    fn comp_humid(&self, adc_h: i32, t_fine: i64) -> f64 {
        let x = t_fine - 76800_i64;
        let h4 = self.h4 as i64;
        let h5 = self.h5 as i64;
        let h6 = self.h6 as i64;
        let h2 = self.h2 as i64;
        let h1 = self.h1 as i64;
        let h3 = self.h3 as i64;

        let v = ((adc_h as i64) << 14) - (h4 << 20) - h5 * x;
        let v = v + 16384;
        let v = (v >> 15)
            * (((((v * h6 / 4096) >> 10) * (v * h3 / 2048 + 32768) / 1024 + 2097152) * h2 + 8192)
                >> 14);
        let v = v - ((((v >> 15) * (v >> 15)) >> 7) * h1 >> 4);
        let v = v.clamp(0, 419430400);
        (v >> 12) as f64 / 1024.0
    }
}

// ---------------------------------------------------------------------------
// Field descriptors
// ---------------------------------------------------------------------------

static FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        key: "temperature",
        label: "Temperature",
        viz: VizType::Numeric { unit: "°C" },
        group: Some("BME280"),
    },
    FieldDescriptor {
        key: "pressure_hpa",
        label: "Pressure",
        viz: VizType::Numeric { unit: "hPa" },
        group: None,
    },
    FieldDescriptor {
        key: "humidity_pct",
        label: "Humidity",
        viz: VizType::Numeric { unit: "%" },
        group: None,
    },
    FieldDescriptor {
        key: "altitude_m",
        label: "Altitude",
        viz: VizType::Numeric { unit: "m" },
        group: None,
    },
];

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

pub struct Bme280 {
    name: String,
    device: I2cdev,
    address: u8,
    settings: Bme280Settings,
    enabled: bool,
    calib: Option<Calib>,
}

impl Bme280 {
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let conn = match &cfg.connection {
            ConnectionConfig::I2c(c) => c.clone(),
            _ => anyhow::bail!("BME280 requires an I2C connection"),
        };
        let settings: Bme280Settings = cfg
            .settings
            .as_ref()
            .map(|v| v.clone().try_into())
            .transpose()?
            .unwrap_or_default();
        let device = I2cdev::new(&conn.device).context("opening I2C device for BME280")?;
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
        let mut b = [0u8];
        self.device.write_read(self.address, &[reg], &mut b)?;
        Ok(b[0])
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

impl Sensor for Bme280 {
    fn init(&mut self) -> Result<()> {
        let id = self.read_reg(REG_ID)?;
        if id != CHIP_ID {
            anyhow::bail!("BME280: unexpected chip id {:#04x}", id);
        }
        self.write_reg(REG_RESET, 0xB6)?;
        std::thread::sleep(std::time::Duration::from_millis(10));

        let mut tp = [0u8; 26];
        self.read_regs(REG_CALIB_TP, &mut tp)?;
        let h1 = self.read_reg(REG_CALIB_H1)?;
        let mut hx = [0u8; 7];
        self.read_regs(REG_CALIB_H2, &mut hx)?;
        self.calib = Some(Calib::from_bytes(&tp, h1, &hx));

        // humidity oversampling ×1 (must be set before ctrl_meas)
        self.write_reg(REG_CTRL_HUM, 0b001)?;
        // temp ×2, press ×16, normal mode
        self.write_reg(REG_CTRL_MEAS, 0b_101_101_11)?;
        // t_standby=500ms, filter×16
        self.write_reg(REG_CONFIG, 0b_100_101_00)?;
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let calib = self
            .calib
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("BME280 not initialised — call init() first"))?;

        let mut raw = [0u8; 8];
        self.read_regs(REG_PRESS_MSB, &mut raw)?;

        let adc_p = (raw[0] as i32) << 12 | (raw[1] as i32) << 4 | (raw[2] as i32) >> 4;
        let adc_t = (raw[3] as i32) << 12 | (raw[4] as i32) << 4 | (raw[5] as i32) >> 4;
        let adc_h = (raw[6] as i32) << 8 | raw[7] as i32;

        let (temperature, t_fine) = calib.comp_temp(adc_t);
        let pressure = calib.comp_press(adc_p, t_fine);
        let humidity = calib.comp_humid(adc_h, t_fine);
        let altitude =
            44330.0 * (1.0 - (pressure / self.settings.sea_level_pressure_hpa).powf(1.0 / 5.255));

        let mut fields = HashMap::new();
        fields.insert("temperature".into(), temperature);
        fields.insert("pressure_hpa".into(), pressure);
        fields.insert("humidity_pct".into(), humidity);
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
        "bme280"
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
