//! I2C bus abstraction supporting local hardware and TCP bridges.
//!
//! # TCP wire protocol — length-prefixed framing (io-to-net)
//!
//! The bridge wraps every I2C read response in a 2-byte big-endian length
//! header so the client can consume exactly one complete response per
//! receive call without relying on TCP segment boundaries.
//!
//! The I2C device address is configured **on the bridge side** (e.g.
//! `i2c:///dev/i2c-1@0x68`), so the TCP client never sends an address byte.
//!
//! ## Write
//! ```text
//! TX: [register][value...]            (forwarded to the I2C device as-is)
//! ```
//!
//! ## Write-Read (most common for sensor registers)
//! ```text
//! TX: [register_addr]                 (sets the sensor's internal register pointer)
//! RX: [len_hi][len_lo][data...]       (2-byte BE length + payload)
//! ```
//!
//! ## Read
//! ```text
//! RX: [len_hi][len_lo][data...]       (2-byte BE length + payload)
//! ```

use anyhow::{bail, Context, Result};
use std::io::{Read, Write as IoWrite};
use std::net::TcpStream;
use std::time::Duration;

use crate::config::{ConnectionConfig, SensorConfig};
use crate::transport::framing::tcp_read_framed;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Minimal I2C bus abstraction, compatible with both local hardware and TCP.
///
/// For the TCP implementation (`TcpI2c`), the `addr` parameter is **ignored**
/// because the I2C device address is fixed in the io-to-net bridge config.
pub trait I2cBus: Send {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<()>;
    fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<()>;
    fn write_read(&mut self, addr: u8, write: &[u8], read: &mut [u8]) -> Result<()>;
}

// ---------------------------------------------------------------------------
// TCP transport — transparent bridge client
// ---------------------------------------------------------------------------

pub struct TcpI2c {
    stream: TcpStream,
    framing: bool,
}

impl TcpI2c {
    pub fn connect(host: &str, port: u16, framing: bool) -> Result<Self> {
        let addr = format!("{}:{}", host, port);
        let stream = TcpStream::connect(&addr)
            .with_context(|| format!("Failed to connect to io-to-net bridge at {}", addr))?;
        stream
            .set_read_timeout(Some(Duration::from_millis(2000)))
            .context("Failed to set TCP read timeout")?;
        stream
            .set_write_timeout(Some(Duration::from_millis(2000)))
            .context("Failed to set TCP write timeout")?;
        Ok(Self { stream, framing })
    }
}

impl I2cBus for TcpI2c {
    /// Write bytes to the I2C device via the transparent bridge.
    /// `addr` is ignored — the bridge has the device address configured.
    fn write(&mut self, _addr: u8, data: &[u8]) -> Result<()> {
        self.stream
            .write_all(data)
            .context("TCP bridge: write failed")
    }

    /// Read bytes from the I2C device via the bridge.
    /// `addr` is ignored — the bridge has the device address configured.
    fn read(&mut self, _addr: u8, buf: &mut [u8]) -> Result<()> {
        if self.framing {
            tcp_read_framed(&mut self.stream, buf).context("TCP bridge: read failed")
        } else {
            self.stream.read_exact(buf).context("TCP bridge: read failed")
        }
    }

    /// Write register address, then read the sensor response.
    /// `addr` is ignored — the bridge has the device address configured.
    fn write_read(&mut self, _addr: u8, write: &[u8], read: &mut [u8]) -> Result<()> {
        self.stream
            .write_all(write)
            .context("TCP bridge: write_read (write) failed")?;
        if self.framing {
            tcp_read_framed(&mut self.stream, read)
                .context("TCP bridge: write_read (read) failed")
        } else {
            self.stream
                .read_exact(read)
                .context("TCP bridge: write_read (read) failed")
        }
    }
}

// ---------------------------------------------------------------------------
// Local I2C (Linux only)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub struct LocalI2c(linux_embedded_hal::I2cdev);

#[cfg(target_os = "linux")]
impl I2cBus for LocalI2c {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<()> {
        use embedded_hal::i2c::I2c;
        self.0
            .write(addr, data)
            .map_err(|e| anyhow::anyhow!("{:?}", e))
    }

    fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<()> {
        use embedded_hal::i2c::I2c;
        self.0
            .read(addr, buf)
            .map_err(|e| anyhow::anyhow!("{:?}", e))
    }

    fn write_read(&mut self, addr: u8, write: &[u8], read: &mut [u8]) -> Result<()> {
        use embedded_hal::i2c::I2c;
        self.0
            .write_read(addr, write, read)
            .map_err(|e| anyhow::anyhow!("{:?}", e))
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Open an I2C bus from a sensor config. Returns `(bus, i2c_address)`.
///
/// - `type = "i2c"`: uses the local I2C device (Linux only).
/// - `type = "tcp"`: connects to an io-to-net bridge (all platforms).
///   The I2C device address is taken from the optional `address` field in the
///   TCP connection config, or falls back to `default_address`.
pub fn open_i2c(cfg: &SensorConfig, default_address: u8) -> Result<(Box<dyn I2cBus>, u8)> {
    match &cfg.connection {
        ConnectionConfig::I2c(c) => {
            #[cfg(target_os = "linux")]
            {
                let dev = linux_embedded_hal::I2cdev::new(&c.device)
                    .with_context(|| format!("Failed to open I2C device {}", c.device))?;
                Ok((Box::new(LocalI2c(dev)), c.address as u8))
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = c;
                bail!(
                    "Local I2C (type = \"i2c\") is only supported on Linux. \
                     Use type = \"tcp\" to connect via an io-to-net bridge."
                )
            }
        }
        ConnectionConfig::Tcp(c) => {
            let addr = c.address.map(|a| a as u8).unwrap_or(default_address);
            let bus = TcpI2c::connect(&c.host, c.port, c.framing)?;
            Ok((Box::new(bus), addr))
        }
        other => bail!(
            "Expected 'i2c' or 'tcp' connection for I2C sensor, got '{}'",
            match other {
                ConnectionConfig::Serial(_) => "serial",
                ConnectionConfig::Gpio(_) => "gpio",
                _ => "unknown",
            }
        ),
    }
}
