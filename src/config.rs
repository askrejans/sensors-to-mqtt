#[derive(Clone, Debug, Default)]
pub struct AppConfig {
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub mqtt_base_topic: String,
}
