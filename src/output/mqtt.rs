//! MQTT output (rumqttc, rustls).
//!
//! | Format          | Topic(s)                                  | Retained | Rate                          |
//! |-----------------|-------------------------------------------|----------|-------------------------------|
//! | `canonical`     | `<base>/<sensor>/telemetry` (G86 envelope) | never   | every reading                 |
//! | `json`          | `<base>/<sensor>/state`                   | yes      | ≤ `max_rate_hz` per sensor    |
//! | `legacy`        | `<base>/IMU/<sensor>/{INFO,FILTERED,DERIVED}` | no   | ≤ `max_rate_hz` per sensor    |
//! | `homeassistant` | `<prefix>/sensor/<node>/<channel>/config` | yes      | on connect                    |
//!
//! `<base>/status` is `online`/`offline` (retained; `offline` is the last will).
//! `<base>/health` is the bridge health document (retained, every 5 s and on
//! a state change; see [`super::health`]).
//!
//! **On demand** (for consumers on slow links, such as the G86 displays): a
//! message on `<base>/telemetry/get` whose payload is a reply topic is
//! answered on that topic with every fresh channel of every sensor. Each
//! request is answered with a reading the consumer hasn't had yet, held up to
//! 2 s for the next one, so a consumer with several requests in flight gets
//! every new reading once and never a backlog. Same contract as
//! ecu-to-mqtt ≥ 0.7.3 and gps-to-mqtt ≥ 0.7.2.
//!
//! Publishing never blocks acquisition: while the broker is unreachable live
//! readings are dropped and counted, and state is re-sent after reconnecting.

use super::health;
use crate::channels;
use crate::config::{AppConfig, MqttFormat};
use crate::models::{CHANNEL_MAX_AGE_MS, Counters, MqttStatus, Shared, Stamped};
use anyhow::{Context, Result};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS, Transport};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const RECONNECT_MAX: Duration = Duration::from_secs(60);

/// Per reply topic: bursts of up to DEMAND_BURST requests are answered at
/// once (a display keeps several in flight, and Wi-Fi delivers them in
/// bursts), at most DEMAND_RATE per second on average.
const DEMAND_BURST: f32 = 12.0;
const DEMAND_RATE: f32 = 100.0;
/// A request waits at most this long for a new reading.
const DEMAND_HOLD: Duration = Duration::from_secs(2);
const DEMAND_MAX_CLIENTS: usize = 32;

/// Token bucket for on-demand answers to one reply topic.
#[derive(Debug, Clone, Copy)]
pub struct DemandBucket {
    tokens: f32,
    at: Instant,
}

impl DemandBucket {
    pub fn new(now: Instant) -> Self {
        DemandBucket {
            tokens: DEMAND_BURST,
            at: now,
        }
    }

    /// One answer if the bucket has a token; refills at DEMAND_RATE per second.
    pub fn take(&mut self, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.at).as_secs_f32();
        self.tokens = (self.tokens + dt * DEMAND_RATE).min(DEMAND_BURST);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// One consumer asking on demand: its rate bucket and the requests it holds
/// open. A request is answered with the next reading it hasn't had yet, so
/// it gets every new reading once, as soon as it exists, and never a
/// duplicate (that would only cost airtime on a weak link).
#[derive(Debug, Clone, Copy)]
pub struct DemandClient {
    bucket: DemandBucket,
    pending: u8,
    last_seq: Option<u64>,
    since: Instant,
}

impl DemandClient {
    pub fn new(now: Instant) -> Self {
        DemandClient {
            bucket: DemandBucket::new(now),
            pending: 0,
            last_seq: None,
            since: now,
        }
    }

    /// A request arrived; false if over the rate (dropped).
    pub fn request(&mut self, now: Instant) -> bool {
        if !self.bucket.take(now) {
            return false;
        }
        self.pending = (self.pending + 1).min(DEMAND_BURST as u8);
        self.since = now;
        true
    }

