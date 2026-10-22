//! Bridge health on `<base>/health` (G86 car computer contract, §2).
//!
//! One retained JSON document for the whole bridge, published every 5 s and
//! at once when `state` or `link` changes:
//!
//! ```json
//! {"schema": 1, "bridge": "sensors-to-mqtt", "version": "0.6.1", "bootId": "…",
//!  "timestampMs": 1792664130000, "state": "ok", "link": "connected",
//!  "sources": ["sensors/mpu6500/imu"], "rateHz": 50.0, "ageMs": 12,
//!  "channels": ["lateralG", "longitudinalG"], "errors": {"read": 0, "init": 0, "reconnects": 0},
//!  "clockSynced": true, "sensors": {"imu": "ok"}}
//! ```
//!
//! * `state`: `down` when no enabled sensor delivers (nothing in 10 s);
//!   `degraded` when a sensor is down, the newest sample is older than 2 s,
//!   or an error counter rose in the last 5 s; otherwise `ok`.
//! * `link`: `connected` when a sensor reads, `disconnected` after a failed
//!   open or read, `connecting` before the first attempt finished.

use crate::channels;
use crate::config::AppConfig;
use crate::models::{Counters, Shared};
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

/// Rate window and how long a rising error counter keeps `degraded`.
pub const WINDOW: Duration = Duration::from_secs(5);
/// Samples older than this make the bridge `degraded`.
pub const STALE: Duration = Duration::from_secs(2);
/// No sample for this long: `down`.
pub const DOWN_AFTER: Duration = Duration::from_secs(10);
/// Periodic republish.
pub const PERIOD: Duration = Duration::from_secs(5);

/// What the health document needs of one sensor.
#[derive(Debug, Clone)]
pub struct SensorView {
    pub name: String,
    pub connected: bool,
    /// Time since its last reading.
    pub age: Option<Duration>,
    /// A failed open or read is on record.
    pub failed: bool,
}

pub fn sensor_state(v: &SensorView) -> &'static str {
    match v.age {
        Some(a) if v.connected && a <= STALE => "ok",
        Some(a) if v.connected && a <= DOWN_AFTER => "degraded",
        _ => "down",
    }
}

/// `(state, link)` of the bridge.
pub fn evaluate(
    sensors: &[SensorView],
    newest_age: Option<Duration>,
    errors_rising: bool,
) -> (&'static str, &'static str) {
    let link = if sensors.iter().any(|s| s.connected) {
        "connected"
    } else if sensors.iter().any(|s| s.failed) {
        "disconnected"
    } else {
        "connecting"
    };
    let states: Vec<&str> = sensors.iter().map(sensor_state).collect();
    let delivering = states.iter().any(|s| *s != "down");
    let state = if !delivering || newest_age.is_none_or(|a| a > DOWN_AFTER) {
        "down"
    } else if states.contains(&"down")
        || states.contains(&"degraded")
        || newest_age.is_some_and(|a| a > STALE)
        || errors_rising
    {
        "degraded"
    } else {
        "ok"
    };
    (state, link)
}

/// Rate, sample age and error trend, fed by the MQTT publisher.
#[derive(Debug, Default)]
pub struct Tracker {
    samples: VecDeque<Instant>,
    first: Option<Instant>,
    newest: Option<Instant>,
    newest_captured_ms: Option<u64>,
    errors: u64,
    errors_rose_at: Option<Instant>,
}

impl Tracker {
    /// One canonical sample (captured at `captured_ms`, Unix ms).
    pub fn record(&mut self, now: Instant, captured_ms: u64) {
        self.first.get_or_insert(now);
        self.newest = Some(now);
        self.newest_captured_ms = Some(
            self.newest_captured_ms
                .map_or(captured_ms, |c| c.max(captured_ms)),
        );
        self.samples.push_back(now);
        self.trim(now);
    }

    fn trim(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > WINDOW)
        {
            self.samples.pop_front();
        }
    }

    /// Samples per second over the last 5 s (or since the first sample).
    pub fn rate_hz(&mut self, now: Instant) -> f64 {
        self.trim(now);
        let Some(first) = self.first else { return 0.0 };
        let span = now
            .saturating_duration_since(first)
            .clamp(Duration::from_secs(1), WINDOW);
        (self.samples.len() as f64 / span.as_secs_f64() * 10.0).round() / 10.0
    }

    /// Track the sum of the error counters; true while it rose in the last 5 s.
    pub fn errors_rising(&mut self, total: u64, now: Instant) -> bool {
        if total > self.errors {
            self.errors_rose_at = Some(now);
        }
        self.errors = total;
        self.errors_rose_at
            .is_some_and(|t| now.saturating_duration_since(t) <= WINDOW)
    }

    pub fn newest_age(&self, now: Instant) -> Option<Duration> {
        self.newest.map(|t| now.saturating_duration_since(t))
    }

    /// Wall-clock age of the newest sample's capture time.
    pub fn age_ms(&self, now_ms: u64) -> Option<u64> {
        self.newest_captured_ms.map(|c| now_ms.saturating_sub(c))
    }
}

