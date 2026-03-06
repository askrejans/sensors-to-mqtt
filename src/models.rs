//! Shared application state models.

use crate::sensors::SensorData;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// MQTT status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MqttStatus {
    Disabled,
    Connecting,
    Connected,
    Disconnected,
    Error(String),
}

impl MqttStatus {
    pub fn is_connected(&self) -> bool {
        matches!(self, MqttStatus::Connected)
    }

    pub fn label(&self) -> &str {
        match self {
            MqttStatus::Disabled => "DISABLED",
            MqttStatus::Connecting => "CONNECTING",
            MqttStatus::Connected => "CONNECTED",
            MqttStatus::Disconnected => "DISCONNECTED",
            MqttStatus::Error(_) => "ERROR",
        }
    }
}

// ---------------------------------------------------------------------------
// Per-sensor rolling history (for sparklines / charts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SensorHistory {
    pub capacity: usize,
    pub fields: HashMap<String, VecDeque<f64>>,
    pub max_g_magnitude: f64,
}

impl SensorHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            fields: HashMap::new(),
            max_g_magnitude: 0.0,
        }
    }

    pub fn push(&mut self, data: &SensorData) {
        for (key, &val) in &data.fields {
            let buf = self
                .fields
                .entry(key.clone())
                .or_insert_with(|| VecDeque::with_capacity(self.capacity));
            if buf.len() >= self.capacity {
                buf.pop_front();
            }
            buf.push_back(val);
        }
        let gx = data.fields.get("g_force_x").copied().unwrap_or(0.0);
        let gy = data.fields.get("g_force_y").copied().unwrap_or(0.0);
        let gz = data.fields.get("g_force_z").copied().unwrap_or(0.0);
        let mag = (gx * gx + gy * gy + gz * gz).sqrt();
        if mag > self.max_g_magnitude {
            self.max_g_magnitude = mag;
        }
    }

    pub fn get(&self, field: &str) -> Option<&VecDeque<f64>> {
        self.fields.get(field)
    }

    pub fn stats(&self, field: &str) -> Option<(f64, f64, f64)> {
        let buf = self.fields.get(field)?;
        if buf.is_empty() {
            return None;
        }
        let min = buf.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = buf.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let avg = buf.iter().sum::<f64>() / buf.len() as f64;
        Some((min, max, avg))
    }
}

// ---------------------------------------------------------------------------
// Per-sensor status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SensorStatus {
    pub name: String,
    pub driver: String,
    pub connection_display: String,
    pub enabled: bool,
    pub connected: bool,
    pub last_error: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub sensor_statuses: HashMap<String, SensorStatus>,
    pub sensor_data: HashMap<String, SensorData>,
    pub sensor_history: HashMap<String, SensorHistory>,
    pub mqtt_status: MqttStatus,
    pub messages_published: Arc<AtomicU64>,
    pub mqtt_address: String,
    pub mqtt_enabled: bool,
    pub log_buffer: VecDeque<String>,
    pub log_capacity: usize,
    pub selected_tab: usize,
}

impl AppState {
    pub fn new(mqtt_address: String, mqtt_enabled: bool, log_capacity: usize) -> Self {
        Self {
            sensor_statuses: HashMap::new(),
            sensor_data: HashMap::new(),
            sensor_history: HashMap::new(),
            mqtt_status: if mqtt_enabled {
                MqttStatus::Connecting
            } else {
                MqttStatus::Disabled
            },
            messages_published: Arc::new(AtomicU64::new(0)),
            mqtt_address,
            mqtt_enabled,
            log_buffer: VecDeque::with_capacity(log_capacity),
            log_capacity,
            selected_tab: 0,
        }
    }

    pub fn add_log(&mut self, line: String) {
        if self.log_buffer.len() >= self.log_capacity {
            self.log_buffer.pop_front();
        }
        self.log_buffer.push_back(line);
    }

    pub fn sensor_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.sensor_statuses.keys().cloned().collect();
        names.sort();
        names
    }
}

