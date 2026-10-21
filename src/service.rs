//! Sensor threads and the hub.
//!
//! Every enabled sensor gets its own OS thread: driver reads block (I2C
//! transfers, conversion waits, a 2 s serial timeout), and must never stall
//! the async runtime or another sensor. A thread reads on a fixed schedule,
//! reopens its driver with backoff after failures, and sends events to the
//! hub.
//!
//! The hub is the single consumer of readings: it maps fields to canonical
//! channels, sequences every reading, builds the envelope, updates the
//! snapshot and broadcasts to the outputs. It never awaits an output.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::channels::{self, ChannelSpec};
use crate::config::SensorConfig;
use crate::models::{
    AppState, ChannelState, Counters, SensorHistory, SensorStatus, Shared, Stamped,
};
use crate::sensors::registry::create_sensor;
use crate::sensors::{Sensor, SensorData};

const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);
/// Consecutive read errors before the driver is reopened.
const MAX_READ_ERRORS: u32 = 10;

// ---------------------------------------------------------------------------
// Events from the sensor threads
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SensorEvent {
    Reading {
        sensor: String,
        data: SensorData,
    },
    Status {
        sensor: String,
        connected: bool,
        error: Option<String>,
    },
    ReadError {
        sensor: String,
        error: String,
    },
}

// ---------------------------------------------------------------------------
// Initialise AppState with sensor stubs (before threads start)
// ---------------------------------------------------------------------------

pub fn register_sensors(state: &mut AppState, sensors: &[SensorConfig]) {
    for cfg in sensors {
        let history_size = cfg
            .settings
            .as_ref()
            .and_then(|v| v.get("history_size"))
            .and_then(|v| v.as_integer())
            .unwrap_or(600)
            .clamp(10, 100_000) as usize;

        state.sensor_statuses.insert(
            cfg.name.clone(),
            SensorStatus::new(
                &cfg.name,
                &cfg.driver,
                cfg.connection.to_display(),
                cfg.enabled,
            ),
        );

        state
            .sensor_history
            .insert(cfg.name.clone(), SensorHistory::new(history_size));
    }
}

// ---------------------------------------------------------------------------
// One thread per sensor
// ---------------------------------------------------------------------------

