use std::{fs, thread, time::Duration};
use anyhow::Result;

mod sensors;
use sensors::{SensorConfig, SensorType};

fn display_startup_info(sensor_buses: &Vec<sensors::i2c::I2CBus>) {
    println!("\x1B[2J\x1B[1;1H"); // Clear screen
    println!("Sensors-to-MQTT System");
    println!("=====================");
    println!("🔍 Detected Sensors:");
    
    for (bus_idx, bus) in sensor_buses.iter().enumerate() {
        println!("\nBus #{}", bus_idx + 1);
        println!("---------------");
        for device in &bus.devices {
            if let Ok(info) = device.get_info() {
                println!("✓ {}", info);
            }
        }
    }
    
    println!("\nInitialization complete! Starting sensor readings...");
    thread::sleep(Duration::from_secs(3));
}

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

    // Display startup information
    display_startup_info(&sensor_buses);
    
    // Main loop
    loop {
        println!("\x1B[2J\x1B[1;1H"); // Clear screen
        println!("Sensor Readings");
        println!("==============");
        
        for (bus_idx, bus) in sensor_buses.iter_mut().enumerate() {
            println!("\nBus #{}", bus_idx + 1);
            println!("---------------");
            
            for device in &mut bus.devices {
                match device.read() {
                    Ok(data) => {
                        // Calculate angles and display formatted data
                        if let Some(angles) = calculate_angles(&data.values) {
                            display_sensor_data(&data, &angles);
                        } else {
                            display_raw_data(&data);
                        }
                    }
                    Err(e) => eprintln!("Error reading sensor: {}", e),
                }
            }
        }
        
        thread::sleep(Duration::from_millis(10));
    }
}

fn calculate_angles(values: &[(String, f64)]) -> Option<(f64, f64)> {
    let mut accel = [0.0; 3];
    let mut has_accel = false;

    for (key, value) in values {
        match key.as_str() {
            "accel_x" => { accel[0] = *value; has_accel = true; }
            "accel_y" => { accel[1] = *value; }
            "accel_z" => { accel[2] = *value; }
            _ => {}
        }
    }

    if !has_accel {
        return None;
    }

    let ax2 = accel[0] * accel[0];
    let az2 = accel[2] * accel[2];
    let lean_angle = (accel[1] / (ax2 + az2).sqrt()).atan() * 180.0 / std::f64::consts::PI;
    let bank_angle = (accel[0] / accel[2].abs()).atan() * 180.0 / std::f64::consts::PI;

    Some((lean_angle, bank_angle))
}

fn display_sensor_data(data: &sensors::SensorData, angles: &(f64, f64)) {
    println!("\n📊 Sensor Data @ {}", 
        chrono::DateTime::from_timestamp_millis(data.timestamp)
            .unwrap()
            .format("%H:%M:%S.%3f"));

    println!("\n🎯 G-Forces:");
    for (key, value) in &data.values {
        match key.as_str() {
            "accel_x" => println!("  Lateral: {:.2} G", value),
            "accel_y" => println!("  Forward: {:.2} G", value),
            "accel_z" => println!("  Vertical: {:.2} G", value),
            _ => {}
        }
    }

    println!("\n🔄 Turn Rate (°/s):");
    for (key, value) in &data.values {
        match key.as_str() {
            "gyro_x" => println!("  Roll: {:.2}", value),
            "gyro_y" => println!("  Pitch: {:.2}", value),
            "gyro_z" => println!("  Yaw: {:.2}", value),
            _ => {}
        }
    }

    println!("\n📐 Angles:");
    println!("  Lean: {:.2}°", angles.0);
    println!("  Bank: {:.2}°", angles.1);
}

fn display_raw_data(data: &sensors::SensorData) {
    println!("\n📊 Raw Data @ {}", 
        chrono::DateTime::from_timestamp_millis(data.timestamp)
            .unwrap()
            .format("%H:%M:%S.%3f"));
            
    for (key, value) in &data.values {
        println!("  {}: {:.3}", key, value);
    }
}