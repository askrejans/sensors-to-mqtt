//! I2C bus abstraction supporting local hardware and TCP bridges.
//!
//! # TCP wire protocol (compatible with io-to-net transparent bridge)
//!
//! Each operation is a request/response exchange:
//!
//! ## Write
//! ```text
//! TX: [0x01][i2c_addr:1][len:1][data:len]
//! RX: [0x00] = ok  |  [0x01] = error
//! ```
//!
//! ## Read
//! ```text
//! TX: [0x02][i2c_addr:1][len:1]
//! RX: [0x00][data:len] = ok  |  [0x01] = error
//! ```
//!
//! ## Write-Read
//! ```text
//! TX: [0x03][i2c_addr:1][write_len:1][read_len:1][write_data:write_len]
//! RX: [0x00][read_data:read_len] = ok  |  [0x01] = error
//! ```

use anyhow::{bail, Context, Result};
use std::io::{Read, Write as IoWrite};
use std::net::TcpStream;
use std::time::Duration;

use crate::config::{ConnectionConfig, SensorConfig};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Minimal I2C bus abstraction, compatible with both local hardware and TCP.
pub trait I2cBus: Send {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<()>;
    fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<()>;
    fn write_read(&mut self, addr: u8, write: &[u8], read: &mut [u8]) -> Result<()>;
}

// ---------------------------------------------------------------------------
// TCP transport
// ---------------------------------------------------------------------------

pub struct TcpI2c {
    stream: TcpStream,
}

impl TcpI2c {
    pub fn connect(host: &str, port: u16) -> Result<Self> {
        let addr = format!("{}:{}", host, port);
        let stream = TcpStream::connect(&addr)
            .with_context(|| format!("Failed to connect to I2C-over-TCP bridge at {}", addr))?;
        stream
            .set_read_timeout(Some(Duration::from_millis(2000)))
            .context("Failed to set TCP read timeout")?;
        stream
            .set_write_timeout(Some(Duration::from_millis(2000)))
            .context("Failed to set TCP write timeout")?;
        Ok(Self { stream })
    }

    fn recv_status(&mut self) -> Result<()> {
        let mut status = [0u8; 1];
        self.stream
            .read_exact(&mut status)
            .context("I2C-over-TCP: no status byte from bridge")?;
        if status[0] != 0x00 {
            bail!("I2C-over-TCP bridge returned error status {:#04x}", status[0]);
        }
        Ok(())
    }
}

impl I2cBus for TcpI2c {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<()> {
        let mut req = vec![0x01, addr, data.len() as u8];
        req.extend_from_slice(data);
        self.stream
            .write_all(&req)
            .context("I2C-over-TCP: write request failed")?;
        self.recv_status()
    }

    fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<()> {
        let req = [0x02, addr, buf.len() as u8];
        self.stream
            .write_all(&req)
            .context("I2C-over-TCP: read request failed")?;
        self.recv_status()?;
        self.stream
            .read_exact(buf)
            .context("I2C-over-TCP: read data failed")?;
        Ok(())
    }

    fn write_read(&mut self, addr: u8, write: &[u8], read: &mut [u8]) -> Result<()> {
        let mut req = vec![0x03, addr, write.len() as u8, read.len() as u8];
        req.extend_from_slice(write);
        self.stream
            .write_all(&req)
            .context("I2C-over-TCP: write_read request failed")?;
        self.recv_status()?;
        self.stream
            .read_exact(read)
            .context("I2C-over-TCP: write_read data failed")?;
        Ok(())
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
            let bus = TcpI2c::connect(&c.host, c.port)?;
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
