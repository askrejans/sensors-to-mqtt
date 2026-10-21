//! Process-wide state shared by the sensor threads, hub, outputs, HTTP API
//! and TUI.
//!
//! * Bus — lossy broadcast of stamped readings. A slow consumer lags and
//!   skips; it never holds up a sensor.
//! * [`AppState`] — latest values for request/response readers, behind a std
//!   `RwLock` that is never held across `.await`.
//! * [`Counters`] — lock-free statistics for Prometheus and the dashboards.

use crate::sensors::SensorData;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Instant;
use tokio::sync::broadcast;

/// Channels older than this are stale (the G86 consumers' 2 s window).
pub const CHANNEL_MAX_AGE_MS: u64 = 2000;
const BUS_CAPACITY: usize = 1024;

// ---------------------------------------------------------------------------
// MQTT status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MqttStatus {
    Disabled,
    Connecting,
    Connected,
    Disconnected,
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

#[derive(Debug, Clone, Serialize)]
pub struct SensorStatus {
    pub name: String,
    pub driver: String,
    pub connection_display: String,
    pub enabled: bool,
    pub connected: bool,
    pub last_error: Option<String>,
    /// Readings in the last second.
    pub rate_hz: f64,
    #[serde(skip)]
    pub last_read_at: Option<Instant>,
    #[serde(skip)]
    pub reads_this_second: u64,
}

