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
    cursor, event,
    style::{Color, Print, SetForegroundColor},
    terminal::self,
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
    width: u16,
    height: u16,
}

impl ScreenWriter {
    fn new() -> Self {
        let mut stdout = io::stdout();
        stdout.execute(terminal::EnterAlternateScreen).unwrap();
        stdout.execute(cursor::Hide).unwrap();
        terminal::enable_raw_mode().unwrap();

        let (width, height) = terminal::size().unwrap_or((80, 24));

        Self {
            stdout,
            width,
            height,
        }
    }

    fn write_at(&mut self, x: u16, y: u16, text: &str, color: Option<Color>) -> io::Result<()> {
        self.stdout.queue(cursor::MoveTo(x, y))?;
        if let Some(color) = color {
            self.stdout.queue(SetForegroundColor(color))?;
        }
        self.stdout.queue(Print(text))?;
        Ok(())
    }

    fn draw_box(&mut self, x: u16, y: u16, width: u16, height: u16) -> io::Result<()> {
        // Draw top border
        self.write_at(x, y, "┌", None)?;
        self.write_at(x + width - 1, y, "┐", None)?;

        // Draw sides
        for dy in 1..height - 1 {
            self.write_at(x, y + dy, "│", None)?;
            self.write_at(x + width - 1, y + dy, "│", None)?;
        }

        // Draw bottom border
        self.write_at(x, y + height - 1, "└", None)?;
        self.write_at(x + width - 1, y + height - 1, "┘", None)?;

        // Draw horizontal lines
        for dx in 1..width - 1 {
            self.write_at(x + dx, y, "─", None)?;
            self.write_at(x + dx, y + height - 1, "─", None)?;
        }

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
    // Header
    screen.write_at(2, 1, "Sensors-to-MQTT System", Some(Color::Green))?;
    screen.write_at(
        screen.width - 20,
        1,
        "Press 'q' to exit",
        Some(Color::Yellow),
    )?;
    screen.draw_box(0, 0, screen.width, screen.height)?;

    // Sensor panel (left side)
    let panel_width = screen.width / 2;
    screen.draw_box(1, 3, panel_width - 2, 10)?;
    screen.write_at(3, 4, "🔍 Active Sensors", Some(Color::Blue))?;

    let mut y = 5;
    for (bus_idx, bus) in sensor_buses.iter().enumerate() {
        screen.write_at(3, y, &format!("Bus #{}", bus_idx + 1), Some(Color::Yellow))?;
        y += 1;

        for device in &bus.devices {
            if let Ok(info) = device.get_info() {
                screen.write_at(5, y, &format!("✓ {}", info), Some(Color::White))?;
                y += 1;
            }
        }
        y += 1;
    }

    // Data panel (right side)
    screen.draw_box(panel_width + 1, 3, panel_width - 2, screen.height - 4)?;
    screen.write_at(panel_width + 3, 4, "📊 Sensor Data", Some(Color::Blue))?;

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
    let panel_width = screen.width / 2;

    loop {
        // Check for 'q' key press
        if event::poll(Duration::from_millis(10))? {
            if let event::Event::Key(key_event) = event::read()? {
                if key_event.code == event::KeyCode::Char('q') {
                    break;
                }
            }
        }

        // Clear only the data panel area
        for y in 6..screen.height - 1 {
            screen.write_at(
                panel_width + 3,
                y,
                &" ".repeat((panel_width - 5) as usize),
                None,
            )?;
        }
        let mut data_y = 6;

        // Display and publish sensor readings
        for bus in sensor_buses.iter_mut() {
            for device in &mut bus.devices {
                match device.read() {
                    Ok(data) => {
                        // Get display data from sensor
                        if let Ok((_lines, Some(display_text))) = device.display_data(&data) {
                            // Split the display text into lines and write each line
                            for line in display_text.lines() {
                                if data_y < screen.height - 1 {
                                    screen.write_at(
                                        panel_width + 3,
                                        data_y,
                                        line,
                                        Some(Color::Cyan),
                                    )?;
                                    data_y += 1;
                                }
                            }
                        }

                        // Publish to MQTT
                        if let Some(mpu6500) = device.as_mpu6500() {
                            if let Err(e) = mpu6500.publish_mqtt(&mqtt_handler, &data) {
                                screen.write_at(
                                    panel_width + 3,
                                    screen.height - 2,
                                    &format!("MQTT error: {}", e),
                                    Some(Color::Red),
                                )?;
                            }
                        }
                    }
                    Err(e) => {
                        screen.write_at(
                            panel_width + 3,
                            screen.height - 2,
                            &format!("Sensor error: {}", e),
                            Some(Color::Red),
                        )?;
                    }
                }
            }
        }

        screen.flush()?;
        thread::sleep(Duration::from_millis(10));
    }

    Ok(())
}
