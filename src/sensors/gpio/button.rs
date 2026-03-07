//! GPIO button / switch sensor.
//!
//! Supports two transports:
//! - **Local sysfs GPIO** (Linux only, via `/sys/class/gpio`)
//! - **TCP bridge** (all platforms, via io-to-net or compatible bridge)
//!
//! Each instance monitors a single GPIO pin and reports:
//!
//! | field            | description                                    |
//! |------------------|------------------------------------------------|
//! | `state`          | current logic level after debounce (0 or 1)    |
//! | `press_count`    | edge count since start (rising if active-high) |
//! | `press_duration_ms` | how long the pin has been in its current state |
//!
//! ## TCP wire protocol
//!
//! Polling mode: client sends `[0x01]`, server responds with `[0x00]` (low)
//! or `[0x01]` (high). The `active_low` and `debounce_ms` fields are applied
//! in software on the client side.

use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::config::{ConnectionConfig, SensorConfig};
use crate::transport::tcp_read_framed;
use crate::sensors::{FieldDescriptor, Sensor, SensorData, VizType};

// ---------------------------------------------------------------------------
// Field descriptors
// ---------------------------------------------------------------------------

static FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        key: "state",
        label: "State",
        viz: VizType::Value,
        group: Some("GPIO"),
    },
    FieldDescriptor {
        key: "press_count",
        label: "Presses",
        viz: VizType::Numeric { unit: "count" },
        group: None,
    },
    FieldDescriptor {
        key: "press_duration_ms",
        label: "Duration",
        viz: VizType::Numeric { unit: "ms" },
        group: None,
    },
];

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

enum Transport {
    #[cfg(target_os = "linux")]
    Sysfs {
        value_path: std::path::PathBuf,
        pin: u32,
    },
    Tcp {
        stream: std::net::TcpStream,
        framing: bool,
    },
}

