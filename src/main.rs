use std::{fs, thread, time::Duration};
use anyhow::Result;

mod sensors;
use sensors::{SensorConfig, SensorType};

fn main() -> Result<()> {
    // Read config file
    let config: SensorConfig = serde_yaml_ng::from_str(
        &fs::read_to_string("config.yaml")?
    )?;
    
    // Initialize sensor buses
    let mut sensor_buses = Vec::new();
    
    for sensor_type in config.sensors {
        match sensor_type {
            SensorType::I2C(config) => {
                let bus = sensors::i2c::I2CBus::new(config)?;
                sensor_buses.push(bus);
            }
        }
    }
    
    // Main loop
    loop {
        for bus in &mut sensor_buses {
            for device in &mut bus.devices {
                match device.read() {
                    Ok(data) => {
                        // Process and display sensor data
                        println!("\x1B[2J\x1B[1;1H"); // Clear screen
                        println!("Sensor readings:");
                        println!("Timestamp: {}", data.timestamp);
                        for (key, value) in data.values {
                            println!("  {}: {:.3}", key, value);
                        }
                    }
                    Err(e) => eprintln!("Error reading sensor: {}", e),
                }
            }
        }
        
        thread::sleep(Duration::from_millis(10));
    }
}