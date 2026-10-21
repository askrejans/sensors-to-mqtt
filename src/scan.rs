//! `--scan`: find the chips on a local I²C bus and suggest their configuration.
//!
//! Like `i2cdetect -r`, every address answers a one-byte read or not; the
//! responders are then identified by their ID registers where they have one
//! (MPU-6500 `WHO_AM_I`, BMP280/BME280 `id`, INA219 and ADS1115 power-on
//! configuration). Only reads are sent, so a scan never changes a chip's
//! setup.

use crate::transport::I2cBus;

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub address: u8,
    /// Chip name, e.g. `MPU-6500`, or `unknown`.
    pub chip: String,
    /// Driver to configure, when this gateway supports the chip.
    pub driver: Option<&'static str>,
    /// Identified by an ID register (true) or by its address alone (false).
    pub certain: bool,
}

fn reg8(bus: &mut dyn I2cBus, addr: u8, reg: u8) -> Option<u8> {
    let mut b = [0u8; 1];
    bus.write_read(addr, &[reg], &mut b).ok().map(|_| b[0])
}

fn reg16(bus: &mut dyn I2cBus, addr: u8, reg: u8) -> Option<u16> {
    let mut b = [0u8; 2];
    bus.write_read(addr, &[reg], &mut b)
        .ok()
        .map(|_| u16::from_be_bytes(b))
}

fn found(address: u8, chip: &str, driver: Option<&'static str>, certain: bool) -> Found {
    Found {
        address,
        chip: chip.into(),
        driver,
        certain,
    }
}

/// What answers at `addr`, given that it answered.
pub fn identify(bus: &mut dyn I2cBus, addr: u8) -> Found {
    match addr {
        0x68 | 0x69 => match reg8(bus, addr, 0x75) {
            Some(0x70) => found(addr, "MPU-6500", Some("mpu6500"), true),
            Some(0x71) => found(addr, "MPU-9250", Some("mpu6500"), true),
            Some(0x73) => found(addr, "MPU-9255", Some("mpu6500"), true),
            Some(0x68) => found(addr, "MPU-6050", Some("mpu6500"), true),
            _ => found(
                addr,
                "unknown (MPU address, or a DS3231/DS1307 clock)",
                None,
                false,
            ),
        },
        0x76 | 0x77 => match reg8(bus, addr, 0xD0) {
            Some(0x60) => found(addr, "BME280", Some("bme280"), true),
            Some(0x56..=0x58) => found(addr, "BMP280", Some("bmp280"), true),
            Some(0x61) => found(addr, "BME680 (not supported)", None, true),
            _ => found(addr, "unknown", None, false),
        },
        0x48..=0x4B => {
            // ADS1115 config register resets to 0x8583, INA219's to 0x399F
            if reg16(bus, addr, 0x01) == Some(0x8583) {
                found(addr, "ADS1115", Some("ads1115"), true)
            } else if reg16(bus, addr, 0x00) == Some(0x399F) {
                found(addr, "INA219", Some("ina219"), true)
            } else {
                found(addr, "ADS1115 or INA219", Some("ads1115"), false)
            }
        }
        0x44 | 0x45 => {
            if reg16(bus, addr, 0x00) == Some(0x399F) {
                found(addr, "INA219", Some("ina219"), true)
            } else {
                found(addr, "SHT31 (by address)", Some("sht31"), false)
            }
        }
        0x40..=0x4F => {
            if reg16(bus, addr, 0x00) == Some(0x399F) {
                found(addr, "INA219", Some("ina219"), true)
            } else {
                found(addr, "INA219 (by address)", Some("ina219"), false)
            }
        }
        0x23 | 0x5C => found(addr, "BH1750 (by address)", Some("bh1750"), false),
        _ => found(addr, "unknown", None, false),
    }
}

/// Probe every 7-bit address (0x03–0x77) and identify the responders.
pub fn scan_bus(bus: &mut dyn I2cBus) -> Vec<Found> {
    let mut out = Vec::new();
    for addr in 0x03u8..=0x77 {
        let mut b = [0u8; 1];
        if bus.read(addr, &mut b).is_ok() {
            out.push(identify(bus, addr));
        }
    }
    out
}

