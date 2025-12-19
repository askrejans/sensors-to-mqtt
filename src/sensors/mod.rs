use anyhow::Result;
use serde::Deserialize;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

pub mod i2c;

/// Configuration for sensors and MQTT settings.
///
/// # Fields
///
/// * `sensors` - A vector containing the types of sensors to be used.
#[derive(Debug, Deserialize)]
pub struct SensorConfig {
    pub sensors: Vec<SensorType>,
}

/// Enum representing different types of sensors.
///
/// # Variants
///
/// * `I2C` - Configuration for I2C sensors.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum SensorType {
    #[serde(rename = "i2c")]
    I2C(i2c::I2CConfig),
}

/// Trait representing a generic sensor.
///
/// # Required Methods
///
/// * `init` - Initializes the sensor.
/// * `read` - Reads data from the sensor.
/// * `get_info` - Retrieves information about the sensor.
/// * `get_name` - Returns the name of the sensor.
/// * `is_enabled` - Check if sensor is enabled.
/// * `set_enabled` - Enable or disable the sensor.
/// * `display_data` - Formats the sensor data for display.
pub trait Sensor: Send {
    fn init(&mut self) -> Result<()>;
    fn read(&mut self) -> Result<SensorData>;
    fn get_info(&self) -> Result<String>;
    fn get_name(&self) -> &str;
    fn is_enabled(&self) -> bool;
    fn set_enabled(&mut self, enabled: bool);
    fn display_data(&self, data: &SensorData) -> Result<(u16, Option<String>)>;
}

/// Struct representing sensor data.
///
/// # Fields
///
/// * `timestamp` - The timestamp of the data.
/// * `data` - A HashMap of key-value pairs representing the sensor values.
#[derive(Debug, Clone)]
pub struct SensorData {
    pub timestamp: DateTime<Utc>,
    pub data: HashMap<String, f64>,
}
