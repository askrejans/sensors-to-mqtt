//! I2C sensor drivers.

#[cfg(target_os = "linux")]
pub mod ads1115;
#[cfg(target_os = "linux")]
pub mod bh1750;
#[cfg(target_os = "linux")]
pub mod bme280;
#[cfg(target_os = "linux")]
pub mod bmp280;
#[cfg(target_os = "linux")]
pub mod ina219;
#[cfg(target_os = "linux")]
pub mod mpu6500;
#[cfg(target_os = "linux")]
pub mod sht31;