/// A `[[sensors]]` block for a found chip.
pub fn config_snippet(f: &Found, device: &str) -> Option<String> {
    let driver = f.driver?;
    let name = match driver {
        "mpu6500" => "imu",
        "bme280" | "sht31" => "climate",
        "bmp280" => "pressure",
        "bh1750" => "light",
        "ina219" => "battery",
        "ads1115" => "analog",
        _ => driver,
    };
    Some(format!(
        "[[sensors]]\nname    = \"{name}\"\ndriver  = \"{driver}\"\nenabled = true\n\n[sensors.connection]\ntype    = \"i2c\"\ndevice  = \"{device}\"\naddress = {:#04x}\n",
        f.address
    ))
}

/// Run the scan on a local bus and print the result.
pub fn run(device: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use anyhow::Context;
        let dev = linux_embedded_hal::I2cdev::new(device).with_context(|| {
            format!("opening {device} (is i2c enabled, and are you in the i2c group?)")
        })?;
        let mut bus = crate::transport::i2c_bus::LocalI2c(dev);
        let list = scan_bus(&mut bus);
        if list.is_empty() {
            println!("No devices answer on {device}.");
            return Ok(());
        }
        println!("{device}:");
        for f in &list {
            println!(
                "  {:#04x}  {:<44} {}",
                f.address,
                f.chip,
                f.driver
                    .map(|d| format!("driver = \"{d}\""))
                    .unwrap_or_default()
            );
        }
        let snippets: Vec<String> = list
            .iter()
            .filter_map(|f| config_snippet(f, device))
            .collect();
        if !snippets.is_empty() {
            println!("\nSuggested configuration:\n");
            println!("{}", snippets.join("\n"));
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("--scan needs a local I2C bus ({device}), which is Linux only")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A bus with chips at fixed addresses and register contents.
    struct FakeBus(HashMap<u8, HashMap<u8, Vec<u8>>>);

    impl I2cBus for FakeBus {
        fn write(&mut self, _addr: u8, _data: &[u8]) -> anyhow::Result<()> {
            anyhow::bail!("a scan must not write")
        }
        fn read(&mut self, addr: u8, buf: &mut [u8]) -> anyhow::Result<()> {
            anyhow::ensure!(self.0.contains_key(&addr), "nack");
            buf.fill(0);
            Ok(())
        }
        fn write_read(&mut self, addr: u8, write: &[u8], read: &mut [u8]) -> anyhow::Result<()> {
            let regs = self.0.get(&addr).ok_or_else(|| anyhow::anyhow!("nack"))?;
            let v = regs.get(&write[0]).cloned().unwrap_or_default();
            for (i, b) in read.iter_mut().enumerate() {
                *b = v.get(i).copied().unwrap_or(0xFF);
            }
            Ok(())
        }
    }

    #[test]
    fn finds_and_identifies_chips() {
        let mut bus = FakeBus(HashMap::from([
            (0x68, HashMap::from([(0x75, vec![0x70])])),
            (0x76, HashMap::from([(0xD0, vec![0x60])])),
            (0x77, HashMap::from([(0xD0, vec![0x58])])),
            (0x48, HashMap::from([(0x01, vec![0x85, 0x83])])),
            (0x40, HashMap::from([(0x00, vec![0x39, 0x9F])])),
            (0x44, HashMap::new()),
            (0x50, HashMap::new()),
        ]));
        let list = scan_bus(&mut bus);
        let by: HashMap<u8, &Found> = list.iter().map(|f| (f.address, f)).collect();
        assert_eq!(list.len(), 7);
        assert_eq!(by[&0x68].chip, "MPU-6500");
        assert!(by[&0x68].certain);
        assert_eq!(by[&0x76].driver, Some("bme280"));
        assert_eq!(by[&0x77].driver, Some("bmp280"));
        assert_eq!(by[&0x48].chip, "ADS1115");
        assert_eq!(by[&0x40].chip, "INA219");
        assert!(!by[&0x44].certain);
        assert_eq!(by[&0x50].driver, None);
        let snippet = config_snippet(by[&0x68], "/dev/i2c-1").unwrap();
        assert!(snippet.contains("driver  = \"mpu6500\"") && snippet.contains("address = 0x68"));
        let cfg: crate::config::AppConfig = toml::from_str(&snippet).unwrap();
        assert_eq!(cfg.sensors[0].name, "imu");
        assert!(config_snippet(by[&0x50], "/dev/i2c-1").is_none());
    }
}