/// Envelope sources of the enabled sensors that publish channels.
pub fn sources(config: &AppConfig) -> Vec<String> {
    let mut out: Vec<String> = config
        .sensors
        .iter()
        .filter(|s| s.enabled && !channels::channels_for(s).is_empty())
        .map(|s| format!("sensors/{}/{}", s.driver, s.name))
        .collect();
    out.sort();
    out
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// The health document now, with its `(state, link)`.
pub fn document(
    shared: &Shared,
    tracker: &mut Tracker,
    sources: &[String],
) -> (serde_json::Value, &'static str, &'static str) {
    let now = Instant::now();
    let c = &shared.counters;
    let (read, init, reconnects) = (
        Counters::get(&c.read_errors),
        Counters::get(&c.init_errors),
        Counters::get(&c.reconnects),
    );
    let rising = tracker.errors_rising(read + init + reconnects, now);
    let s = shared.read();
    let views: Vec<SensorView> = s
        .sensor_statuses
        .values()
        .filter(|st| st.enabled)
        .map(|st| SensorView {
            name: st.name.clone(),
            connected: st.connected,
            age: st.last_read_at.map(|t| now.saturating_duration_since(t)),
            failed: st.last_error.is_some(),
        })
        .collect();
    let (state, link) = evaluate(&views, tracker.newest_age(now), rising);
    let per_sensor: BTreeMap<&str, &str> = views
        .iter()
        .map(|v| (v.name.as_str(), sensor_state(v)))
        .collect();
    let channels: Vec<&String> = s.fresh_channels(now).map(|(k, _)| k).collect();
    let doc = json!({
        "schema": 1,
        "bridge": "sensors-to-mqtt",
        "version": env!("CARGO_PKG_VERSION"),
        "bootId": shared.boot_id,
        "timestampMs": now_ms(),
        "state": state,
        "link": link,
        "sources": sources,
        "rateHz": tracker.rate_hz(now),
        "ageMs": tracker.age_ms(now_ms()),
        "channels": channels,
        "errors": {"read": read, "init": init, "reconnects": reconnects},
        "clockSynced": clock_synced(),
        "sensors": per_sensor,
    });
    (doc, state, link)
}

/// The final document of a clean shutdown: `down`, input closed.
pub fn shutdown_document(doc: &mut serde_json::Value) {
    doc["state"] = json!("down");
    doc["link"] = json!("disconnected");
    doc["rateHz"] = json!(0.0);
    doc["channels"] = json!([]);
    if let Some(sensors) = doc["sensors"].as_object_mut() {
        for v in sensors.values_mut() {
            *v = json!("down");
        }
    }
}

/// Whether the kernel clock is synchronised (NTP/chrony/GPS discipline).
#[cfg(target_os = "linux")]
pub fn clock_synced() -> bool {
    // SAFETY: adjtimex with modes = 0 only reads the kernel clock state into
    // a zeroed, properly sized struct.
    let mut tx: libc::timex = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::adjtimex(&mut tx) };
    ret >= 0 && ret != libc::TIME_ERROR && tx.status & libc::STA_UNSYNC == 0
}

