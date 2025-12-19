//! Application state management for the TUI.
//!
//! This module manages the application state including sensor data,
//! UI state, and user interactions.

use crate::sensors::SensorData;
use std::collections::{HashMap, VecDeque};
use chrono::{DateTime, Utc};

/// Maximum number of data points to keep in history
const MAX_HISTORY_SIZE: usize = 600; // 60 seconds at 10ms interval

/// Application state
pub struct App {
    /// Whether the application should quit
    pub should_quit: bool,
    /// Whether sensors are currently measuring
    pub is_measuring: bool,
    /// Index of currently selected sensor
    pub selected_sensor: usize,
    /// Total number of sensors
    pub sensor_count: usize,
    /// Sensor names
    pub sensor_names: Vec<String>,
    /// Sensor enabled states
    pub sensor_enabled: HashMap<String, bool>,
    /// Historical sensor data for charts
    pub sensor_history: HashMap<String, SensorHistory>,
    /// Current sensor data
    pub current_data: HashMap<String, SensorData>,
    /// Status message
    pub status_message: Option<String>,
    /// Error message
    pub error_message: Option<String>,
    /// MQTT connection status
    pub mqtt_connected: bool,
    /// Show help panel
    pub show_help: bool,
}

/// Historical data for a sensor
pub struct SensorHistory {
    /// Timestamps
    pub timestamps: VecDeque<DateTime<Utc>>,
    /// G-force X values
    pub g_force_x: VecDeque<f64>,
    /// G-force Y values
    pub g_force_y: VecDeque<f64>,
    /// G-force Z values
    pub g_force_z: VecDeque<f64>,
    /// Maximum G-force magnitude recorded
    pub max_g_magnitude: f64,
}

impl SensorHistory {
    /// Create a new sensor history
    pub fn new() -> Self {
        Self {
            timestamps: VecDeque::with_capacity(MAX_HISTORY_SIZE),
            g_force_x: VecDeque::with_capacity(MAX_HISTORY_SIZE),
            g_force_y: VecDeque::with_capacity(MAX_HISTORY_SIZE),
            g_force_z: VecDeque::with_capacity(MAX_HISTORY_SIZE),
            max_g_magnitude: 0.0,
        }
    }

    /// Add a data point to history
    pub fn add_data(&mut self, data: &SensorData) {
        // Add timestamp
        if self.timestamps.len() >= MAX_HISTORY_SIZE {
            self.timestamps.pop_front();
        }
        self.timestamps.push_back(data.timestamp);

        // Extract G-forces
        let g_x = data.data.get("g_force_x").copied().unwrap_or(0.0);
        let g_y = data.data.get("g_force_y").copied().unwrap_or(0.0);
        let g_z = data.data.get("g_force_z").copied().unwrap_or(0.0);

        // Add G-force values
        if self.g_force_x.len() >= MAX_HISTORY_SIZE {
            self.g_force_x.pop_front();
        }
        self.g_force_x.push_back(g_x);

        if self.g_force_y.len() >= MAX_HISTORY_SIZE {
            self.g_force_y.pop_front();
        }
        self.g_force_y.push_back(g_y);

        if self.g_force_z.len() >= MAX_HISTORY_SIZE {
            self.g_force_z.pop_front();
        }
        self.g_force_z.push_back(g_z);

        // Update max magnitude
        let magnitude = (g_x * g_x + g_y * g_y + g_z * g_z).sqrt();
        if magnitude > self.max_g_magnitude {
            self.max_g_magnitude = magnitude;
        }
    }

    /// Clear history
    pub fn clear(&mut self) {
        self.timestamps.clear();
        self.g_force_x.clear();
        self.g_force_y.clear();
        self.g_force_z.clear();
        self.max_g_magnitude = 0.0;
    }

