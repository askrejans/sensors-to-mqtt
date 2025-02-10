use anyhow::Result;
use serde::Deserialize;

pub mod i2c;

#[derive(Debug, Deserialize)]
pub struct SensorConfig {
    pub sensors: Vec<SensorType>,
    pub mqtt: MqttConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub base_topic: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum SensorType {
    #[serde(rename = "i2c")]
    I2C(i2c::I2CConfig),
}

pub trait Sensor {
    fn init(&mut self) -> Result<()>;
    fn read(&mut self) -> Result<SensorData>;
    fn get_info(&self) -> Result<String>;
}

#[derive(Debug, Clone)]
pub struct SensorData {
    pub timestamp: i64,
    pub values: Vec<(String, f64)>,
}
