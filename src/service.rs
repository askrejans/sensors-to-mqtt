//! Per-sensor Tokio tasks and service lifecycle.

use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::SensorConfig;
use crate::models::{AppState, SensorHistory, SensorStatus, SharedState};
use crate::mqtt_handler::MqttHandle;
use crate::sensors::registry::create_sensor;
use crate::sensors::{Sensor, SensorData};

// ---------------------------------------------------------------------------
// Sensor reading event
// ---------------------------------------------------------------------------

pub struct SensorEvent {
    pub name: String,
    pub data: SensorData,
}

// ---------------------------------------------------------------------------
// Initialise AppState with sensor stubs (before tasks start)
// ---------------------------------------------------------------------------

pub fn register_sensors(state: &mut AppState, sensors: &[SensorConfig]) {
    for cfg in sensors {
        let history_size = cfg
            .settings
            .as_ref()
            .and_then(|v| v.get("history_size"))
            .and_then(|v| v.as_integer())
            .unwrap_or(600) as usize;

        state.sensor_statuses.insert(
            cfg.name.clone(),
            SensorStatus {
                name: cfg.name.clone(),
                driver: cfg.driver.clone(),
                connection_display: cfg.connection.to_display(),
                enabled: cfg.enabled,
                connected: false,
                last_error: None,
            },
        );

        state
            .sensor_history
            .insert(cfg.name.clone(), SensorHistory::new(history_size));
    }
}

// ---------------------------------------------------------------------------
// Spawn one async task per sensor
// ---------------------------------------------------------------------------

/// Spawn a task that continuously reads the sensor and pushes events.
/// Uses `spawn_blocking` for the blocking I2C read.
pub fn spawn_sensor_task(
    cfg: SensorConfig,
    state: SharedState,
    mqtt: Option<MqttHandle>,
    cancel: CancellationToken,
    base_topic: String,
) {
    tokio::spawn(async move {
        let name = cfg.name.clone();
        info!("Starting sensor task for '{}'", name);

        // Build the driver
        let sensor_result = tokio::task::spawn_blocking({
            let cfg2 = cfg.clone();
            move || create_sensor(&cfg2)
        })
        .await;

        let mut sensor: Box<dyn Sensor> = match sensor_result {
            Ok(Ok(s)) => {
                update_status(&state, &cfg.name, true, None).await;
                s
            }
            Ok(Err(e)) => {
                error!("Failed to initialise sensor '{}': {}", name, e);
                update_status(&state, &cfg.name, false, Some(e.to_string())).await;
                return;
            }
            Err(e) => {
                error!("Panic creating sensor '{}': {}", name, e);
                return;
            }
        };

        let interval_ms = 20u64; // 50 Hz; driver sample_rate limits actual rate

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("Sensor task '{}' cancelled", name);
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(interval_ms)) => {}
            }

            // Read (blocking) in a thread pool
            let read_result = {
                // We need to move the sensor into spawn_blocking, but it's borrowed.
                // Pattern: read synchronously here since I2C reads are ~1ms.
                // For long-blocking drivers, restructure to Arc<Mutex<Box<dyn Sensor>>>.
                match sensor.read() {
                    Ok(data) => Ok(data),
                    Err(e) => Err(e.to_string()),
                }
            };

            match read_result {
                Ok(data) => {
                    update_status(&state, &name, true, None).await;
                    push_data(&state, &name, data.clone()).await;
                    if let Some(ref h) = mqtt {
                        publish_sensor_data(h, &base_topic, &name, &data).await;
                    }
                }
                Err(e) => {
                    warn!("Read error on '{}': {}", name, e);
                    update_status(&state, &name, false, Some(e)).await;
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// State helpers
// ---------------------------------------------------------------------------

async fn update_status(state: &SharedState, name: &str, connected: bool, error: Option<String>) {
    let mut s = state.write().await;
    if let Some(st) = s.sensor_statuses.get_mut(name) {
        st.connected = connected;
        st.last_error = error;
    }
}

async fn push_data(state: &SharedState, name: &str, data: SensorData) {
    let mut s = state.write().await;
    if let Some(hist) = s.sensor_history.get_mut(name) {
        hist.push(&data);
    }
    s.sensor_data.insert(name.to_string(), data);
}

// ---------------------------------------------------------------------------
// MQTT publishing helper
// ---------------------------------------------------------------------------

async fn publish_sensor_data(mqtt: &MqttHandle, base_topic: &str, name: &str, data: &SensorData) {
    use serde_json::json;

    let ts = data.timestamp.to_rfc3339();

    // INFO
    let info_payload = json!({ "sensor": name, "timestamp": ts }).to_string();
    mqtt.publish(format!("{}/IMU/{}/INFO", base_topic, name), info_payload)
        .await;

    // Build separate filtered and derived maps
    let filtered_keys = [
        "accel_x",
        "accel_y",
        "accel_z",
        "gyro_x",
        "gyro_y",
        "gyro_z",
        "roll_rate",
        "pitch_rate",
        "yaw_rate",
    ];
    let mut filtered = serde_json::Map::new();
    filtered.insert("timestamp".into(), json!(ts));
    for key in &filtered_keys {
        if let Some(&v) = data.fields.get(*key) {
            filtered.insert(key.to_string(), json!(v));
        }
    }
    mqtt.publish(
        format!("{}/IMU/{}/FILTERED", base_topic, name),
        serde_json::Value::Object(filtered).to_string(),
    )
    .await;

    // DERIVED — everything else
    let derived_keys = [
        "g_force_x",
        "g_force_y",
        "g_force_z",
        "combined_g",
        "peak_g",
        "lean_angle",
        "bank_angle",
        "tilt_angle",
        "angular_velocity",
    ];
    let mut derived = serde_json::Map::new();
    derived.insert("timestamp".into(), json!(ts));
    for key in &derived_keys {
        if let Some(&v) = data.fields.get(*key) {
            derived.insert(key.to_string(), json!(v));
        }
    }
    mqtt.publish(
        format!("{}/IMU/{}/DERIVED", base_topic, name),
        serde_json::Value::Object(derived).to_string(),
    )
    .await;
}