    /// Get statistics
    pub fn get_stats(&self) -> HistoryStats {
        let calc_stats = |data: &VecDeque<f64>| {
            if data.is_empty() {
                return (0.0, 0.0, 0.0);
            }
            let sum: f64 = data.iter().sum();
            let avg = sum / data.len() as f64;
            let min = data.iter().copied().fold(f64::INFINITY, f64::min);
            let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            (min, max, avg)
        };

        HistoryStats {
            g_force_x: calc_stats(&self.g_force_x),
            g_force_y: calc_stats(&self.g_force_y),
            g_force_z: calc_stats(&self.g_force_z),
            max_magnitude: self.max_g_magnitude,
        }
    }
}

/// Statistics for historical data
pub struct HistoryStats {
    /// G-force X (min, max, avg)
    pub g_force_x: (f64, f64, f64),
    /// G-force Y (min, max, avg)
    pub g_force_y: (f64, f64, f64),
    /// G-force Z (min, max, avg)
    pub g_force_z: (f64, f64, f64),
    /// Maximum magnitude
    pub max_magnitude: f64,
}

impl App {
    /// Create a new application state
    pub fn new(sensor_names: Vec<String>) -> Self {
        let sensor_count = sensor_names.len();
        let mut sensor_enabled = HashMap::new();
        let mut sensor_history = HashMap::new();

        for name in &sensor_names {
            sensor_enabled.insert(name.clone(), true);
            sensor_history.insert(name.clone(), SensorHistory::new());
        }

        Self {
            should_quit: false,
            is_measuring: true,
            selected_sensor: 0,
            sensor_count,
            sensor_names,
            sensor_enabled,
            sensor_history,
            current_data: HashMap::new(),
            status_message: Some("Application started".to_string()),
            error_message: None,
            mqtt_connected: false,
            show_help: false,
        }
    }

    /// Update current sensor data
    pub fn update_sensor_data(&mut self, sensor_name: &str, data: SensorData) {
        // Update current data
        self.current_data.insert(sensor_name.to_string(), data.clone());

        // Add to history if sensor is enabled
        if self.sensor_enabled.get(sensor_name).copied().unwrap_or(false) {
            if let Some(history) = self.sensor_history.get_mut(sensor_name) {
                history.add_data(&data);
            }
        }
    }

    /// Toggle measurement state
    pub fn toggle_measuring(&mut self) {
        self.is_measuring = !self.is_measuring;
        self.status_message = Some(format!(
            "Measurement {}",
            if self.is_measuring { "started" } else { "paused" }
        ));
    }

    /// Select next sensor
    pub fn next_sensor(&mut self) {
        if self.sensor_count > 0 {
            self.selected_sensor = (self.selected_sensor + 1) % self.sensor_count;
        }
    }

    /// Select previous sensor
    pub fn prev_sensor(&mut self) {
        if self.sensor_count > 0 {
            self.selected_sensor = if self.selected_sensor == 0 {
                self.sensor_count - 1
            } else {
                self.selected_sensor - 1
            };
        }
    }

    /// Toggle selected sensor enabled state
    pub fn toggle_selected_sensor(&mut self) {
        if let Some(name) = self.sensor_names.get(self.selected_sensor) {
            let enabled = self.sensor_enabled.get(name).copied().unwrap_or(true);
            self.sensor_enabled.insert(name.clone(), !enabled);
            self.status_message = Some(format!(
                "Sensor {} {}",
                name,
                if !enabled { "enabled" } else { "disabled" }
            ));
        }
    }

    /// Clear chart history
    pub fn clear_charts(&mut self) {
        for history in self.sensor_history.values_mut() {
            history.clear();
        }
        self.status_message = Some("Charts cleared".to_string());
    }

    /// Toggle help panel
    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    /// Set status message
    pub fn set_status(&mut self, message: String) {
        self.status_message = Some(message);
    }

    /// Set error message
    pub fn set_error(&mut self, message: String) {
        self.error_message = Some(message);
    }

    /// Clear error message
    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    /// Get currently selected sensor name
    pub fn get_selected_sensor_name(&self) -> Option<&str> {
        self.sensor_names.get(self.selected_sensor).map(|s| s.as_str())
    }
}
