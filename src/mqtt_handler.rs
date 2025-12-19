use crate::config::AppConfig;
use paho_mqtt as mqtt;
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

    /// Publishes a message to an MQTT topic
    pub fn publish(&self, topic: &str, payload: &str) -> Result<(), String> {
        let qos = self.config.mqtt.qos;
        let msg = mqtt::Message::new(topic, payload, qos);
        self.client
            .publish(msg)
            .map_err(|e| format!("Failed to publish to {}: {}", topic, e))
    }

    /// Check if the MQTT client is connected
    pub fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    /// Attempt to reconnect to the MQTT broker
    pub fn reconnect(&self) -> Result<(), String> {
        if self.is_connected() {
            return Ok(());
        }

        log::info!("Attempting to reconnect to MQTT broker...");
        
        let conn_opts = mqtt::ConnectOptionsBuilder::new()
            .keep_alive_interval(Duration::from_secs(self.config.mqtt.keep_alive_secs))
            .clean_session(self.config.mqtt.clean_session)
            .finalize();

        self.client
            .reconnect(conn_opts)
            .map_err(|e| format!("Failed to reconnect to MQTT broker: {}", e))?;

        log::info!("Reconnected to MQTT broker");
        Ok(())
    }

    /// Disconnect from the MQTT broker
    pub fn disconnect(&self) -> Result<(), String> {
        if self.client.is_connected() {
            log::info!("Disconnecting from MQTT broker");
            self.client
                .disconnect(None)
                .map_err(|e| format!("Failed to disconnect: {}", e))?;
        }
        Ok(())
    }
}

/// Sets up and returns an MQTT client
fn setup_mqtt(config: &Arc<AppConfig>) -> Result<mqtt::Client, String> {
    // Create client options
    let host = format!("mqtt://{}:{}", config.mqtt.host, config.mqtt.port);
    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri(&host)
        .client_id(&config.mqtt.client_id)
        .finalize();

    // Create the client
    let client = mqtt::Client::new(create_opts)
        .map_err(|e| format!("Failed to create MQTT client: {}", e))?;

    // Create connection options
    let mut conn_opts_builder = mqtt::ConnectOptionsBuilder::new();
    conn_opts_builder
        .keep_alive_interval(Duration::from_secs(config.mqtt.keep_alive_secs))
        .clean_session(config.mqtt.clean_session);

    // Add authentication if provided
    if let (Some(username), Some(password)) = (&config.mqtt.username, &config.mqtt.password) {
        conn_opts_builder.user_name(username).password(password);
    }

    let conn_opts = conn_opts_builder.finalize();

    // Connect to the broker
    client
        .connect(conn_opts)
        .map_err(|e| format!("Failed to connect to MQTT broker: {}", e))?;

    log::info!(
        "Connected to MQTT broker at {}:{}",
        config.mqtt.host, config.mqtt.port
    );
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MqttConfig, ServiceConfig, LoggingConfig};

    #[test]
    fn test_mqtt_publish() {
        // Note: This test requires a running MQTT broker
        // Skip in CI or when broker is not available
        let config = Arc::new(AppConfig {
            service: ServiceConfig::default(),
            logging: LoggingConfig::default(),
            mqtt: MqttConfig {
                host: "localhost".to_string(),
                port: 1883,
                base_topic: "/test".to_string(),
                client_id: "test-client".to_string(),
                keep_alive_secs: 20,
                clean_session: true,
                qos: 1,
                username: None,
                password: None,
            },
        });

        // Only run if we can connect
        if let Ok(handler) = MqttHandler::new(config) {
            assert!(handler.is_connected());
            let result = handler.publish("/test/topic", "test message");
            assert!(result.is_ok());
        }
    }
}
