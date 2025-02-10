use std::{fs, thread, time::Duration};
use std::io::{self, Write};
use anyhow::Result;

mod sensors;
use sensors::{SensorConfig, SensorType};

// Terminal control functions
fn move_cursor_up(lines: u16) {
    print!("\x1B[{}A", lines);
}

fn clear_line() {
    print!("\x1B[2K\r"); // Add \r to return cursor to start of line
}

fn clear_screen_from_cursor() {
    print!("\x1B[J");
}

fn clear_lines(count: u16) {
    for _ in 0..count {
        clear_line();
        println!();
    }
    move_cursor_up(count);
}

fn display_startup_info(sensor_buses: &Vec<sensors::i2c::I2CBus>) -> u16 {
    let mut lines = 0;
    println!("🔍 Active Sensors:");
    lines += 1;
    
    for (bus_idx, bus) in sensor_buses.iter().enumerate() {
        println!("Bus #{}", bus_idx + 1);
        println!("---------------");
        lines += 2;
        for device in &bus.devices {
            if let Ok(info) = device.get_info() {
                println!("✓ {}", info);
                lines += 1;
            }
        }
        println!();
        lines += 1;
    }
    
    lines
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

    // Initial full screen clear
    print!("\x1B[2J\x1B[1;1H");
    println!("Sensors-to-MQTT System");
    println!("=====================\n");
    
    // Display initial sensor info
    let info_lines = display_startup_info(&sensor_buses);
    println!("\nInitialization complete! Starting sensor readings...");
    io::stdout().flush().unwrap();
    thread::sleep(Duration::from_secs(3));
    
    // Track total lines including header and sensor info
    let mut total_lines = 3 + info_lines;
    
    loop {
        move_cursor_up(total_lines);
        clear_screen_from_cursor();
        
        // Redraw header and sensor info
        println!("Sensors-to-MQTT System");
        println!("=====================\n");
        let info_lines = display_startup_info(&sensor_buses);
        total_lines = 3 + info_lines;
        
        // Display sensor readings
        for (bus_idx, bus) in sensor_buses.iter_mut().enumerate() {
            for device in &mut bus.devices {
                match device.read() {
                    Ok(data) => {
                        if let Some(angles) = calculate_angles(&data.values) {
                            total_lines += display_sensor_data(&data, &angles);
                        } else {
                            total_lines += display_raw_data(&data);
                        }
                    }
                    Err(e) => {
                        eprintln!("Error reading sensor: {}", e);
                        total_lines += 1;
                    }
                }
            }
        }
        
        io::stdout().flush().unwrap();
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

fn display_sensor_data(data: &sensors::SensorData, angles: &(f64, f64)) -> u16 {
    let mut lines = 0;
    
    println!("\n📊 Sensor Data @ {}", 
        chrono::DateTime::from_timestamp_millis(data.timestamp)
            .unwrap()
            .format("%H:%M:%S.%3f"));
    lines += 2;

    println!("\n🎯 G-Forces:");
    lines += 2;
    for (key, value) in &data.values {
        match key.as_str() {
            "accel_x" => { println!("  Lateral: {:.2} G", value); lines += 1; }
            "accel_y" => { println!("  Forward: {:.2} G", value); lines += 1; }
            "accel_z" => { println!("  Vertical: {:.2} G", value); lines += 1; }
            _ => {}
        }
    }

    println!("\n🔄 Turn Rate (°/s):");
    lines += 2;
    for (key, value) in &data.values {
        match key.as_str() {
            "gyro_x" => { println!("  Roll: {:.2}", value); lines += 1; }
            "gyro_y" => { println!("  Pitch: {:.2}", value); lines += 1; }
            "gyro_z" => { println!("  Yaw: {:.2}", value); lines += 1; }
            _ => {}
        }
    }

    println!("\n📐 Angles:");
    println!("  Lean: {:.2}°", angles.0);
    println!("  Bank: {:.2}°", angles.1);
    lines += 4;

    lines
}

fn display_raw_data(data: &sensors::SensorData) -> u16 {
    let mut lines = 0;
    
    println!("\n📊 Raw Data @ {}", 
        chrono::DateTime::from_timestamp_millis(data.timestamp)
            .unwrap()
            .format("%H:%M:%S.%3f"));
    lines += 2;
            
    for (key, value) in &data.values {
        println!("  {}: {:.3}", key, value);
        lines += 1;
    }
    
    lines
}