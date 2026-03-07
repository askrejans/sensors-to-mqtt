//! SDS011 laser PM2.5 / PM10 particulate-matter sensor (Nova Fitness).
//!
//! Supports two transports:
//! - **USB-serial** (e.g. `/dev/ttyUSB0`) via `type = "serial"` (Linux / macOS)
//! - **Serial-over-IP / raw TCP** (e.g. an io-to-net bridge) via `type = "tcp"`
//!
//! The sensor outputs a 10-byte binary frame continuously at ~1 Hz:
//!
//! ```text
//! AA  C0  D1  D2  D3  D4  ID1 ID2 CS  AB
//! ↑                               ↑       ↑
//! head                          tail  checksum = (D1+D2+D3+D4+ID1+ID2) & 0xFF
//! PM2.5 = (D2*256 + D1) / 10.0  μg/m³
//! PM10  = (D4*256 + D3) / 10.0  μg/m³
//! ```
//!
//! Configure with `driver = "sds011"` in `settings.toml`.

use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::collections::HashMap;
use std::io::Read;
use std::net::TcpStream;
use std::time::Duration;

use crate::config::{ConnectionConfig, SensorConfig};
use crate::sensors::{FieldDescriptor, Sensor, SensorData, VizType};

// ---------------------------------------------------------------------------
// Frame constants
// ---------------------------------------------------------------------------

const FRAME_LEN: usize = 10;
const HEAD: u8 = 0xAA;
const CMD: u8 = 0xC0;
const TAIL: u8 = 0xAB;

// ---------------------------------------------------------------------------
// Field descriptors
// ---------------------------------------------------------------------------

static FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        key: "pm2_5",
        label: "PM2.5",
        viz: VizType::Numeric { unit: "μg/m³" },
        group: Some("PARTICULATE MATTER"),
    },
    FieldDescriptor {
        key: "pm10",
        label: "PM10",
        viz: VizType::Numeric { unit: "μg/m³" },
        group: None,
    },
    FieldDescriptor {
        key: "aqi_pm2_5",
        label: "AQI (PM2.5)",
        viz: VizType::Numeric { unit: "" },
        group: Some("AIR QUALITY"),
    },
    FieldDescriptor {
        key: "aqi_pm10",
        label: "AQI (PM10)",
        viz: VizType::Numeric { unit: "" },
        group: None,
    },
];

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

pub struct Sds011 {
    name: String,
    /// Underlying byte stream — either a native serial port or a TCP socket.
    port: Box<dyn Read + Send>,
    enabled: bool,
    /// Byte buffer for partial frames
    buf: Vec<u8>,
}

impl Sds011 {
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let port: Box<dyn Read + Send> = match &cfg.connection {
            ConnectionConfig::Serial(c) => {
                let p = serialport::new(&c.port, c.baud_rate)
                    .timeout(Duration::from_millis(2000))
                    .open()
                    .with_context(|| format!("Failed to open serial port {}", c.port))?;
                Box::new(p)
            }
            ConnectionConfig::Tcp(c) => {
                let addr = format!("{}:{}", c.host, c.port);
                let stream = TcpStream::connect(&addr)
                    .with_context(|| format!("Failed to connect to TCP endpoint {}", addr))?;
                stream
                    .set_read_timeout(Some(Duration::from_millis(2000)))
                    .context("Failed to set TCP read timeout")?;
                Box::new(stream)
            }
            _ => bail!("SDS011 requires a 'serial' or 'tcp' connection"),
        };

