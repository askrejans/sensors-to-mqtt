use crate::config::AppConfig;
use paho_mqtt as mqtt;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub struct MqttHandler {
    client: mqtt::Client,
    config: Arc<AppConfig>,
}

impl MqttHandler {
    /// Creates a new MQTT handler with the given configuration
    pub fn new(config: Arc<AppConfig>) -> Result<Self, String> {
        let client = setup_mqtt(&config)?;
        Ok(Self { client, config })
    }
    /// Publishes data to an MQTT topic with the configured base topic prefix
    pub fn publish_data(&self, subtopic: &str, data: &Value) -> Result<(), String> {
        let full_topic = format!("{}/{}", self.config.mqtt_base_topic, subtopic);
        self.publish_json(&full_topic, data)
    }

    /// Helper function to publish JSON data to a topic
    fn publish_json(&self, topic: &str, data: &Value) -> Result<(), String> {
        let msg = mqtt::Message::new(topic, data.to_string(), 1);
        self.client
            .publish(msg)
            .map_err(|e| format!("Failed to publish to {}: {}", topic, e))
    }
}

/// Sets up and returns an MQTT client
fn setup_mqtt(config: &Arc<AppConfig>) -> Result<mqtt::Client, String> {
    // Create client options
    let host = format!("mqtt://{}:{}", config.mqtt_host, config.mqtt_port);
    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri(&host)
        .client_id("sensors-to-mqtt")
        .finalize();

    // Create the client
    let client = mqtt::Client::new(create_opts)
        .map_err(|e| format!("Failed to create MQTT client: {}", e))?;

    // Create connection options
    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_secs(20))
        .clean_session(true)
        .finalize();

    // Connect to the broker
    client
        .connect(conn_opts)
        .map_err(|e| format!("Failed to connect to MQTT broker: {}", e))?;

    println!(
        "Connected to MQTT broker at {}:{}",
        config.mqtt_host, config.mqtt_port
    );
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_mqtt_handler() {
        // Create test configuration
        let config = Arc::new(AppConfig {
            mqtt_host: String::from("localhost"),
            mqtt_port: 1883,
            mqtt_base_topic: String::from("test"),
            ..Default::default()
        });

        // Use mutex for test synchronization
        let mutex = Mutex::new(());
        let _guard = mutex.lock().unwrap();

        // Create handler
        let handler = MqttHandler::new(config.clone());
        assert!(handler.is_ok(), "Failed to create MQTT handler");

        // Create test data
        let test_data = SensorData {
            timestamp: chrono::Utc::now().timestamp_millis(),
            values: vec![
                ("accel_x".to_string(), 1.0),
                ("accel_y".to_string(), 2.0),
                ("accel_z".to_string(), 3.0),
            ],
        };

        // Test publishing
        if let Ok(handler) = handler {
            let result = handler.publish_sensor_data(&test_data);
            assert!(result.is_ok(), "Failed to publish test data");
        }
    }
}
