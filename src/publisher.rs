//! Publisher abstraction for sensor data.
//!
//! This module defines the Publisher trait and concrete implementations
//! for publishing sensor data to various destinations (MQTT, logs, etc.).

use crate::error::{MqttError, Result};
use crate::mqtt_handler::MqttHandler;
use crate::sensors::SensorData;
use std::sync::Arc;

/// Trait for publishing sensor data
pub trait Publisher: Send + Sync {
    /// Publish sensor data
    fn publish(&self, sensor_name: &str, data: &SensorData) -> Result<()>;
    
    /// Check if publisher is connected/ready
    fn is_connected(&self) -> bool;
    
    /// Attempt to reconnect if disconnected
    fn reconnect(&self) -> Result<()>;
}

/// MQTT publisher implementation
pub struct MqttPublisher {
    mqtt_handler: Arc<MqttHandler>,
    base_topic: String,
}

impl MqttPublisher {
    /// Create a new MQTT publisher
    pub fn new(mqtt_handler: Arc<MqttHandler>, base_topic: String) -> Self {
        Self {
            mqtt_handler,
            base_topic,
        }
    }

    /// Publish sensor info (sensor identification and configuration)
    fn publish_info(&self, sensor_name: &str, data: &SensorData) -> Result<()> {
        let topic = format!("{}/IMU/{}/INFO", self.base_topic, sensor_name);
        let payload = serde_json::json!({
            "sensor": sensor_name,
            "timestamp": data.timestamp.to_rfc3339(),
        });

        self.mqtt_handler
            .publish(&topic, &payload.to_string())
            .map_err(|e| MqttError::PublishError(e))?;
        Ok(())
    }

    /// Publish filtered sensor data
    fn publish_filtered(&self, sensor_name: &str, data: &SensorData) -> Result<()> {
        let topic = format!("{}/IMU/{}/FILTERED", self.base_topic, sensor_name);
        
        // Create payload from sensor data
        let mut payload = serde_json::Map::new();
        payload.insert("timestamp".to_string(), serde_json::json!(data.timestamp.to_rfc3339()));
        
        for (key, value) in &data.data {
            payload.insert(key.clone(), serde_json::json!(value));
        }

        self.mqtt_handler
            .publish(&topic, &serde_json::to_string(&payload).unwrap())
            .map_err(|e| MqttError::PublishError(e))?;
        Ok(())
    }

    /// Publish derived data (calculations from sensor data)
    fn publish_derived(&self, sensor_name: &str, data: &SensorData) -> Result<()> {
        let topic = format!("{}/IMU/{}/DERIVED", self.base_topic, sensor_name);
        
        // Extract derived values (angles, G-forces, etc.)
        let mut derived = serde_json::Map::new();
        derived.insert("timestamp".to_string(), serde_json::json!(data.timestamp.to_rfc3339()));
        
        // Look for angle and G-force data
        for (key, value) in &data.data {
            if key.contains("angle") || key.contains("g_force") || key.contains("rate") {
                derived.insert(key.clone(), serde_json::json!(value));
            }
        }

        if !derived.is_empty() {
            self.mqtt_handler
                .publish(&topic, &serde_json::to_string(&derived).unwrap())
                .map_err(|e| MqttError::PublishError(e))?;
        }

        Ok(())
    }
}

impl Publisher for MqttPublisher {
    fn publish(&self, sensor_name: &str, data: &SensorData) -> Result<()> {
        self.publish_info(sensor_name, data)?;
        self.publish_filtered(sensor_name, data)?;
        self.publish_derived(sensor_name, data)?;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.mqtt_handler.is_connected()
    }

    fn reconnect(&self) -> Result<()> {
        self.mqtt_handler
            .reconnect()
            .map_err(|e| MqttError::ConnectionError(e))?;
        Ok(())
    }
}

/// No-op publisher for testing or when MQTT is disabled
pub struct NoOpPublisher;

impl Publisher for NoOpPublisher {
    fn publish(&self, _sensor_name: &str, _data: &SensorData) -> Result<()> {
        Ok(())
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn reconnect(&self) -> Result<()> {
        Ok(())
    }
}

/// Logging publisher that logs sensor data instead of publishing
pub struct LoggingPublisher;

impl Publisher for LoggingPublisher {
    fn publish(&self, sensor_name: &str, data: &SensorData) -> Result<()> {
        log::debug!("Sensor {}: {} data points", sensor_name, data.data.len());
        for (key, value) in &data.data {
            log::trace!("  {}: {}", key, value);
        }
        Ok(())
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn reconnect(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    #[test]
    fn test_noop_publisher() {
        let publisher = NoOpPublisher;
        let data = SensorData {
            timestamp: Utc::now(),
            data: HashMap::new(),
        };
        
        assert!(publisher.publish("test", &data).is_ok());
        assert!(publisher.is_connected());
        assert!(publisher.reconnect().is_ok());
    }

    #[test]
    fn test_logging_publisher() {
        let publisher = LoggingPublisher;
        let mut data_map = HashMap::new();
        data_map.insert("test_key".to_string(), 42.0);
        
        let data = SensorData {
            timestamp: Utc::now(),
            data: data_map,
        };
        
        assert!(publisher.publish("test", &data).is_ok());
        assert!(publisher.is_connected());
    }
}