        Ok(Self {
            name: cfg.name.clone(),
            port,
            enabled: cfg.enabled,
            buf: Vec::with_capacity(32),
        })
    }

    /// Read bytes until we have a complete, valid 10-byte frame.
    /// Discards bytes until we see a HEAD+CMD sequence to re-sync on noise.
    fn read_frame(&mut self) -> Result<[u8; FRAME_LEN]> {
        let mut tmp = [0u8; 16];
        loop {
            let n = self.port.read(&mut tmp).context("serial read")?;
            self.buf.extend_from_slice(&tmp[..n]);

            // Try to find a valid frame in the buffer
            while self.buf.len() >= FRAME_LEN {
                if self.buf[0] == HEAD && self.buf[1] == CMD && self.buf[9] == TAIL {
                    let checksum: u8 = self.buf[2..8]
                        .iter()
                        .fold(0u8, |acc, &b| acc.wrapping_add(b));
                    if checksum == self.buf[8] {
                        let mut frame = [0u8; FRAME_LEN];
                        frame.copy_from_slice(&self.buf[..FRAME_LEN]);
                        self.buf.drain(..FRAME_LEN);
                        return Ok(frame);
                    }
                }
                // Bad byte — discard and re-sync
                self.buf.remove(0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// US EPA AQI breakpoints for PM2.5 and PM10
// ---------------------------------------------------------------------------

/// Piecewise-linear AQI calculation.
fn aqi(c: f64, breakpoints: &[(f64, f64, f64, f64)]) -> f64 {
    for &(c_lo, c_hi, i_lo, i_hi) in breakpoints {
        if c <= c_hi {
            let c_clamped = c.max(c_lo);
            return (i_hi - i_lo) / (c_hi - c_lo) * (c_clamped - c_lo) + i_lo;
        }
    }
    500.0 // Beyond index
}

fn aqi_pm2_5(pm: f64) -> f64 {
    // Truncate to 1 decimal per EPA spec
    let c = (pm * 10.0).floor() / 10.0;
    aqi(
        c,
        &[
            (0.0, 12.0, 0.0, 50.0),
            (12.1, 35.4, 51.0, 100.0),
            (35.5, 55.4, 101.0, 150.0),
            (55.5, 150.4, 151.0, 200.0),
            (150.5, 250.4, 201.0, 300.0),
            (250.5, 350.4, 301.0, 400.0),
            (350.5, 500.4, 401.0, 500.0),
        ],
    )
}

fn aqi_pm10(pm: f64) -> f64 {
    let c = pm.floor();
    aqi(
        c,
        &[
            (0.0, 54.0, 0.0, 50.0),
            (55.0, 154.0, 51.0, 100.0),
            (155.0, 254.0, 101.0, 150.0),
            (255.0, 354.0, 151.0, 200.0),
            (355.0, 424.0, 201.0, 300.0),
            (425.0, 504.0, 301.0, 400.0),
            (505.0, 604.0, 401.0, 500.0),
        ],
    )
}

// ---------------------------------------------------------------------------
// Sensor trait
// ---------------------------------------------------------------------------

impl Sensor for Sds011 {
    fn init(&mut self) -> Result<()> {
        // Flush anything in the serial buffer
        self.buf.clear();
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let frame = self.read_frame()?;

        let pm2_5 = (frame[3] as f64 * 256.0 + frame[2] as f64) / 10.0;
        let pm10 = (frame[5] as f64 * 256.0 + frame[4] as f64) / 10.0;

        let mut fields = HashMap::new();
        fields.insert("pm2_5".into(), pm2_5);
        fields.insert("pm10".into(), pm10);
        fields.insert("aqi_pm2_5".into(), aqi_pm2_5(pm2_5));
        fields.insert("aqi_pm10".into(), aqi_pm10(pm10));

        Ok(SensorData {
            timestamp: Utc::now(),
            fields,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
    fn driver_name(&self) -> &str {
        "sds011"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    fn recalibrate(&mut self) -> Result<()> {
        self.buf.clear();
        Ok(())
    }
    fn field_descriptors(&self) -> &[FieldDescriptor] {
        FIELDS
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aqi_pm2_5_good() {
        assert!((aqi_pm2_5(5.0) - 20.83).abs() < 0.1);
    }

    #[test]
    fn test_aqi_pm2_5_moderate() {
        // (49 / 23.3) * 7.9 + 51 ≈ 67.61
        assert!((aqi_pm2_5(20.0) - 67.61).abs() < 0.1);
    }

    #[test]
    fn test_aqi_pm10_good() {
        assert!((aqi_pm10(30.0) - 27.77).abs() < 0.1);
    }

    #[test]
    fn test_aqi_max() {
        assert_eq!(aqi_pm2_5(600.0), 500.0);
    }
}
