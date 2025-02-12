use anyhow::Result;
use serde::Deserialize;

pub mod i2c;
use i2c::mpu6500::MPU6500;

/// Configuration for sensors and MQTT settings.
///
/// # Fields
///
/// * `sensors` - A vector containing the types of sensors to be used.
/// * `mqtt` - Configuration settings for MQTT.
#[derive(Debug, Deserialize)]
pub struct SensorConfig {
    pub sensors: Vec<SensorType>,
    pub mqtt: MqttConfig,
}

/// Configuration for connecting to an MQTT broker.
///
/// # Fields
///
/// * `host` - The hostname or IP address of the MQTT broker.
/// * `port` - The port number on which the MQTT broker is listening.
/// * `base_topic` - The base topic to be used for MQTT messages.
#[derive(Debug, Deserialize, Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub base_topic: String,
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
/// * `display_data` - Displays the sensor data.
pub trait Sensor {
    fn init(&mut self) -> Result<()>;
    fn read(&mut self) -> Result<SensorData>;
    fn get_info(&self) -> Result<String>;
    fn display_data(&self, data: &SensorData) -> Result<(u16, Option<String>)>;

    fn as_mpu6500(&self) -> Option<&MPU6500> {
        None
    }
}

/// Struct representing sensor data.
///
/// # Fields
///
/// * `timestamp` - The timestamp of the data.
/// * `device_name` - The name of the device.
/// * `sample_rate` - The sample rate of the data.
/// * `values` - A vector of key-value pairs representing the sensor values.
#[derive(Debug, Clone)]
pub struct SensorData {
    pub timestamp: i64,
    pub device_name: String,
    pub sample_rate: u16,
    pub values: Vec<(String, f64)>,
}
