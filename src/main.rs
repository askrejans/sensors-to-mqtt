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

use crossterm::{
    cursor,
    style::{Color, Print, SetForegroundColor},
    terminal::{self, Clear, ClearType},
    ExecutableCommand, QueueableCommand,
};

use anyhow::Result;

mod config;
mod mqtt_handler;
mod sensors;

use config::AppConfig;
use mqtt_handler::MqttHandler;
use sensors::{SensorConfig, SensorType};

struct ScreenWriter {
    stdout: io::Stdout,
}

impl ScreenWriter {
    fn new() -> Self {
        let mut stdout = io::stdout();
        // Enter alternate screen buffer and hide cursor
        stdout.execute(terminal::EnterAlternateScreen).unwrap();
        stdout.execute(cursor::Hide).unwrap();
        terminal::enable_raw_mode().unwrap();

        Self { stdout }
    }

    fn clear(&mut self) -> io::Result<()> {
        self.stdout
            .queue(Clear(ClearType::All))?
            .queue(cursor::MoveTo(0, 0))?;
        Ok(())
    }

    fn write_line(&mut self, text: &str, color: Option<Color>) -> io::Result<()> {
        if let Some(color) = color {
            self.stdout.queue(SetForegroundColor(color))?;
        }
        self.stdout.queue(Print(text))?.queue(Print("\r\n"))?;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stdout.flush()
    }
}

impl Drop for ScreenWriter {
    fn drop(&mut self) {
        // Restore terminal state
        let _ = terminal::disable_raw_mode();
        let _ = self.stdout.execute(terminal::LeaveAlternateScreen);
        let _ = self.stdout.execute(cursor::Show);
    }
}

fn display_startup_info(
    screen: &mut ScreenWriter,
    sensor_buses: &Vec<sensors::i2c::I2CBus>,
) -> Result<()> {
    screen.write_line("Sensors-to-MQTT System", Some(Color::Green))?;
    screen.write_line("=====================", Some(Color::Green))?;
    screen.write_line("", None)?;
    screen.write_line("🔍 Active Sensors:", Some(Color::Blue))?;

    for (bus_idx, bus) in sensor_buses.iter().enumerate() {
        screen.write_line(&format!("Bus #{}", bus_idx + 1), Some(Color::Yellow))?;
        screen.write_line("---------------", Some(Color::Yellow))?;

        for device in &bus.devices {
            if let Ok(info) = device.get_info() {
                screen.write_line(&format!("✓ {}", info), Some(Color::White))?;
            }
        }
        screen.write_line("", None)?;
    }

    screen.flush()?;
    Ok(())
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

    // Initialize screen writer
    let mut screen = ScreenWriter::new();

    // Initial display
    display_startup_info(&mut screen, &sensor_buses)?;
    thread::sleep(Duration::from_secs(3));

    loop {
        screen.clear()?;
        display_startup_info(&mut screen, &sensor_buses)?;

        // Display and publish sensor readings
        for bus in sensor_buses.iter_mut() {
            for device in &mut bus.devices {
                match device.read() {
                    Ok(data) => {
                        // Get display data from sensor
                        if let Ok((_, Some(display_text))) = device.display_data(&data) {
                            screen.write_line(&display_text, Some(Color::Cyan))?;
                        }

                        // Publish to MQTT
                        if let Some(mpu6500) = device.as_mpu6500() {
                            if let Err(e) = mpu6500.publish_mqtt(&mqtt_handler, &data) {
                                screen.write_line(
                                    &format!("MQTT publish error for MPU6500: {}", e),
                                    Some(Color::Red),
                                )?;
                            }
                        }
                    }
                    Err(e) => {
                        screen.write_line(
                            &format!("Error reading sensor: {}", e),
                            Some(Color::Red),
                        )?;
                    }
                }
            }
        }

        screen.flush()?;
        thread::sleep(Duration::from_millis(10));
    }
}
