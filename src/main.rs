/// Main entry point for the Sensors-to-MQTT system.
///
/// This function performs the following steps:
/// 1. Loads the sensor configuration from a YAML file.
/// 2. Initializes the MQTT handler with the loaded configuration.
/// 3. Initializes the sensor buses based on the configuration.
/// 4. Clears the terminal screen and displays initial sensor information.
/// 5. Enters an infinite loop where it periodically reads sensor data, displays it, and publishes it to the MQTT broker.
///
/// # Returns
///
/// * `Result<()>` - Returns `Ok(())` if the program runs successfully, or an error if any step fails.
///
/// # Errors
///
/// This function will return an error if:
/// * The configuration file cannot be read or parsed.
/// * The MQTT handler cannot be initialized.
/// * Any sensor bus cannot be initialized.
/// * Sensor data cannot be read or published to the MQTT broker.
use std::{
    fs,
    io::{self, Write},
    sync::Arc,
    thread,
    time::Duration,
};

use anyhow::Result;

mod config;
mod mqtt_handler;
mod sensors;

use config::AppConfig;
use mqtt_handler::MqttHandler;
use sensors::{SensorConfig, SensorType};

fn move_cursor_up(lines: u16) {
    print!("\x1B[{}A", lines);
}

fn clear_screen_from_cursor() {
    print!("\x1B[J");
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
    // Load configs
    let sensor_config: SensorConfig = serde_yaml_ng::from_str(&fs::read_to_string("config.yaml")?)?;

    let app_config = Arc::new(AppConfig {
        mqtt_host: sensor_config.mqtt.host.clone(),
        mqtt_port: sensor_config.mqtt.port,
        mqtt_base_topic: sensor_config.mqtt.base_topic.clone(),
    });

    // Initialize MQTT handler
    let mqtt_handler = MqttHandler::new(app_config.clone())
        .map_err(|e| anyhow::anyhow!("Failed to create MQTT handler: {}", e))?;

    // Initialize sensor buses
    let mut sensor_buses = Vec::new();
    for sensor_type in sensor_config.sensors {
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

        let info_lines = display_startup_info(&sensor_buses);
        total_lines = 3 + info_lines;

        // Display and publish sensor readings
        for bus in sensor_buses.iter_mut() {
            for device in &mut bus.devices {
                match device.read() {
                    Ok(data) => {
                        // Get display data from sensor
                        if let Ok((lines, Some(display_text))) = device.display_data(&data) {
                            print!("{}", display_text);
                            total_lines += lines;
                        }

                        // Publish to MQTT
                        if let Some(mpu6500) = device.as_mpu6500() {
                            if let Err(e) = mpu6500.publish_mqtt(&mqtt_handler, &data) {
                                eprintln!("MQTT publish error for MPU6500: {}", e);
                                total_lines += 1;
                            }
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