pub type SharedState = Arc<RwLock<AppState>>;

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::SensorData;
    use chrono::Utc;

    fn make_sensor_data(pairs: &[(&str, f64)]) -> SensorData {
        SensorData {
            timestamp: Utc::now(),
            fields: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    // --- MqttStatus ---

    #[test]
    fn test_mqtt_status_is_connected() {
        assert!(MqttStatus::Connected.is_connected());
        assert!(!MqttStatus::Disconnected.is_connected());
        assert!(!MqttStatus::Disabled.is_connected());
        assert!(!MqttStatus::Connecting.is_connected());
        assert!(!MqttStatus::Error("oops".into()).is_connected());
    }

    #[test]
    fn test_mqtt_status_label() {
        assert_eq!(MqttStatus::Connected.label(), "CONNECTED");
        assert_eq!(MqttStatus::Disabled.label(), "DISABLED");
        assert_eq!(MqttStatus::Connecting.label(), "CONNECTING");
        assert_eq!(MqttStatus::Disconnected.label(), "DISCONNECTED");
        assert_eq!(MqttStatus::Error("x".into()).label(), "ERROR");
    }

    // --- SensorHistory ---

    #[test]
    fn test_history_push_single_field() {
        let mut h = SensorHistory::new(10);
        h.push(&make_sensor_data(&[("temp", 22.0)]));
        assert_eq!(h.get("temp").unwrap().back(), Some(&22.0));
    }

    #[test]
    fn test_history_capacity_respected() {
        let mut h = SensorHistory::new(5);
        for i in 0..10 {
            h.push(&make_sensor_data(&[("val", i as f64)]));
        }
        let buf = h.get("val").unwrap();
        assert_eq!(buf.len(), 5, "should not exceed capacity");
        assert_eq!(*buf.back().unwrap(), 9.0, "last value should be 9");
        assert_eq!(*buf.front().unwrap(), 5.0, "oldest should be 5");
    }

    #[test]
    fn test_history_stats() {
        let mut h = SensorHistory::new(100);
        for v in [1.0_f64, 2.0, 3.0, 4.0, 5.0] {
            h.push(&make_sensor_data(&[("x", v)]));
        }
        let (min, max, avg) = h.stats("x").unwrap();
        assert!((min - 1.0).abs() < 1e-9);
        assert!((max - 5.0).abs() < 1e-9);
        assert!((avg - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_history_stats_missing_key() {
        let h = SensorHistory::new(10);
        assert!(h.stats("nonexistent").is_none());
    }

    #[test]
    fn test_history_tracks_max_g() {
        let mut h = SensorHistory::new(100);
        h.push(&make_sensor_data(&[
            ("g_force_x", 2.0),
            ("g_force_y", 0.0),
            ("g_force_z", 0.0),
        ]));
        h.push(&make_sensor_data(&[
            ("g_force_x", 0.0),
            ("g_force_y", 3.0),
            ("g_force_z", 0.0),
        ]));
        // max_g should be 3.0 (from second push)
        assert!((h.max_g_magnitude - 3.0).abs() < 1e-9);
    }

    // --- AppState ---

    #[test]
    fn test_appstate_new_mqtt_disabled() {
        let s = AppState::new("localhost:1883".into(), false, 100);
        assert_eq!(s.mqtt_status, MqttStatus::Disabled);
    }

    #[test]
    fn test_appstate_new_mqtt_enabled() {
        let s = AppState::new("localhost:1883".into(), true, 100);
        assert_eq!(s.mqtt_status, MqttStatus::Connecting);
    }

    #[test]
    fn test_appstate_add_log_rotation() {
        let mut s = AppState::new("".into(), false, 3);
        s.add_log("a".into());
        s.add_log("b".into());
        s.add_log("c".into());
        s.add_log("d".into()); // should evict "a"
        assert_eq!(s.log_buffer.len(), 3);
        assert_eq!(s.log_buffer.front().unwrap(), "b");
    }

    #[test]
    fn test_appstate_sensor_names_sorted() {
        let mut s = AppState::new("".into(), false, 10);
        for name in ["zebra", "alpha", "mango"] {
            s.sensor_statuses.insert(
                name.into(),
                SensorStatus {
                    name: name.into(),
                    driver: "synthetic".into(),
                    connection_display: "".into(),
                    enabled: true,
                    connected: false,
                    last_error: None,
                },
            );
        }
        let names = s.sensor_names();
        assert_eq!(names, vec!["alpha", "mango", "zebra"]);
    }
}
