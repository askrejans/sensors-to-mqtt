use crate::config::AppConfig;
use crate::sensors::SensorData;
use paho_mqtt as mqtt;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

pub struct MqttHandler {
    client: mqtt::Client,
    config: Arc<AppConfig>,
}

impl MqttHandler {
    /// Creates a new MQTT handler instance
    pub fn new(config: Arc<AppConfig>) -> Result<Self, String> {
        let client = setup_mqtt(&config)?;
        Ok(Self { client, config })
    }

    /// Publishes all sensor data to MQTT topics
    pub fn publish_sensor_data(
        &self,
        data: &SensorData,
        angles: Option<(f64, f64)>,
    ) -> Result<(), String> {
        // Base topic
        let base_topic = format!("{}", self.config.mqtt_base_topic);

        // 1. Publish sensor info
        let info_json = json!({
            "timestamp": data.timestamp,
            "device": "MPU6500",
            "sample_rate": 100
        });
        self.publish_json(&format!("{}/SENSOR_INFO", base_topic), &info_json)?;

        // 2. Publish raw sensor data
        let mut raw_values = serde_json::Map::new();
        for (key, value) in &data.values {
            raw_values.insert(key.clone(), json!(value));
        }
        raw_values.insert("timestamp".to_string(), json!(data.timestamp));
        let raw_json = Value::Object(raw_values);
        self.publish_json(&format!("{}/RAW", base_topic), &raw_json)?;

        // 3. Publish derived data
        let mut derived_data = serde_json::Map::new();

        // Add angles if available
        if let Some((lean, bank)) = angles {
            derived_data.insert("lean_angle".to_string(), json!(lean));
            derived_data.insert("bank_angle".to_string(), json!(bank));
        }

        // Calculate G forces
        for (key, value) in &data.values {
            match key.as_str() {
                "accel_x" => derived_data.insert("lateral_g".to_string(), json!(value)),
                "accel_y" => derived_data.insert("forward_g".to_string(), json!(value)),
                "accel_z" => derived_data.insert("vertical_g".to_string(), json!(value)),
                "gyro_x" => derived_data.insert("roll_rate".to_string(), json!(value)),
                "gyro_y" => derived_data.insert("pitch_rate".to_string(), json!(value)),
                "gyro_z" => derived_data.insert("yaw_rate".to_string(), json!(value)),
                _ => None,
            };
        }

        derived_data.insert("timestamp".to_string(), json!(data.timestamp));
        let derived_json = Value::Object(derived_data);
        self.publish_json(&format!("{}/DERIVED", base_topic), &derived_json)?;

        Ok(())
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