impl Transport {
    /// Read the raw pin level (before active_low inversion).
    fn read_level(&mut self) -> Result<bool> {
        match self {
            #[cfg(target_os = "linux")]
            Transport::Sysfs { value_path, .. } => {
                let s =
                    std::fs::read_to_string(value_path).context("reading GPIO sysfs value")?;
                Ok(s.trim() == "1")
            }
            Transport::Tcp { stream, framing } => {
                use std::io::{Read, Write};
                stream
                    .write_all(&[0x01])
                    .context("GPIO-over-TCP: poll request failed")?;
                let mut buf = [0u8; 1];
                if *framing {
                    tcp_read_framed(stream, &mut buf)
                        .context("GPIO-over-TCP: poll response failed")?;
                } else {
                    stream
                        .read_exact(&mut buf)
                        .context("GPIO-over-TCP: poll response failed")?;
                }
                Ok(buf[0] != 0)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

pub struct GpioButton {
    name: String,
    transport: Transport,
    active_low: bool,
    debounce_ms: u64,
    enabled: bool,

    last_state: Option<bool>,
    debounce_until: Option<Instant>,
    stable_state: bool,
    press_count: u64,
    state_entered: Instant,
}

impl GpioButton {
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        match &cfg.connection {
            #[cfg(target_os = "linux")]
            ConnectionConfig::Gpio(g) => {
                let value_path = std::path::PathBuf::from(format!(
                    "/sys/class/gpio/gpio{}/value",
                    g.pin
                ));
                Ok(Self {
                    name: cfg.name.clone(),
                    transport: Transport::Sysfs {
                        value_path,
                        pin: g.pin,
                    },
                    active_low: g.active_low,
                    debounce_ms: g.debounce_ms,
                    enabled: cfg.enabled,
                    last_state: None,
                    debounce_until: None,
                    stable_state: false,
                    press_count: 0,
                    state_entered: Instant::now(),
                })
            }

            #[cfg(not(target_os = "linux"))]
            ConnectionConfig::Gpio(_) => {
                anyhow::bail!(
                    "Local GPIO (type = \"gpio\") is only supported on Linux. \
                     Use type = \"tcp\" to connect via an io-to-net bridge."
                )
            }

            ConnectionConfig::Tcp(t) => {
                use std::net::TcpStream;
                let addr = format!("{}:{}", t.host, t.port);
                let stream = TcpStream::connect(&addr)
                    .with_context(|| format!("Failed to connect to GPIO bridge at {}", addr))?;
                stream
                    .set_read_timeout(Some(Duration::from_millis(2000)))
                    .context("Failed to set TCP read timeout")?;
                stream
                    .set_write_timeout(Some(Duration::from_millis(2000)))
                    .context("Failed to set TCP write timeout")?;
                Ok(Self {
                    name: cfg.name.clone(),
                    transport: Transport::Tcp { stream, framing: t.framing },
                    active_low: false,
                    debounce_ms: 50,
                    enabled: cfg.enabled,
                    last_state: None,
                    debounce_until: None,
                    stable_state: false,
                    press_count: 0,
                    state_entered: Instant::now(),
                })
            }

            other => anyhow::bail!(
                "gpio_button requires a 'gpio' or 'tcp' connection, got '{}'",
                match other {
                    ConnectionConfig::I2c(_) => "i2c",
                    ConnectionConfig::Serial(_) => "serial",
                    _ => "unknown",
                }
            ),
        }
    }

    fn read_raw(&mut self) -> Result<bool> {
        let level = self.transport.read_level()?;
        Ok(if self.active_low { !level } else { level })
    }
}

#[cfg(target_os = "linux")]
impl Drop for GpioButton {
    fn drop(&mut self) {
        if let Transport::Sysfs { pin, .. } = &self.transport {
            let _ = std::fs::write("/sys/class/gpio/unexport", pin.to_string());
        }
    }
}

impl Sensor for GpioButton {
    fn init(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        if let Transport::Sysfs { value_path, pin } = &self.transport {
            let pin_str = pin.to_string();
            if !value_path.exists() {
                std::fs::write("/sys/class/gpio/export", &pin_str)
                    .context("exporting GPIO pin")?;
                let deadline = Instant::now() + Duration::from_millis(500);
                while !value_path.exists() {
                    if Instant::now() > deadline {
                        anyhow::bail!(
                            "GPIO{} sysfs entry did not appear after export",
                            pin
                        );
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            let dir_path = format!("/sys/class/gpio/gpio{}/direction", pin);
            std::fs::write(&dir_path, "in").context("setting GPIO direction to 'in'")?;
            let al_path = format!("/sys/class/gpio/gpio{}/active_low", pin);
            std::fs::write(&al_path, if self.active_low { "1" } else { "0" }).ok();
        }

        self.stable_state = self.read_raw()?;
        self.last_state = Some(self.stable_state);
        self.state_entered = Instant::now();
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let raw = self.read_raw()?;
        let debounce = Duration::from_millis(self.debounce_ms);
        let now = Instant::now();

        if Some(raw) != self.last_state {
            self.debounce_until = Some(now + debounce);
            self.last_state = Some(raw);
        }

        if let Some(until) = self.debounce_until {
            if now >= until {
                self.debounce_until = None;
                if raw != self.stable_state {
                    if raw {
                        self.press_count += 1;
                    }
                    self.stable_state = raw;
                    self.state_entered = now;
                }
            }
        }

        let duration_ms = self.state_entered.elapsed().as_millis() as f64;

        let mut fields = HashMap::new();
        fields.insert("state".into(), self.stable_state as u8 as f64);
        fields.insert("press_count".into(), self.press_count as f64);
        fields.insert("press_duration_ms".into(), duration_ms);
        Ok(SensorData {
            timestamp: Utc::now(),
            fields,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
    fn driver_name(&self) -> &str {
        "gpio_button"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }
    fn set_enabled(&mut self, e: bool) {
        self.enabled = e;
    }
    fn recalibrate(&mut self) -> Result<()> {
        self.press_count = 0;
        self.state_entered = Instant::now();
        Ok(())
    }
    fn field_descriptors(&self) -> &[FieldDescriptor] {
        FIELDS
    }
}