    /// Whether to answer one held request now with reading `seq` (None: no
    /// fresh reading). Held requests expire after DEMAND_HOLD.
    pub fn ready(&mut self, seq: Option<u64>, now: Instant) -> bool {
        if self.pending == 0 {
            return false;
        }
        if now.saturating_duration_since(self.since) > DEMAND_HOLD {
            self.pending = 0;
            return false;
        }
        match seq {
            Some(s) if self.last_seq != Some(s) => {
                self.last_seq = Some(s);
                self.pending -= 1;
                true
            }
            _ => false,
        }
    }

    pub fn last_seen(&self) -> Instant {
        self.since
    }
}

/// A usable reply topic: 1–128 bytes of UTF-8, no wildcards, not a `$` system topic.
pub fn valid_reply_topic(payload: &[u8]) -> Option<&str> {
    let t = std::str::from_utf8(payload).ok()?.trim();
    (!t.is_empty() && t.len() <= 128 && !t.contains(['+', '#', '\0']) && !t.starts_with('$'))
        .then_some(t)
}

/// The reading an on-demand answer would carry now, or None (no fresh data).
pub fn demand_sequence(shared: &Shared) -> Option<u64> {
    let s = shared.read();
    if s.sample_age_s(Instant::now())? * 1000.0 > CHANNEL_MAX_AGE_MS as f64 {
        return None;
    }
    Some(s.last_sequence)
}

/// Every fresh channel of every sensor, shaped like the canonical envelope
/// (`partial: false`). `None` when no sensor is delivering.
pub fn demand_reply(shared: &Shared) -> Option<String> {
    let now = Instant::now();
    let s = shared.read();
    if s.sample_age_s(now)? * 1000.0 > CHANNEL_MAX_AGE_MS as f64 {
        return None;
    }
    let mut captured = 0u64;
    let channels: serde_json::Map<String, serde_json::Value> = s
        .fresh_channels(now)
        .map(|(k, c)| {
            captured = captured.max(c.captured_ms);
            (k.clone(), json!(c.value))
        })
        .collect();
    if channels.is_empty() {
        return None;
    }
    Some(
        json!({
            "schema": 1,
            "source": "sensors",
            "partial": false,
            "bootId": shared.boot_id,
            "timestampMs": captured,
            "sequence": s.last_sequence,
            "channels": channels,
        })
        .to_string(),
    )
}

/// `base` + `sub` with exactly one slash between them.
pub fn topic(base: &str, sub: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        sub.trim_start_matches('/')
    )
}

fn qos(level: u8) -> QoS {
    match level {
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}

pub fn client_id(config: &AppConfig) -> String {
    config
        .mqtt
        .client_id
        .clone()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos();
            format!(
                "sensors-to-mqtt-{:x}{:04x}",
                std::process::id(),
                nanos & 0xffff
            )
        })
}

fn options(config: &AppConfig, id: &str) -> Result<MqttOptions> {
    let m = &config.mqtt;
    let mut o = MqttOptions::new(id, &m.host, m.port);
    o.set_keep_alive(Duration::from_secs(m.keep_alive_secs.clamp(5, 3600)));
    o.set_clean_session(true);
    o.set_request_channel_capacity(1024);
    o.set_max_packet_size(256 * 1024, 256 * 1024);
    o.set_last_will(LastWill::new(
        topic(&m.base_topic, "status"),
        "offline",
        QoS::AtLeastOnce,
        true,
    ));
    if let Some(user) = &m.username {
        o.set_credentials(user, m.password()?.unwrap_or_default());
    }
    if m.use_tls || m.ca_cert_path.is_some() {
        let transport = match &m.ca_cert_path {
            Some(ca) => {
                let ca = std::fs::read(ca).with_context(|| format!("reading {ca}"))?;
                let client_auth = match (&m.client_cert_path, &m.client_key_path) {
                    (Some(c), Some(k)) => Some((
                        std::fs::read(c).with_context(|| format!("reading {c}"))?,
                        std::fs::read(k).with_context(|| format!("reading {k}"))?,
                    )),
                    _ => None,
                };
                Transport::tls(ca, client_auth, None)
            }
            None => Transport::tls_with_default_config(),
        };
        o.set_transport(transport);
    }
    Ok(o)
}

