//! Sensor registry — instantiates the correct driver from a SensorConfig.
//!
//! Supported drivers (value of `driver` field in TOML):
//!
//! | driver        | connection | crate feature |
//! |---------------|------------|--------------|
//! | `synthetic`   | any        | always        |
//! | `mpu6500`     | i2c        | linux         |
//! | `bmp280`      | i2c        | linux         |
//! | `bme280`      | i2c        | linux         |
//! | `sht31`       | i2c        | linux         |
//! | `bh1750`      | i2c        | linux         |
//! | `ina219`      | i2c        | linux         |
//! | `ads1115`     | i2c        | linux         |
//! | `gpio_button` | gpio       | linux         |
//! | `sds011`      | serial     | linux         |

use super::Sensor;
use super::synthetic::SyntheticSensor;
use crate::config::SensorConfig;
use anyhow::{Result, bail};

/// Create a boxed [`Sensor`] from configuration.
pub fn create_sensor(config: &SensorConfig) -> Result<Box<dyn Sensor>> {
    match config.driver.as_str() {
        "synthetic" => Ok(Box::new(SyntheticSensor::from_config(config)?)),

        #[cfg(target_os = "linux")]
        "mpu6500" => Ok(Box::new(super::i2c::mpu6500::MPU6500::from_config(config)?)),

        #[cfg(target_os = "linux")]
        "bmp280" => Ok(Box::new(super::i2c::bmp280::Bmp280::from_config(config)?)),

        #[cfg(target_os = "linux")]
        "bme280" => Ok(Box::new(super::i2c::bme280::Bme280::from_config(config)?)),

        #[cfg(target_os = "linux")]
        "sht31" => Ok(Box::new(super::i2c::sht31::Sht31::from_config(config)?)),

        #[cfg(target_os = "linux")]
        "bh1750" => Ok(Box::new(super::i2c::bh1750::Bh1750::from_config(config)?)),

        #[cfg(target_os = "linux")]
        "ina219" => Ok(Box::new(super::i2c::ina219::Ina219::from_config(config)?)),

        #[cfg(target_os = "linux")]
        "ads1115" => Ok(Box::new(super::i2c::ads1115::Ads1115::from_config(config)?)),

        #[cfg(target_os = "linux")]
        "gpio_button" => Ok(Box::new(super::gpio::button::GpioButton::from_config(
            config,
        )?)),

        #[cfg(target_os = "linux")]
        "sds011" => Ok(Box::new(super::serial::sds011::Sds011::from_config(
            config,
        )?)),

        other => bail!(
            "Unknown sensor driver: '{}'. Available: synthetic, mpu6500, \
            bmp280, bme280, sht31, bh1750, ina219, ads1115, gpio_button, sds011",
            other
        ),
    }
}
