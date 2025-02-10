use anyhow::Result;
use serde::Deserialize;

pub mod mpu6500;

#[derive(Debug, Deserialize)]
pub struct I2CConfig {
    pub bus: String,
    pub devices: Vec<I2CDevice>,
}

#[derive(Debug, Deserialize)]
pub struct I2CDevice {
    pub name: String,
    pub address: u16,
    pub driver: String,
    pub settings: serde_yaml_ng::Value,
}

pub struct I2CBus {
    pub devices: Vec<Box<dyn super::Sensor>>,
}

impl I2CBus {
    pub fn new(config: I2CConfig) -> Result<Self> {
        let mut devices = Vec::new();

        for device in config.devices {
            match device.driver.as_str() {
                "mpu6500" => {
                    let sensor = mpu6500::MPU6500::new(&config.bus, device)?;
                    devices.push(Box::new(sensor) as Box<dyn super::Sensor>);
                }
                _ => println!("Unsupported I2C device: {}", device.driver),
            }
        }

        Ok(Self { devices })
    }
}