/// The `<base>/<sensor>/state` document: every field, and the channels.
pub fn state_document(stamped: &Stamped) -> serde_json::Value {
    let fields: std::collections::BTreeMap<&String, &f64> = stamped
        .fields
        .iter()
        .filter(|(_, v)| v.is_finite())
        .collect();
    let channels: serde_json::Map<String, serde_json::Value> = stamped
        .channels
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    json!({
        "timestampMs": stamped.captured_ms,
        "sequence": stamped.sequence,
        "sensor": stamped.sensor,
        "driver": stamped.driver,
        "fields": fields,
        "channels": channels,
    })
}

/// The topics of 0.5: `IMU/<sensor>/INFO`, `FILTERED` and `DERIVED`.
pub fn legacy_documents(base: &str, stamped: &Stamped) -> Vec<(String, String)> {
    const FILTERED: &[&str] = &[
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
    const DERIVED: &[&str] = &[
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
    let ts = chrono::DateTime::from_timestamp_millis(stamped.captured_ms as i64)
        .unwrap_or_default()
        .to_rfc3339();
    let pick = |keys: &[&str]| {
        let mut m = serde_json::Map::new();
        m.insert("timestamp".into(), json!(ts));
        for k in keys {
            if let Some(v) = stamped.fields.get(*k).filter(|v| v.is_finite()) {
                m.insert((*k).into(), json!(v));
            }
        }
        serde_json::Value::Object(m).to_string()
    };
    let t = |leaf: &str| topic(base, &format!("IMU/{}/{leaf}", stamped.sensor));
    vec![
        (
            t("INFO"),
            json!({"sensor": stamped.sensor, "timestamp": ts}).to_string(),
        ),
        (t("FILTERED"), pick(FILTERED)),
        (t("DERIVED"), pick(DERIVED)),
    ]
}

fn node_id(raw: &str) -> String {
    let n: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let n = n.trim_matches('_').to_string();
    if n.is_empty() {
        "sensors_to_mqtt".into()
    } else {
        n
    }
}

/// Home Assistant device class for a channel, from its unit.
fn device_class(channel: &str, unit: &str) -> Option<&'static str> {
    match (channel, unit) {
        ("batterySocPct", _) => Some("battery"),
        ("humidityPct", _) => Some("humidity"),
        ("pm25", _) => Some("pm25"),
        ("pm10", _) => Some("pm10"),
        (_, "°C") => Some("temperature"),
        (_, "kPa") => Some("pressure"),
        (_, "V") => Some("voltage"),
        (_, "A") => Some("current"),
        (_, "W") => Some("power"),
        (_, "lx") => Some("illuminance"),
        _ => None,
    }
}

/// Home Assistant discovery documents: one sensor entity per channel, reading
/// its sensor's state topic.
pub fn homeassistant_discovery(config: &AppConfig) -> Vec<(String, String)> {
    let m = &config.mqtt;
    let base = m.base_topic.trim_end_matches('/');
    let node = node_id(m.client_id.as_deref().unwrap_or(base));
    let avail = topic(base, "status");
    let prefix = m.homeassistant_prefix.trim_end_matches('/');
    let mut out = Vec::new();
    for s in config.sensors.iter().filter(|s| s.enabled) {
        let state = topic(base, &format!("{}/state", s.name));
        for spec in channels::channels_for(s) {
            let info = channels::info(&spec.channel);
            let label = if info.label.is_empty() {
                spec.channel.as_str()
            } else {
                info.label
            };
            let mut doc = json!({
                "name": label,
                "unique_id": format!("{node}_{}", spec.channel.to_lowercase()),
                "state_topic": state,
                "value_template": format!("{{{{ value_json.channels.{} | default(none) }}}}", spec.channel),
                "availability_topic": avail,
                "expire_after": 10,
                "device": {
                    "identifiers": [format!("sensors_to_mqtt_{node}")],
                    "name": format!("Sensors {base}"),
                    "manufacturer": "sensors-to-mqtt",
                    "model": "sensors-to-mqtt",
                    "sw_version": env!("CARGO_PKG_VERSION"),
                },
            });
            let o = doc.as_object_mut().expect("object");
            if !info.unit.is_empty() {
                o.insert("unit_of_measurement".into(), json!(info.unit));
                o.insert("state_class".into(), json!("measurement"));
            }
            if let Some(c) = device_class(&spec.channel, info.unit) {
                o.insert("device_class".into(), json!(c));
            }
            out.push((
                format!(
                    "{prefix}/sensor/{node}/{}/config",
                    spec.channel.to_lowercase()
                ),
                doc.to_string(),
            ));
        }
    }
    out
}

pub async fn run(
    config: Arc<AppConfig>,
    shared: Arc<Shared>,
    cancel: CancellationToken,
) -> Result<()> {
    let id = client_id(&config);
    let (client, mut eventloop) = AsyncClient::new(options(&config, &id)?, 1024);
    let (conn_tx, conn_rx) = watch::channel(false);
    info!("MQTT client {id} → {}", config.mqtt.address());

    let el_shared = shared.clone();
    let el_cancel = cancel.clone();
    let el_client = client.clone();
    let demand_topic = topic(&config.mqtt.base_topic, "telemetry/get");
    let event_loop = tokio::spawn(async move {
        let mut delay = Duration::from_secs(1);
        let mut answered: HashMap<String, DemandClient> = HashMap::new();
        // releases held requests when a new reading exists
        let mut tick = tokio::time::interval(Duration::from_millis(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let answer = |reply: String| {
            if let Some(doc) = demand_reply(&el_shared) {
                match el_client.try_publish(reply, QoS::AtMostOnce, false, doc) {
                    Ok(()) => Counters::inc(&el_shared.counters.mqtt_published),
                    Err(_) => Counters::inc(&el_shared.counters.mqtt_dropped),
                }
            }
        };
        el_shared.set_mqtt(MqttStatus::Connecting);
        loop {
            let polled = tokio::select! {
                r = eventloop.poll() => r,
                _ = tick.tick(), if answered.values().any(|c| c.pending > 0) => {
                    let now = Instant::now();
                    let seq = demand_sequence(&el_shared);
                    let due: Vec<String> = answered
                        .iter_mut()
                        .filter_map(|(reply, c)| c.ready(seq, now).then(|| reply.clone()))
                        .collect();
                    for reply in due {
                        answer(reply);
                    }
                    continue;
                }
                _ = el_cancel.cancelled() => {
                    let _ = tokio::time::timeout(Duration::from_secs(1), async {
                        while eventloop.poll().await.is_ok() {}
                    }).await;
                    break;
                }
            };
            match polled {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    info!("MQTT connected");
                    let _ = el_client.try_subscribe(demand_topic.clone(), QoS::AtMostOnce);
                    delay = Duration::from_secs(1);
                    let _ = conn_tx.send(true);
                    el_shared.set_mqtt(MqttStatus::Connected);
                }
                Ok(Event::Incoming(Packet::Publish(p))) if p.topic == demand_topic => {
                    let Some(reply) = valid_reply_topic(&p.payload) else {
                        continue;
                    };
                    let now = Instant::now();
                    if answered.len() >= DEMAND_MAX_CLIENTS && !answered.contains_key(reply) {
                        answered.retain(|_, c| {
                            now.duration_since(c.last_seen()) < Duration::from_secs(10)
                        });
                        if answered.len() >= DEMAND_MAX_CLIENTS {
                            continue;
                        }
                    }
                    let c = answered
                        .entry(reply.to_string())
                        .or_insert_with(|| DemandClient::new(now));
                    if !c.request(now) {
                        continue;
                    }
                    // answered at once if there is a reading it hasn't had, else held for the next one
                    if c.ready(demand_sequence(&el_shared), now) {
                        answer(reply.to_string());
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = conn_tx.send(false);
                    el_shared.set_mqtt(MqttStatus::Disconnected);
                    warn!("MQTT connection error: {e}; retrying in {delay:?}");
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = el_cancel.cancelled() => break,
                    }
                    delay = (delay * 2).min(RECONNECT_MAX);
                    el_shared.set_mqtt(MqttStatus::Connecting);
                }
            }
        }
        el_shared.set_mqtt(MqttStatus::Disconnected);
    });

    let mut publisher = Publisher::new(config, client, shared.clone());
    publisher.run(shared.bus.subscribe(), conn_rx, cancel).await;
    if let Err(e) = event_loop.await {
        error!("MQTT event loop: {e}");
    }
    Ok(())
}

struct Publisher {
    config: Arc<AppConfig>,
    client: AsyncClient,
    shared: Arc<Shared>,
    qos: QoS,
    min_interval: Option<Duration>,
    /// Per sensor: when its state was last published, and its latest reading.
    last_state_at: HashMap<String, Instant>,
    latest: HashMap<String, Arc<Stamped>>,
    health: health::Tracker,
    sources: Vec<String>,
    /// Last published `(state, link)` and when.
    health_sent: Option<(&'static str, &'static str, Instant)>,
}

impl Publisher {
    fn new(config: Arc<AppConfig>, client: AsyncClient, shared: Arc<Shared>) -> Self {
        let min_interval = (config.mqtt.max_rate_hz > 0.0)
            .then(|| Duration::from_secs_f64(1.0 / config.mqtt.max_rate_hz));
        Self {
            qos: qos(config.mqtt.qos),
            client,
            shared,
            min_interval,
            last_state_at: HashMap::new(),
            latest: HashMap::new(),
            health: health::Tracker::default(),
            sources: health::sources(&config),
            health_sent: None,
            config,
        }
    }

    /// `<base>/health`, retained: when forced, when `state`/`link` changed,
    /// or every 5 s.
    fn publish_health(&mut self, force: bool) {
        let (doc, state, link) = health::document(&self.shared, &mut self.health, &self.sources);
        let now = Instant::now();
        let due = match self.health_sent {
            Some((s, l, at)) => {
                force || s != state || l != link || now.duration_since(at) >= health::PERIOD
            }
            None => true,
        };
        if due {
            let t = topic(&self.config.mqtt.base_topic, "health");
            if self.send(t, doc.to_string(), true, QoS::AtLeastOnce) {
                self.health_sent = Some((state, link, now));
            }
        }
    }

    fn send(&self, topic: String, payload: String, retain: bool, qos: QoS) -> bool {
        match self.client.try_publish(topic, qos, retain, payload) {
            Ok(()) => {
                Counters::inc(&self.shared.counters.mqtt_published);
                true
            }
            Err(_) => {
                Counters::inc(&self.shared.counters.mqtt_dropped);
                false
            }
        }
    }

    fn on_connect(&mut self) {
        self.last_state_at.clear();
        let base = self.config.mqtt.base_topic.clone();
        self.send(
            topic(&base, "status"),
            "online".into(),
            true,
            QoS::AtLeastOnce,
        );
        if self.config.mqtt.wants(MqttFormat::Homeassistant) {
            for (t, doc) in homeassistant_discovery(&self.config) {
                self.send(t, doc, true, QoS::AtLeastOnce);
            }
        }
        let latest: Vec<Arc<Stamped>> = self.latest.values().cloned().collect();
        for stamped in latest {
            self.publish_state(&stamped, true);
        }
        self.publish_health(true);
    }

    /// The per-sensor state document and legacy topics, rate-limited per sensor.
    fn publish_state(&mut self, stamped: &Stamped, force: bool) {
        let now = Instant::now();
        if !force
            && let (Some(min), Some(last)) =
                (self.min_interval, self.last_state_at.get(&stamped.sensor))
            && now.duration_since(*last) < min
        {
            return;
        }
        self.last_state_at.insert(stamped.sensor.clone(), now);
        let m = &self.config.mqtt;
        let base = m.base_topic.clone();
        if m.wants(MqttFormat::Json) || m.wants(MqttFormat::Homeassistant) {
            let doc = state_document(stamped).to_string();
            self.send(
                topic(&base, &format!("{}/state", stamped.sensor)),
                doc,
                true,
                self.qos,
            );
        }
        if m.wants(MqttFormat::Legacy) {
            for (t, doc) in legacy_documents(&base, stamped) {
                self.send(t, doc, false, self.qos);
            }
        }
    }

    fn on_sample(&mut self, stamped: Arc<Stamped>, connected: bool) {
        self.latest.insert(stamped.sensor.clone(), stamped.clone());
        if stamped.envelope.is_some() {
            self.health.record(Instant::now(), stamped.captured_ms);
        }
        if !connected {
            Counters::inc(&self.shared.counters.mqtt_dropped);
            return;
        }
        if self.config.mqtt.wants(MqttFormat::Canonical)
            && let Some(envelope) = &stamped.envelope
        {
            // Never retained: consumers reject replayed frames as stale.
            let t = topic(
                &self.config.mqtt.base_topic,
                &format!("{}/telemetry", stamped.sensor),
            );
            self.send(t, envelope.to_string(), false, self.qos);
        }
        self.publish_state(&stamped, false);
    }

    async fn run(
        &mut self,
        mut bus: broadcast::Receiver<Arc<Stamped>>,
        mut conn: watch::Receiver<bool>,
        cancel: CancellationToken,
    ) {
        let mut connected = false;
        let mut health_tick = tokio::time::interval(Duration::from_secs(1));
        health_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = health_tick.tick() => if connected { self.publish_health(false); },
                changed = conn.changed() => {
                    if changed.is_err() { break; }
                    connected = *conn.borrow_and_update();
                    if connected { self.on_connect(); }
                }
                event = bus.recv() => match event {
                    Ok(stamped) => self.on_sample(stamped, connected),
                    Err(broadcast::error::RecvError::Lagged(n)) => Counters::add(&self.shared.counters.mqtt_dropped, n),
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
        if connected {
            let (mut doc, _, _) = health::document(&self.shared, &mut self.health, &self.sources);
            health::shutdown_document(&mut doc);
            self.send(
                topic(&self.config.mqtt.base_topic, "health"),
                doc.to_string(),
                true,
                QoS::AtLeastOnce,
            );
            self.send(
                topic(&self.config.mqtt.base_topic, "status"),
                "offline".into(),
                true,
                QoS::AtLeastOnce,
            );
            let _ = self.client.try_disconnect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConnectionConfig, I2cConnectionConfig, SensorConfig};
    use crate::models::{ChannelState, Info};

    fn stamped(sensor: &str, fields: &[(&str, f64)], channels: &[(&str, f64)]) -> Stamped {
        Stamped {
            sequence: 9,
            sensor: sensor.into(),
            driver: "mpu6500".into(),
            captured_ms: 1_700_000_000_000,
            fields: fields.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            channels: channels.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            envelope: Some(Arc::from("{}")),
        }
    }

    #[test]
    fn demand_client_answers_each_reading_once() {
        let t0 = Instant::now();
        let mut c = DemandClient::new(t0);
        for _ in 0..3 {
            assert!(c.request(t0));
        }
        assert!(c.ready(Some(1), t0), "a reading it hasn't had: at once");
        assert!(!c.ready(Some(1), t0), "the same reading again: held");
        assert!(!c.ready(None, t0), "no fresh data: held");
        assert!(
            c.ready(Some(2), t0 + Duration::from_millis(20)),
            "the next reading releases one"
        );
        assert!(c.ready(Some(3), t0 + Duration::from_millis(40)));
        assert!(
            !c.ready(Some(4), t0 + Duration::from_millis(60)),
            "all three answered"
        );
        // held requests expire
        assert!(c.request(t0 + Duration::from_millis(100)));
        assert!(!c.ready(
            Some(4),
            t0 + Duration::from_millis(100) + DEMAND_HOLD + Duration::from_millis(1)
        ));
        assert!(!c.ready(Some(5), t0 + Duration::from_secs(10)));
    }

    #[test]
    fn demand_bucket_allows_bursts_and_caps_the_rate() {
        let t0 = Instant::now();
        let mut b = DemandBucket::new(t0);
        for _ in 0..12 {
            assert!(b.take(t0));
        }
        assert!(!b.take(t0), "beyond the burst");
        assert!(b.take(t0 + Duration::from_millis(10)));
        assert!(!b.take(t0 + Duration::from_millis(11)));
        let mut n = 0;
        for ms in 0..1000u64 {
            if b.take(t0 + Duration::from_millis(20 + ms)) {
                n += 1;
            }
        }
        assert!((95..=110).contains(&n), "{n} answered");
    }

    #[test]
    fn demand_reply_topics_are_validated() {
        assert_eq!(
            valid_reply_topic(b"/GOLF86/DISPLAY/g86-info-2b84/imu"),
            Some("/GOLF86/DISPLAY/g86-info-2b84/imu")
        );
        assert_eq!(valid_reply_topic(b" a/b \n"), Some("a/b"));
        for bad in [
            &b""[..],
            b"a/+/b",
            b"a/#",
            b"$SYS/x",
            &[0xff, 0xfe],
            &[b'a'; 129],
        ] {
            assert_eq!(valid_reply_topic(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn demand_reply_merges_fresh_channels_of_all_sensors() {
        let shared = Shared::new(Info::default(), false);
        assert_eq!(
            demand_reply(&shared),
            None,
            "no answer before the first reading"
        );
        {
            let mut s = shared.write();
            s.last_sample_at = Some(Instant::now());
            s.last_sequence = 41;
            for (k, v, sensor, age) in [
                ("lateralG", 0.4, "imu", 0),
                ("ambientC", 21.0, "climate", 0),
                ("lightLux", 300.0, "light", 5000),
            ] {
                s.channels.insert(
                    k.into(),
                    ChannelState {
                        value: v,
                        captured_ms: 7,
                        sensor: sensor.into(),
                        at: Instant::now() - Duration::from_millis(age),
                    },
                );
            }
        }
        assert_eq!(demand_sequence(&shared), Some(41));
        let doc: serde_json::Value = serde_json::from_str(&demand_reply(&shared).unwrap()).unwrap();
        assert_eq!(doc["schema"], 1);
        assert_eq!(doc["partial"], false);
        assert_eq!(doc["sequence"], 41);
        assert_eq!(doc["channels"]["lateralG"], 0.4);
        assert_eq!(doc["channels"]["ambientC"], 21.0);
        assert!(
            doc["channels"].get("lightLux").is_none(),
            "stale channels are left out"
        );
        shared.write().last_sample_at = Some(Instant::now() - Duration::from_secs(5));
        assert_eq!(demand_reply(&shared), None, "stale data is not answered");
        assert_eq!(demand_sequence(&shared), None);
    }

    #[test]
    fn topics_join_with_one_slash() {
        assert_eq!(topic("/SENSORS/", "telemetry"), "/SENSORS/telemetry");
        assert_eq!(
            topic("/GOLF86/SENSORS", "/imu/state"),
            "/GOLF86/SENSORS/imu/state"
        );
    }

    #[test]
    fn qos_levels_map() {
        assert_eq!(qos(0), QoS::AtMostOnce);
        assert_eq!(qos(1), QoS::AtLeastOnce);
        assert_eq!(qos(2), QoS::ExactlyOnce);
    }

    #[test]
    fn state_and_legacy_documents() {
        let s = stamped(
            "imu",
            &[("accel_x", 0.1), ("g_force_x", 0.2), ("bad", f64::NAN)],
            &[("lateralG", 0.2)],
        );
        let doc = state_document(&s);
        assert_eq!(doc["sensor"], "imu");
        assert_eq!(doc["fields"]["accel_x"], 0.1);
        assert!(doc["fields"].get("bad").is_none(), "NaN is not valid JSON");
        assert_eq!(doc["channels"]["lateralG"], 0.2);
        let legacy = legacy_documents("/SENSORS", &s);
        assert_eq!(legacy[0].0, "/SENSORS/IMU/imu/INFO");
        assert_eq!(legacy[1].0, "/SENSORS/IMU/imu/FILTERED");
        let filtered: serde_json::Value = serde_json::from_str(&legacy[1].1).unwrap();
        assert_eq!(filtered["accel_x"], 0.1);
        assert!(filtered.get("g_force_x").is_none());
        let derived: serde_json::Value = serde_json::from_str(&legacy[2].1).unwrap();
        assert_eq!(derived["g_force_x"], 0.2);
    }

    #[tokio::test]
    async fn state_is_rate_limited_per_sensor() {
        let mut config = AppConfig::default();
        config.mqtt.max_rate_hz = 10.0;
        let config = Arc::new(config);
        let shared = Shared::new(Info::default(), false);
        let (client, _eventloop) = AsyncClient::new(options(&config, "t").unwrap(), 1024);
        let mut p = Publisher::new(config, client, shared.clone());
        let published = || Counters::get(&shared.counters.mqtt_published);
        let a = Arc::new(stamped("a", &[("x", 1.0)], &[("lateralG", 1.0)]));
        let b = Arc::new(Stamped {
            envelope: None,
            ..stamped("b", &[("x", 1.0)], &[])
        });
        p.on_sample(a.clone(), true);
        assert_eq!(published(), 2, "envelope and state");
        p.on_sample(a.clone(), true);
        assert_eq!(published(), 3, "envelope only: state is throttled");
        p.on_sample(b, true);
        assert_eq!(
            published(),
            4,
            "another sensor has its own limit; no channels, no envelope"
        );
        p.on_sample(a, false);
        assert_eq!(
            Counters::get(&shared.counters.mqtt_dropped),
            1,
            "offline readings are counted"
        );
        p.on_connect();
        assert_eq!(
            published(),
            4 + 1 + 2 + 1,
            "status, each sensor's latest state, then health"
        );
    }

    #[test]
    fn homeassistant_documents_read_the_state_topic() {
        let mut config = AppConfig::default();
        config.mqtt.base_topic = "/CAR/SENSORS/".into();
        config.sensors.push(SensorConfig {
            name: "climate".into(),
            enabled: true,
            driver: "bme280".into(),
            connection: ConnectionConfig::I2c(I2cConnectionConfig {
                device: "/dev/i2c-1".into(),
                address: 0x76,
            }),
            settings: None,
            channels: None,
        });
        let docs = homeassistant_discovery(&config);
        assert_eq!(docs.len(), 4);
        let (t, doc) = docs
            .iter()
            .find(|(t, _)| t.ends_with("/ambientc/config"))
            .unwrap();
        assert_eq!(t, "homeassistant/sensor/car_sensors/ambientc/config");
        let v: serde_json::Value = serde_json::from_str(doc).unwrap();
        assert_eq!(v["state_topic"], "/CAR/SENSORS/climate/state");
        assert_eq!(v["availability_topic"], "/CAR/SENSORS/status");
        assert_eq!(
            v["value_template"],
            "{{ value_json.channels.ambientC | default(none) }}"
        );
        assert_eq!(v["device_class"], "temperature");
        assert_eq!(v["unit_of_measurement"], "°C");
    }
}
