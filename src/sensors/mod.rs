//! Sensor abstraction layer.
//!
//! Any new sensor driver only needs to:
//!   1. Implement the `Sensor` trait
//!   2. Add a match arm in `registry::create_sensor`

pub mod gpio;
#[cfg(target_os = "linux")]
pub mod i2c;
#[cfg(target_os = "linux")]
pub mod serial;
pub mod registry;
pub mod synthetic;

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Field descriptor — tells the TUI how to display a field
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum VizType {
    Value,
    GForce,
    AngularRate,
    Angle,
    Numeric { unit: &'static str },
}

#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    pub key: &'static str,
    pub label: &'static str,
    pub viz: VizType,
    /// Group header shown in the data panel (None = continuation)
    pub group: Option<&'static str>,
}

// ---------------------------------------------------------------------------
// Sensor data
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SensorData {
    pub timestamp: DateTime<Utc>,
    pub fields: HashMap<String, f64>,
}

// ---------------------------------------------------------------------------
// Sensor trait
// ---------------------------------------------------------------------------

/// A hardware sensor.  Implementations are synchronous — Tokio tasks
/// run blocking operations via `spawn_blocking` when needed.
pub trait Sensor: Send + 'static {
    fn init(&mut self) -> Result<()>;
    fn read(&mut self) -> Result<SensorData>;
    fn name(&self) -> &str;
    fn driver_name(&self) -> &str {
        "unknown"
    }
    fn is_enabled(&self) -> bool;
    fn set_enabled(&mut self, enabled: bool);
    fn recalibrate(&mut self) -> Result<()> {
        Ok(())
    }
    /// Ordered field descriptors for TUI rendering.
    fn field_descriptors(&self) -> &[FieldDescriptor];
}
