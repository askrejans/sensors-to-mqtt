use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub mqtt_base_topic: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            mqtt_host: String::from("localhost"),
            mqtt_port: 1883,
            mqtt_base_topic: String::from("/GOLF86/SENSORS"),
        }
    }
}
