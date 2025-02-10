use anyhow::Result;
use serde::Deserialize;

pub mod i2c;

#[derive(Debug, Deserialize)]
pub struct SensorConfig {
    pub sensors: Vec<SensorType>,
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
}

#[derive(Debug, Clone)]
pub struct SensorData {
    pub timestamp: i64,
    pub values: Vec<(String, f64)>,
}