impl SensorStatus {
    pub fn new(name: &str, driver: &str, connection_display: String, enabled: bool) -> Self {
        Self {
            name: name.into(),
            driver: driver.into(),
            connection_display,
            enabled,
            connected: false,
            last_error: None,
            rate_hz: 0.0,
            last_read_at: None,
            reads_this_second: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Readings on the bus
// ---------------------------------------------------------------------------

/// One reading of one sensor, sequenced by the hub.
#[derive(Debug, Clone)]
pub struct Stamped {
    pub sequence: u64,
    pub sensor: String,
    pub driver: String,
    pub captured_ms: u64,
    pub fields: HashMap<String, f64>,
    /// Canonical channels of this reading (may be empty).
    pub channels: Vec<(String, f64)>,
    /// The canonical envelope (`None` when the reading has no channels).
    pub envelope: Option<Arc<str>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelState {
    pub value: f64,
    pub captured_ms: u64,
    pub sensor: String,
    #[serde(skip)]
    pub at: Instant,
}

#[derive(Default)]
pub struct Counters {
    pub readings: AtomicU64,
    pub read_errors: AtomicU64,
    pub reconnects: AtomicU64,
    pub mqtt_published: AtomicU64,
    pub mqtt_dropped: AtomicU64,
}

impl Counters {
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
    pub fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }
    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

/// Static description of how the process is wired.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Info {
    pub version: &'static str,
    pub mqtt: Option<String>,
    pub outputs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub sensor_statuses: HashMap<String, SensorStatus>,
    pub sensor_data: HashMap<String, SensorData>,
    pub sensor_history: HashMap<String, SensorHistory>,
    /// Merged canonical channels of every sensor.
    pub channels: BTreeMap<String, ChannelState>,
    pub mqtt_status: MqttStatus,
    pub last_sequence: u64,
    pub last_sample_at: Option<Instant>,
    /// Readings per second, all sensors.
    pub sample_rate: f64,
    pub selected_tab: usize,
}

impl AppState {
    pub fn new(mqtt_enabled: bool) -> Self {
        Self {
            sensor_statuses: HashMap::new(),
            sensor_data: HashMap::new(),
            sensor_history: HashMap::new(),
            channels: BTreeMap::new(),
            mqtt_status: if mqtt_enabled {
                MqttStatus::Connecting
            } else {
                MqttStatus::Disabled
            },
            last_sequence: 0,
            last_sample_at: None,
            sample_rate: 0.0,
            selected_tab: 0,
        }
    }

    pub fn sensor_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.sensor_statuses.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn sample_age_s(&self, now: Instant) -> Option<f64> {
        self.last_sample_at
            .map(|t| now.saturating_duration_since(t).as_secs_f64())
    }

    /// Channels updated within [`CHANNEL_MAX_AGE_MS`].
    pub fn fresh_channels(&self, now: Instant) -> impl Iterator<Item = (&String, &ChannelState)> {
        self.channels.iter().filter(move |(_, c)| {
            now.saturating_duration_since(c.at).as_millis() as u64 <= CHANNEL_MAX_AGE_MS
        })
    }
}

pub struct Shared {
    pub info: Info,
    pub counters: Counters,
    pub bus: broadcast::Sender<Arc<Stamped>>,
    pub boot_id: String,
    pub started: Instant,
    state: RwLock<AppState>,
}

pub type SharedState = Arc<Shared>;

impl Shared {
    pub fn new(info: Info, mqtt_enabled: bool) -> Arc<Self> {
        let (bus, _) = broadcast::channel(BUS_CAPACITY);
        Arc::new(Self {
            info,
            counters: Counters::default(),
            bus,
            boot_id: new_boot_id(),
            started: Instant::now(),
            state: RwLock::new(AppState::new(mqtt_enabled)),
        })
    }

    pub fn read(&self) -> RwLockReadGuard<'_, AppState> {
        self.state.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, AppState> {
        self.state.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_mqtt(&self, status: MqttStatus) {
        self.write().mqtt_status = status;
    }

    /// `ok`: every enabled sensor delivers; `degraded`: some do;
    /// `no_data`: none (or no sensor is configured).
    pub fn health(&self) -> Health {
        let now = Instant::now();
        let s = self.read();
        let enabled: Vec<&SensorStatus> =
            s.sensor_statuses.values().filter(|st| st.enabled).collect();
        let live = enabled
            .iter()
            .filter(|st| {
                st.connected
                    && st.last_read_at.is_some_and(|t| {
                        now.saturating_duration_since(t).as_millis() as u64
                            <= CHANNEL_MAX_AGE_MS.max(3000)
                    })
            })
            .count();
        let status = if !enabled.is_empty() && live == enabled.len() {
            "ok"
        } else if live > 0 {
            "degraded"
        } else {
            "no_data"
        };
        Health {
            status,
            sensors: enabled.len(),
            sensors_live: live,
            mqtt: s.mqtt_status.clone(),
            sample_age_s: s.sample_age_s(now),
            sample_rate_hz: s.sample_rate,
            uptime_s: now.duration_since(self.started).as_secs(),
            version: self.info.version,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub sensors: usize,
    pub sensors_live: usize,
    pub mqtt: MqttStatus,
    pub sample_age_s: Option<f64>,
    pub sample_rate_hz: f64,
    pub uptime_s: u64,
    pub version: &'static str,
}

impl Health {
    pub fn is_ok(&self) -> bool {
        self.status == "ok"
    }
}

/// Distinguishes restarts of the process in the envelope (`bootId`), so a
/// consumer knows a restarted sequence is not a replay.
pub fn new_boot_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}-{:x}", std::process::id(), nanos & 0xffff_ffff_ffff)
}

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
    }

    #[test]
    fn test_mqtt_status_label() {
        assert_eq!(MqttStatus::Connected.label(), "CONNECTED");
        assert_eq!(MqttStatus::Disabled.label(), "DISABLED");
        assert_eq!(MqttStatus::Connecting.label(), "CONNECTING");
        assert_eq!(MqttStatus::Disconnected.label(), "DISCONNECTED");
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
    fn test_appstate_mqtt_status() {
        assert_eq!(AppState::new(false).mqtt_status, MqttStatus::Disabled);
        assert_eq!(AppState::new(true).mqtt_status, MqttStatus::Connecting);
    }

    #[test]
    fn test_appstate_sensor_names_sorted() {
        let mut s = AppState::new(false);
        for name in ["zebra", "alpha", "mango"] {
            s.sensor_statuses.insert(
                name.into(),
                SensorStatus::new(name, "synthetic", String::new(), true),
            );
        }
        let names = s.sensor_names();
        assert_eq!(names, vec!["alpha", "mango", "zebra"]);
    }

    #[test]
    fn fresh_channels_excludes_stale_ones() {
        let mut s = AppState::new(false);
        let now = Instant::now();
        for (name, at) in [
            ("lateralG", now),
            ("ambientC", now - std::time::Duration::from_secs(5)),
        ] {
            s.channels.insert(
                name.into(),
                ChannelState {
                    value: 1.0,
                    captured_ms: 0,
                    sensor: "x".into(),
                    at,
                },
            );
        }
        let fresh: Vec<&String> = s.fresh_channels(now).map(|(k, _)| k).collect();
        assert_eq!(fresh, ["lateralG"]);
    }

    #[test]
    fn health_counts_live_sensors() {
        let shared = Shared::new(Info::default(), false);
        assert_eq!(shared.health().status, "no_data");
        {
            let mut s = shared.write();
            for name in ["a", "b"] {
                s.sensor_statuses.insert(
                    name.into(),
                    SensorStatus::new(name, "synthetic", String::new(), true),
                );
            }
            let a = s.sensor_statuses.get_mut("a").unwrap();
            a.connected = true;
            a.last_read_at = Some(Instant::now());
        }
        assert_eq!(shared.health().status, "degraded");
        {
            let mut s = shared.write();
            let b = s.sensor_statuses.get_mut("b").unwrap();
            b.connected = true;
            b.last_read_at = Some(Instant::now());
        }
        let h = shared.health();
        assert!(h.is_ok());
        assert_eq!((h.sensors, h.sensors_live), (2, 2));
    }
}
