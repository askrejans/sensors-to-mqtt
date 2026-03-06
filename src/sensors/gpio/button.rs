//! GPIO button / switch sensor (Linux only, via sysfs /sys/class/gpio).
//!
//! Each instance monitors a single GPIO pin and reports:
//!
//! | field            | description                                    |
//! |------------------|------------------------------------------------|
//! | `state`          | current logic level after debounce (0 or 1)    |
//! | `press_count`    | edge count since start (rising if active-high) |
//! | `press_duration_ms` | how long the pin has been in its current state |
//!
//! Configuration in `config.toml`:
//! ```toml
//! [[sensors]]
//! name        = "Brake Switch"
//! driver      = "gpio_button"
//! enabled     = true
//!
//! [sensors.connection]
//! type        = "gpio"
//! pin         = 17
//! active_low  = false   # true = LOW means button pressed
//! debounce_ms = 50
//! ```
//!
//! The driver exports the GPIO via sysfs on `init()` and unexports on drop.

#![cfg(target_os = "linux")]

use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::config::{ConnectionConfig, GpioConnectionConfig, SensorConfig};
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
// Driver
// ---------------------------------------------------------------------------

pub struct GpioButton {
    name: String,
    cfg: GpioConnectionConfig,
    enabled: bool,
    value_path: PathBuf,

    last_state: Option<bool>,
    debounce_until: Option<Instant>,
    stable_state: bool,
    press_count: u64,
    state_entered: Instant,
}

impl GpioButton {
    pub fn from_config(cfg: &SensorConfig) -> Result<Self> {
        let gpio_cfg = match &cfg.connection {
            ConnectionConfig::Gpio(g) => g.clone(),
            _ => anyhow::bail!("gpio_button requires a GPIO connection"),
        };
        let value_path = PathBuf::from(format!("/sys/class/gpio/gpio{}/value", gpio_cfg.pin));
        Ok(Self {
            name: cfg.name.clone(),
            cfg: gpio_cfg,
            enabled: cfg.enabled,
            value_path,
            last_state: None,
            debounce_until: None,
            stable_state: false,
            press_count: 0,
            state_entered: Instant::now(),
        })
    }

    /// Export GPIO and configure as input via sysfs.
    fn export_gpio(&self) -> Result<()> {
        let pin_str = self.cfg.pin.to_string();

        // Export if not already exported
        if !self.value_path.exists() {
            fs::write("/sys/class/gpio/export", &pin_str).context("exporting GPIO pin")?;
            // Kernel may need a moment to create the directory
            let deadline = Instant::now() + Duration::from_millis(500);
            while !self.value_path.exists() {
                if Instant::now() > deadline {
                    anyhow::bail!(
                        "GPIO{} sysfs entry did not appear after export",
                        self.cfg.pin
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        // Direction
        let dir_path = format!("/sys/class/gpio/gpio{}/direction", self.cfg.pin);
        fs::write(&dir_path, "in").context("setting GPIO direction to 'in'")?;

        // Active-low polarity
        let al_path = format!("/sys/class/gpio/gpio{}/active_low", self.cfg.pin);
        fs::write(&al_path, if self.cfg.active_low { "1" } else { "0" }).ok(); // not fatal if not supported
        Ok(())
    }

    fn read_raw(&self) -> Result<bool> {
        let s = fs::read_to_string(&self.value_path).context("reading GPIO value")?;
        Ok(s.trim() == "1")
    }
}

impl Drop for GpioButton {
    fn drop(&mut self) {
        // Best-effort unexport; ignore errors
        let _ = fs::write("/sys/class/gpio/unexport", self.cfg.pin.to_string());
    }
}

impl Sensor for GpioButton {
    fn init(&mut self) -> Result<()> {
        self.export_gpio()?;
        self.stable_state = self.read_raw()?;
        self.last_state = Some(self.stable_state);
        self.state_entered = Instant::now();
        Ok(())
    }

    fn read(&mut self) -> Result<SensorData> {
        let raw = self.read_raw()?;
        let debounce = Duration::from_millis(self.cfg.debounce_ms);
        let now = Instant::now();

        // Debounce state machine
        if Some(raw) != self.last_state {
            // Edge detected — start/restart debounce timer
            self.debounce_until = Some(now + debounce);
            self.last_state = Some(raw);
        }

        if let Some(until) = self.debounce_until {
            if now >= until {
                self.debounce_until = None;
                if raw != self.stable_state {
                    // Stable transition
                    if raw {
                        self.press_count += 1;
                    } // count rising edges
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