/// Sleep up to `d`, waking early on cancel. False when cancelled.
fn pause(d: Duration, cancel: &CancellationToken) -> bool {
    let until = Instant::now() + d;
    while !cancel.is_cancelled() {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
    false
}

pub fn spawn_sensor_thread(
    cfg: SensorConfig,
    tx: mpsc::Sender<SensorEvent>,
    cancel: CancellationToken,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name(format!("sensor-{}", cfg.name))
        .spawn(move || sensor_loop(cfg, tx, cancel))
}

fn open(cfg: &SensorConfig) -> anyhow::Result<Box<dyn Sensor>> {
    let mut sensor = create_sensor(cfg)?;
    sensor.init()?;
    Ok(sensor)
}

fn sensor_loop(cfg: SensorConfig, tx: mpsc::Sender<SensorEvent>, cancel: CancellationToken) {
    let name = cfg.name.clone();
    let interval = Duration::from_millis(cfg.interval_ms());
    let send = |e: SensorEvent| tx.blocking_send(e).is_ok();
    let mut retry = RETRY_MIN;
    info!(
        "Sensor '{name}' ({}) every {} ms",
        cfg.driver,
        interval.as_millis()
    );

    'reopen: while !cancel.is_cancelled() {
        let mut sensor = match open(&cfg) {
            Ok(s) => {
                retry = RETRY_MIN;
                if !send(SensorEvent::Status {
                    sensor: name.clone(),
                    connected: true,
                    error: None,
                }) {
                    return;
                }
                s
            }
            Err(e) => {
                warn!("Sensor '{name}': {e:#}; retrying in {retry:?}");
                if !send(SensorEvent::Status {
                    sensor: name.clone(),
                    connected: false,
                    error: Some(format!("{e:#}")),
                }) {
                    return;
                }
                if !pause(retry, &cancel) {
                    return;
                }
                retry = (retry * 2).min(RETRY_MAX);
                continue;
            }
        };

        let mut errors = 0u32;
        // fixed schedule: a slow read shortens the next wait instead of
        // stretching the period; after an overrun it restarts from now
        let mut next = Instant::now();
        while !cancel.is_cancelled() {
            match sensor.read() {
                Ok(data) => {
                    errors = 0;
                    if !send(SensorEvent::Reading {
                        sensor: name.clone(),
                        data,
                    }) {
                        return;
                    }
                }
                Err(e) => {
                    errors += 1;
                    if !send(SensorEvent::ReadError {
                        sensor: name.clone(),
                        error: format!("{e:#}"),
                    }) {
                        return;
                    }
                    if errors >= MAX_READ_ERRORS {
                        warn!("Sensor '{name}': {errors} read errors in a row ({e:#}); reopening");
                        let _ = send(SensorEvent::Status {
                            sensor: name.clone(),
                            connected: false,
                            error: Some(format!("{e:#}")),
                        });
                        drop(sensor);
                        if !pause(retry, &cancel) {
                            return;
                        }
                        retry = (retry * 2).min(RETRY_MAX);
                        continue 'reopen;
                    }
                }
            }
            next += interval;
            let now = Instant::now();
            if next <= now {
                next = now;
                continue;
            }
            if !pause(next - now, &cancel) {
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------

/// Per-sensor data the hub needs to stamp readings.
struct HubSensor {
    driver: String,
    specs: Vec<ChannelSpec>,
}

/// The canonical envelope of one reading (G86 schema 1). Each sensor is its
/// own source (`sensors/<driver>/<name>`, on `<base>/<name>/telemetry`), and a
/// reading carries every channel of its sensor, so it is never partial.
pub fn envelope(stamped: &Stamped, boot_id: &str) -> String {
    let channels: serde_json::Map<String, serde_json::Value> = stamped
        .channels
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    json!({
        "schema": 1,
        "source": format!("sensors/{}/{}", stamped.driver, stamped.sensor),
        "partial": false,
        "bootId": boot_id,
        "timestampMs": stamped.captured_ms,
        "sequence": stamped.sequence,
        "channels": channels,
    })
    .to_string()
}

pub async fn run_hub(
    mut rx: mpsc::Receiver<SensorEvent>,
    shared: Arc<Shared>,
    sensors: Vec<SensorConfig>,
    cancel: CancellationToken,
) {
    let enabled: Vec<&SensorConfig> = sensors.iter().filter(|s| s.enabled).collect();
    let table: HashMap<String, HubSensor> = enabled
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                HubSensor {
                    driver: s.driver.clone(),
                    specs: channels::channels_for(s),
                },
            )
        })
        .collect();
    let mut sequence = 0u64;
    let mut this_second = 0u64;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            event = rx.recv() => match event {
                Some(SensorEvent::Reading { sensor, data }) => {
                    let Some(hs) = table.get(&sensor) else { continue };
                    sequence += 1;
                    this_second += 1;
                    let mut stamped = Stamped {
                        sequence,
                        captured_ms: data.timestamp.timestamp_millis().max(0) as u64,
                        channels: channels::apply(&hs.specs, &data.fields),
                        fields: data.fields.clone(),
                        driver: hs.driver.clone(),
                        sensor,
                        envelope: None,
                    };
                    if !stamped.channels.is_empty() {
                        stamped.envelope = Some(Arc::from(envelope(&stamped, &shared.boot_id)));
                    }
                    record(&shared, &stamped, data);
                    let _ = shared.bus.send(Arc::new(stamped));
                }
                Some(SensorEvent::Status { sensor, connected, error }) => {
                    let mut s = shared.write();
                    if let Some(st) = s.sensor_statuses.get_mut(&sensor) {
                        if !connected && st.connected {
                            Counters::inc(&shared.counters.reconnects);
                        }
                        st.connected = connected;
                        st.last_error = error;
                    }
                }
                Some(SensorEvent::ReadError { sensor, error }) => {
                    Counters::inc(&shared.counters.read_errors);
                    if let Some(st) = shared.write().sensor_statuses.get_mut(&sensor) {
                        st.last_error = Some(error);
                    }
                }
                None => break,
            },
            _ = tick.tick() => {
                let mut s = shared.write();
                s.sample_rate = this_second as f64;
                for st in s.sensor_statuses.values_mut() {
                    st.rate_hz = st.reads_this_second as f64;
                    st.reads_this_second = 0;
                }
                this_second = 0;
            }
        }
    }
    info!("Hub stopped");
}

fn record(shared: &Shared, stamped: &Stamped, data: SensorData) {
    Counters::inc(&shared.counters.readings);
    let now = Instant::now();
    let mut s = shared.write();
    s.last_sample_at = Some(now);
    s.last_sequence = stamped.sequence;
    for (name, value) in &stamped.channels {
        s.channels.insert(
            name.clone(),
            ChannelState {
                value: *value,
                captured_ms: stamped.captured_ms,
                sensor: stamped.sensor.clone(),
                at: now,
            },
        );
    }
    if let Some(st) = s.sensor_statuses.get_mut(&stamped.sensor) {
        st.connected = true;
        st.last_error = None;
        st.last_read_at = Some(now);
        st.reads_this_second += 1;
    }
    if let Some(h) = s.sensor_history.get_mut(&stamped.sensor) {
        h.push(&data);
    }
    s.sensor_data.insert(stamped.sensor.clone(), data);
}

