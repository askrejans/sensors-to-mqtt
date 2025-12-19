//! Service layer for managing sensor reading and publishing.
//!
//! This module provides the core service logic that can run in either
//! interactive (with UI) or daemon (headless) mode.

use crate::config::AppConfig;
use crate::error::{AppError, Result};
use crate::mqtt_handler::MqttHandler;
use crate::publisher::{MqttPublisher, NoOpPublisher, Publisher};
use crate::sensors::i2c::I2CBus;
use crate::sensors::{Sensor, SensorType};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Run mode for the service
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Interactive mode with TUI
    Interactive,
    /// Background daemon mode
    Daemon,
}

/// Service state for managing sensors and publishing
pub struct SensorService {
    config: Arc<AppConfig>,
    sensor_buses: Vec<I2CBus>,
    publisher: Arc<dyn Publisher>,
    should_stop: Arc<AtomicBool>,
}

impl SensorService {
    /// Create a new sensor service
    pub fn new(config: Arc<AppConfig>, no_mqtt: bool) -> Result<Self> {
        // Load sensor configuration
        let sensor_config_content = std::fs::read_to_string("config.yaml")
            .map_err(|e| AppError::Io(e))?;
        let sensor_config: crate::sensors::SensorConfig = serde_yaml_ng::from_str(&sensor_config_content)
            .map_err(|e| crate::error::ConfigError::ParseError(e.to_string()))?;

        // Initialize sensor buses
        let mut sensor_buses = Vec::new();
        for sensor_type in sensor_config.sensors {
            match sensor_type {
                SensorType::I2C(i2c_config) => {
                    let bus = I2CBus::new(i2c_config)?;
                    sensor_buses.push(bus);
                }
            }
        }

        // Initialize publisher
        let publisher: Arc<dyn Publisher> = if no_mqtt {
            log::info!("MQTT publishing disabled");
            Arc::new(NoOpPublisher)
        } else {
            let mqtt_handler = Arc::new(
                MqttHandler::new(config.clone())
                    .map_err(|e| crate::error::MqttError::ConnectionError(e))?
            );
            log::info!("MQTT publisher initialized");
            Arc::new(MqttPublisher::new(
                mqtt_handler,
                config.mqtt.base_topic.clone(),
            ))
        };

        Ok(Self {
            config,
            sensor_buses,
            publisher,
            should_stop: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Get sensor names for UI
    pub fn get_sensor_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for bus in &self.sensor_buses {
            for device in &bus.devices {
                names.push(device.get_name().to_string());
            }
        }
        names
    }

    /// Get mutable reference to sensor by name
    pub fn get_sensor_mut(&mut self, name: &str) -> Option<&mut Box<dyn Sensor>> {
        for bus in &mut self.sensor_buses {
            for device in &mut bus.devices {
                if device.get_name() == name {
                    return Some(device);
                }
            }
        }
        None
    }

    /// Read data from all enabled sensors
    pub fn read_sensors(&mut self) -> Result<Vec<(String, crate::sensors::SensorData)>> {
        let mut results = Vec::new();

        for bus in &mut self.sensor_buses {
            for device in &mut bus.devices {
                if !device.is_enabled() {
                    continue;
                }

                match device.read() {
                    Ok(data) => {
                        let name = device.get_name().to_string();
                        results.push((name, data));
                    }
                    Err(e) => {
                        log::error!("Failed to read sensor {}: {}", device.get_name(), e);
                    }
                }
            }
        }

        Ok(results)
    }

    /// Publish sensor data
    pub fn publish(&self, sensor_name: &str, data: &crate::sensors::SensorData) -> Result<()> {
        self.publisher.publish(sensor_name, data)?;
        Ok(())
    }

    /// Check if publisher is connected
    pub fn is_publisher_connected(&self) -> bool {
        self.publisher.is_connected()
    }

    /// Attempt to reconnect publisher
    pub fn reconnect_publisher(&self) -> Result<()> {
        self.publisher.reconnect()?;
        Ok(())
    }

    /// Get the stop signal
    pub fn get_stop_signal(&self) -> Arc<AtomicBool> {
        self.should_stop.clone()
    }

    /// Request service to stop
    pub fn request_stop(&self) {
        self.should_stop.store(true, Ordering::SeqCst);
    }

    /// Run the service in daemon mode
    pub fn run_daemon(&mut self) -> Result<()> {
        log::info!("Starting sensor service in daemon mode");
        let update_interval = Duration::from_millis(self.config.service.update_interval_ms);
        let mut last_reconnect_attempt = Instant::now();
        let reconnect_delay = Duration::from_millis(self.config.service.reconnect_delay_ms);

        while !self.should_stop.load(Ordering::SeqCst) {
            let loop_start = Instant::now();

            // Try to reconnect if disconnected
            if !self.is_publisher_connected() && last_reconnect_attempt.elapsed() > reconnect_delay {
                log::warn!("Publisher disconnected, attempting reconnection...");
                if let Err(e) = self.reconnect_publisher() {
                    log::error!("Reconnection failed: {}", e);
                    last_reconnect_attempt = Instant::now();
                } else {
                    log::info!("Reconnection successful");
                }
            }

            // Read and publish sensor data
            match self.read_sensors() {
                Ok(sensor_data) => {
                    for (name, data) in sensor_data {
                        if let Err(e) = self.publish(&name, &data) {
                            log::error!("Failed to publish data for {}: {}", name, e);
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to read sensors: {}", e);
                }
            }

            // Sleep for remaining time
            let elapsed = loop_start.elapsed();
            if elapsed < update_interval {
                thread::sleep(update_interval - elapsed);
            }
        }

        log::info!("Sensor service stopped");
        Ok(())
    }

    /// Reload configuration
    pub fn reload_config(&mut self) -> Result<()> {
        log::info!("Reloading configuration...");
        // In a full implementation, this would reload the config file
        // and reinitialize sensors and MQTT connection
        log::warn!("Configuration reload not fully implemented yet");
        Ok(())
    }
}

/// Setup signal handlers for graceful shutdown
pub fn setup_signal_handler(stop_signal: Arc<AtomicBool>) -> Result<()> {
    ctrlc::set_handler(move || {
        log::info!("Received shutdown signal");
        stop_signal.store(true, Ordering::SeqCst);
    })
    .map_err(|e| crate::error::ServiceError::SignalError(e.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_mode() {
        assert_eq!(RunMode::Interactive, RunMode::Interactive);
        assert_ne!(RunMode::Interactive, RunMode::Daemon);
    }
}