/// Other systems keep their own clock in sync; assume it is.
#[cfg(not(target_os = "linux"))]
pub fn clock_synced() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SensorConfig;
    use crate::models::{Info, SensorStatus};

    fn view(connected: bool, age_ms: Option<u64>, failed: bool) -> SensorView {
        SensorView {
            name: "x".into(),
            connected,
            age: age_ms.map(Duration::from_millis),
            failed,
        }
    }
    const MS: fn(u64) -> Option<Duration> = |ms| Some(Duration::from_millis(ms));

    #[test]
    fn state_machine() {
        // nothing configured or nothing read yet
        assert_eq!(evaluate(&[], None, false), ("down", "connecting"));
        assert_eq!(
            evaluate(&[view(false, None, false)], None, false),
            ("down", "connecting")
        );
        assert_eq!(
            evaluate(&[view(false, None, true)], None, false),
            ("down", "disconnected")
        );
        // one sensor delivering
        assert_eq!(
            evaluate(&[view(true, Some(20), false)], MS(20), false),
            ("ok", "connected")
        );
        // error counter rising
        assert_eq!(
            evaluate(&[view(true, Some(20), false)], MS(20), true),
            ("degraded", "connected")
        );
        // samples older than 2 s
        assert_eq!(
            evaluate(&[view(true, Some(3000), false)], MS(3000), false),
            ("degraded", "connected")
        );
        // one of two sensors down
        assert_eq!(
            evaluate(
                &[view(true, Some(20), false), view(false, None, true)],
                MS(20),
                false
            ),
            ("degraded", "connected")
        );
        // nothing for over 10 s
        assert_eq!(
            evaluate(&[view(true, Some(11_000), false)], MS(11_000), false),
            ("down", "connected")
        );
    }

    #[test]
    fn tracker_rate_age_and_errors() {
        let mut t = Tracker::default();
        let start = Instant::now();
        assert_eq!(t.rate_hz(start), 0.0);
        assert_eq!(t.age_ms(1000), None);
        for i in 0..250u64 {
            t.record(start + Duration::from_millis(i * 20), 1000 + i * 20);
        }
        let now = start + Duration::from_millis(5000);
        assert!((t.rate_hz(now) - 50.0).abs() <= 0.2, "{}", t.rate_hz(now));
        assert_eq!(t.age_ms(5990), Some(10));
        // ten seconds later only the trimmed window is counted
        assert_eq!(t.rate_hz(now + Duration::from_secs(10)), 0.0);
        assert!(!t.errors_rising(0, now));
        assert!(t.errors_rising(2, now));
        assert!(t.errors_rising(2, now + Duration::from_secs(4)));
        assert!(!t.errors_rising(2, now + Duration::from_secs(6)));
    }

    #[test]
    fn document_matches_the_contract() {
        let shared = Shared::new(Info::default(), true);
        {
            let mut s = shared.write();
            let mut st = SensorStatus::new("imu", "synthetic", String::new(), true);
            st.connected = true;
            st.last_read_at = Some(Instant::now());
            s.sensor_statuses.insert("imu".into(), st);
        }
        let mut t = Tracker::default();
        t.record(Instant::now(), now_ms());
        let sources = vec!["sensors/synthetic/imu".to_string()];
        let (doc, state, link) = document(&shared, &mut t, &sources);
        assert_eq!((state, link), ("ok", "connected"));
        let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
        for k in [
            "schema",
            "bridge",
            "version",
            "bootId",
            "timestampMs",
            "state",
            "link",
            "sources",
            "rateHz",
            "ageMs",
            "channels",
            "errors",
            "clockSynced",
        ] {
            assert!(keys.iter().any(|x| *x == k), "missing {k}");
        }
        assert_eq!(doc["schema"], 1);
        assert_eq!(doc["bridge"], "sensors-to-mqtt");
        assert_eq!(doc["bootId"], shared.boot_id.as_str());
        assert_eq!(doc["sources"][0], "sensors/synthetic/imu");
        assert_eq!(doc["errors"]["read"], 0);
        assert_eq!(doc["sensors"]["imu"], "ok");
        assert!(doc["ageMs"].as_u64().unwrap() < 1000);
        assert!(doc["clockSynced"].is_boolean());

        let mut last = doc.clone();
        shutdown_document(&mut last);
        assert_eq!(last["state"], "down");
        assert_eq!(last["link"], "disconnected");
        assert_eq!(last["sensors"]["imu"], "down");
    }

    #[test]
    fn sources_list_enabled_channel_sensors() {
        let sensor = |name: &str, driver: &str, enabled: bool| SensorConfig {
            name: name.into(),
            enabled,
            driver: driver.into(),
            connection: crate::config::ConnectionConfig::I2c(crate::config::I2cConnectionConfig {
                device: "/dev/i2c-1".into(),
                address: 0x68,
            }),
            settings: None,
            channels: None,
        };
        let config = AppConfig {
            sensors: vec![
                sensor("imu", "mpu6500", true),
                sensor("off", "bme280", false),
                sensor("cab", "bme280", true),
            ],
            ..AppConfig::default()
        };
        assert_eq!(
            sources(&config),
            ["sensors/bme280/cab", "sensors/mpu6500/imu"]
        );
    }
}