/// Start the hub and one thread per enabled sensor.
pub fn start(
    shared: &Arc<Shared>,
    sensors: &[SensorConfig],
    cancel: &CancellationToken,
) -> (
    tokio::task::JoinHandle<()>,
    Vec<std::thread::JoinHandle<()>>,
) {
    let (tx, rx) = mpsc::channel(512);
    let hub = tokio::spawn(run_hub(
        rx,
        shared.clone(),
        sensors.to_vec(),
        cancel.clone(),
    ));
    let mut threads = Vec::new();
    for cfg in sensors {
        if !cfg.enabled {
            info!("Sensor '{}' is disabled, skipping", cfg.name);
            continue;
        }
        match spawn_sensor_thread(cfg.clone(), tx.clone(), cancel.clone()) {
            Ok(t) => threads.push(t),
            Err(e) => error!("Sensor '{}': cannot start its thread: {e}", cfg.name),
        }
    }
    (hub, threads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConnectionConfig, I2cConnectionConfig};
    use crate::models::Info;

    fn synthetic(name: &str) -> SensorConfig {
        SensorConfig {
            name: name.into(),
            enabled: true,
            driver: "synthetic".into(),
            connection: ConnectionConfig::I2c(I2cConnectionConfig {
                device: "/dev/i2c-1".into(),
                address: 0x68,
            }),
            settings: Some(toml::from_str("interval_ms = 5").unwrap()),
            channels: None,
        }
    }

    #[tokio::test]
    async fn readings_are_sequenced_mapped_and_broadcast() {
        let shared = Shared::new(Info::default(), false);
        let sensors = vec![synthetic("imu")];
        register_sensors(&mut shared.write(), &sensors);
        let mut bus = shared.bus.subscribe();
        let cancel = CancellationToken::new();
        let (hub, threads) = start(&shared, &sensors, &cancel);

        let mut seqs = Vec::new();
        for _ in 0..5 {
            let s = tokio::time::timeout(Duration::from_secs(2), bus.recv())
                .await
                .unwrap()
                .unwrap();
            seqs.push(s.sequence);
            let v: serde_json::Value =
                serde_json::from_str(s.envelope.as_deref().unwrap()).unwrap();
            assert_eq!(v["schema"], 1);
            assert_eq!(v["source"], "sensors/synthetic/imu");
            assert_eq!(
                v["partial"], false,
                "a reading carries all of its sensor's channels"
            );
            assert_eq!(v["sequence"], s.sequence);
            assert!(v["channels"]["lateralG"].is_number());
            assert!(
                v["channels"].get("rpm").is_none(),
                "synthetic engine values stay out"
            );
            assert_eq!(v["bootId"], shared.boot_id.as_str());
        }
        assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{seqs:?}");
        {
            let s = shared.read();
            assert!(s.channels.contains_key("ambientC"));
            assert!(s.sensor_statuses["imu"].connected);
            assert!(s.sensor_data.contains_key("imu"));
        }
        assert!(shared.health().is_ok());
        cancel.cancel();
        hub.await.unwrap();
        for t in threads {
            t.join().unwrap();
        }
    }

    #[tokio::test]
    async fn failing_sensor_reports_and_retries() {
        let shared = Shared::new(Info::default(), false);
        let mut bad = synthetic("bad");
        bad.driver = "bmp280".into();
        bad.connection = ConnectionConfig::Tcp(crate::config::TcpConnectionConfig {
            host: "127.0.0.1".into(),
            port: 1, // nothing listens
            address: Some(0x76),
            framing: false,
        });
        let sensors = vec![bad];
        register_sensors(&mut shared.write(), &sensors);
        let cancel = CancellationToken::new();
        let (hub, threads) = start(&shared, &sensors, &cancel);
        let deadline = Instant::now() + Duration::from_secs(3);
        while shared.read().sensor_statuses["bad"].last_error.is_none() && Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(shared.read().sensor_statuses["bad"].last_error.is_some());
        assert!(!shared.read().sensor_statuses["bad"].connected);
        assert_eq!(shared.health().status, "no_data");
        let t0 = Instant::now();
        cancel.cancel();
        hub.await.unwrap();
        for t in threads {
            t.join().unwrap();
        }
        assert!(
            t0.elapsed() < Duration::from_secs(1),
            "the retry wait ends on cancel"
        );
    }

    #[test]
    fn envelope_has_the_seven_contract_keys() {
        let stamped = Stamped {
            sequence: 4,
            sensor: "climate".into(),
            driver: "bme280".into(),
            captured_ms: 1234,
            fields: HashMap::new(),
            channels: vec![("ambientC".into(), 21.5)],
            envelope: None,
        };
        let v: serde_json::Value = serde_json::from_str(&envelope(&stamped, "b")).unwrap();
        assert_eq!(v["partial"], false);
        assert_eq!(v["source"], "sensors/bme280/climate");
        assert_eq!(v["timestampMs"], 1234);
        assert_eq!(v["channels"]["ambientC"], 21.5);
        assert_eq!(
            v.as_object().unwrap().len(),
            7,
            "the envelope has exactly seven keys"
        );
    }
}
