//! HTTP API: health probe, Prometheus metrics, REST snapshot, live SSE and the
//! web dashboard.
//!
//! | Route                  | Auth  | Description                                        |
//! |------------------------|-------|----------------------------------------------------|
//! | `GET /health`          | open  | 200 when every enabled sensor delivers, else 503   |
//! | `GET /metrics`         | open  | Prometheus text exposition                         |
//! | `GET /api/v1/status`   | token | wiring, sensors, counters                          |
//! | `GET /api/v1/live`     | token | channels with ages, every sensor's fields          |
//! | `GET /api/v1/channels` | token | channel catalogue (label, unit, range, group)      |
//! | `GET /api/v1/config`   | token | effective configuration, secrets masked            |
//! | `GET /api/v1/stream`   | token | Server-Sent Events `sample`, ≤ 25 Hz per sensor and client |

use crate::channels;
use crate::config::AppConfig;
use crate::models::{Counters, Shared, Stamped};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response, sse},
    routing::get,
};
use futures_util::stream::{self, Stream};
use serde_json::json;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Minimum spacing of SSE readings per sensor and browser connection.
const SSE_MIN_INTERVAL: Duration = Duration::from_millis(40);

#[derive(Clone)]
struct Ctx {
    shared: Arc<Shared>,
    token: Option<Arc<str>>,
    config: Arc<serde_json::Value>,
    /// Channels the configured sensors publish (for the catalogue).
    channels: Arc<Vec<(String, String)>>,
}

#[derive(Default)]
pub struct Options {
    pub token: Option<String>,
    pub cors_origin: Option<String>,
    pub ui: bool,
    pub config: serde_json::Value,
    /// (channel, sensor) for every configured channel.
    pub channels: Vec<(String, String)>,
}

impl Options {
    pub fn from_config(c: &AppConfig) -> Self {
        Self {
            token: c.http.token.clone(),
            cors_origin: c.http.cors_origin.clone(),
            ui: c.http.ui,
            config: c.redacted(),
            channels: c
                .sensors
                .iter()
                .filter(|s| s.enabled)
                .flat_map(|s| {
                    channels::channels_for(s)
                        .into_iter()
                        .map(|spec| (spec.channel, s.name.clone()))
                })
                .collect(),
        }
    }
}

pub fn router(shared: Arc<Shared>, opts: Options) -> Router {
    let Options {
        token,
        cors_origin,
        ui,
        config,
        channels,
    } = opts;
    let ctx = Ctx {
        shared,
        token: token.filter(|t| !t.is_empty()).map(Arc::from),
        config: Arc::new(config),
        channels: Arc::new(channels),
    };
    let api = Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/live", get(live))
        .route("/api/v1/channels", get(channel_catalog))
        .route("/api/v1/config", get(config_handler))
        .route("/api/v1/stream", get(stream_events))
        .route_layer(middleware::from_fn_with_state(ctx.clone(), require_token));
    let cors = cors_origin.and_then(|o| HeaderValue::from_str(&o).ok());
    let mut app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .merge(api);
    if ui {
        app = app.merge(super::web::router());
    }
    app.layer(middleware::from_fn(move |req: Request, next: Next| {
        let cors = cors.clone();
        async move {
            let mut res = next.run(req).await;
            let h = res.headers_mut();
            h.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            if let Some(origin) = cors {
                h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
                h.insert(
                    header::ACCESS_CONTROL_ALLOW_HEADERS,
                    HeaderValue::from_static("authorization"),
                );
            }
            res
        }
    }))
    .with_state(ctx)
}

pub async fn run(
    config: Arc<AppConfig>,
    shared: Arc<Shared>,
    cancel: CancellationToken,
) -> Result<()> {
    let addr: std::net::SocketAddr = format!("{}:{}", config.http.bind, config.http.port)
        .parse()
        .context("invalid http.bind/http.port")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding HTTP server to {addr}"))?;
    if config.http.token.is_none() && !addr.ip().is_loopback() {
        tracing::warn!(
            "HTTP API on {addr} has no http.token; /api is readable by anyone on the network"
        );
    }
    info!("HTTP API listening on http://{addr}");
    if config.http.ui {
        info!("Web dashboard on http://{addr}/");
    }
    axum::serve(listener, router(shared, Options::from_config(&config)))
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await?;
    Ok(())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn require_token(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Response {
    if let Some(expected) = &ctx.token {
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if !ct_eq(presented.as_bytes(), expected.as_bytes()) {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                Json(json!({"error": "unauthorized"})),
            )
                .into_response();
        }
    }
    next.run(req).await
}

async fn health(State(ctx): State<Ctx>) -> Response {
    let h = ctx.shared.health();
    let code = if h.is_ok() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(h)).into_response()
}

fn counters(shared: &Shared) -> serde_json::Value {
    let c = &shared.counters;
    json!({
        "readings": Counters::get(&c.readings),
        "read_errors": Counters::get(&c.read_errors),
        "init_errors": Counters::get(&c.init_errors),
        "reconnects": Counters::get(&c.reconnects),
        "mqtt_published": Counters::get(&c.mqtt_published),
        "mqtt_dropped": Counters::get(&c.mqtt_dropped),
    })
}

fn sensors_json(shared: &Shared) -> Vec<serde_json::Value> {
    let now = Instant::now();
    let s = shared.read();
    s.sensor_names()
        .iter()
        .filter_map(|n| s.sensor_statuses.get(n))
        .map(|st| {
            let mut v = serde_json::to_value(st).unwrap_or_default();
            v["ageMs"] = json!(
                st.last_read_at
                    .map(|t| now.saturating_duration_since(t).as_millis() as u64)
            );
            v
        })
        .collect()
}

async fn status(State(ctx): State<Ctx>) -> Response {
    Json(json!({
        "info": ctx.shared.info,
        "health": ctx.shared.health(),
        "sensors": sensors_json(&ctx.shared),
        "counters": counters(&ctx.shared),
    }))
    .into_response()
}

/// Everything a dashboard needs for one frame.
pub fn live_document(shared: &Shared) -> serde_json::Value {
    let now = Instant::now();
    let sensors = sensors_json(shared);
    let s = shared.read();
    let channels: serde_json::Map<String, serde_json::Value> = s
        .channels
        .iter()
        .map(|(k, c)| {
            (
                k.clone(),
                json!({"value": c.value, "ageMs": now.saturating_duration_since(c.at).as_millis() as u64, "sensor": c.sensor}),
            )
        })
        .collect();
    let fields: serde_json::Map<String, serde_json::Value> = s
        .sensor_data
        .iter()
        .map(|(name, d)| {
            let f: std::collections::BTreeMap<&String, &f64> =
                d.fields.iter().filter(|(_, v)| v.is_finite()).collect();
            (name.clone(), json!(f))
        })
        .collect();
    json!({
        "sequence": s.last_sequence,
        "sampleAgeS": s.sample_age_s(now),
        "sampleRateHz": s.sample_rate,
        "mqtt": s.mqtt_status,
        "channels": channels,
        "sensors": sensors,
        "fields": fields,
    })
}

async fn live(State(ctx): State<Ctx>) -> Response {
    Json(live_document(&ctx.shared)).into_response()
}

async fn channel_catalog(State(ctx): State<Ctx>) -> Response {
    let list: Vec<serde_json::Value> = ctx
        .channels
        .iter()
        .map(|(name, sensor)| {
            let i = channels::info(name);
            json!({"name": name, "sensor": sensor, "label": if i.label.is_empty() { name.as_str() } else { i.label },
                   "unit": i.unit, "group": i.group, "min": i.min, "max": i.max})
        })
        .collect();
    Json(list).into_response()
}

async fn config_handler(State(ctx): State<Ctx>) -> Response {
    Json(ctx.config.as_ref().clone()).into_response()
}

fn sse_payload(stamped: &Stamped) -> serde_json::Value {
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
        "sequence": stamped.sequence,
        "timestampMs": stamped.captured_ms,
        "sensor": stamped.sensor,
        "driver": stamped.driver,
        "channels": channels,
        "fields": fields,
    })
}

async fn stream_events(
    State(ctx): State<Ctx>,
) -> sse::Sse<impl Stream<Item = std::result::Result<sse::Event, Infallible>>> {
    let rx = ctx.shared.bus.subscribe();
    let events = stream::unfold(
        (rx, HashMap::<String, Instant>::new()),
        |(mut rx, mut last)| async move {
            loop {
                match rx.recv().await {
                    Ok(stamped) => {
                        // each sensor thinned on its own, so a 50 Hz IMU does not
                        // flood a browser nor hide a slow sensor
                        let now = Instant::now();
                        if last
                            .get(&stamped.sensor)
                            .is_some_and(|t| now.duration_since(*t) < SSE_MIN_INTERVAL)
                        {
                            continue;
                        }
                        last.insert(stamped.sensor.clone(), now);
                        if let Ok(e) = sse::Event::default()
                            .event("sample")
                            .json_data(sse_payload(&stamped))
                        {
                            return Some((Ok(e), (rx, last)));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );
    sse::Sse::new(events).keep_alive(sse::KeepAlive::new().interval(Duration::from_secs(15)))
}

// ── Prometheus ───────────────────────────────────────────────────────────

fn metric(out: &mut String, name: &str, kind: &str, help: &str, value: impl std::fmt::Display) {
    let _ = writeln!(
        out,
        "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}"
    );
}

/// Prometheus label value escaping (backslash, quote, newline).
fn label(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

pub fn render_metrics(shared: &Shared) -> String {
    let c = &shared.counters;
    let now = Instant::now();
    let s = shared.read();
    let mut o = String::with_capacity(4096);
    let _ = writeln!(
        o,
        "# HELP sensors_to_mqtt_build_info Build information\n# TYPE sensors_to_mqtt_build_info gauge\nsensors_to_mqtt_build_info{{version=\"{}\"}} 1",
        shared.info.version
    );
    let _ = writeln!(
        o,
        "# HELP sensor_connected 1 if the sensor is delivering\n# TYPE sensor_connected gauge"
    );
    for n in s.sensor_names() {
        if let Some(st) = s.sensor_statuses.get(&n).filter(|st| st.enabled) {
            let _ = writeln!(
                o,
                "sensor_connected{{sensor=\"{}\",driver=\"{}\"}} {}",
                label(&n),
                label(&st.driver),
                i32::from(st.connected)
            );
        }
    }
    let _ = writeln!(
        o,
        "# HELP sensor_read_rate_hz Readings per second\n# TYPE sensor_read_rate_hz gauge"
    );
    for n in s.sensor_names() {
        if let Some(st) = s.sensor_statuses.get(&n).filter(|st| st.enabled) {
            let _ = writeln!(
                o,
                "sensor_read_rate_hz{{sensor=\"{}\"}} {}",
                label(&n),
                st.rate_hz
            );
        }
    }
    metric(
        &mut o,
        "sensor_readings_total",
        "counter",
        "Sensor readings",
        Counters::get(&c.readings),
    );
    metric(
        &mut o,
        "sensor_read_errors_total",
        "counter",
        "Failed sensor reads",
        Counters::get(&c.read_errors),
    );
    metric(
        &mut o,
        "sensor_init_errors_total",
        "counter",
        "Failed sensor driver opens",
        Counters::get(&c.init_errors),
    );
    metric(
        &mut o,
        "sensor_reconnects_total",
        "counter",
        "Sensors lost and reopened",
        Counters::get(&c.reconnects),
    );
    metric(
        &mut o,
        "sensor_sample_age_seconds",
        "gauge",
        "Seconds since the last reading",
        s.sample_age_s(now).unwrap_or(f64::NAN),
    );
    let _ = writeln!(
        o,
        "# HELP sensor_channel Latest fresh value of each canonical channel\n# TYPE sensor_channel gauge"
    );
    for (name, ch) in s.fresh_channels(now) {
        let _ = writeln!(
            o,
            "sensor_channel{{name=\"{name}\",sensor=\"{}\"}} {}",
            label(&ch.sensor),
            ch.value
        );
    }
    metric(
        &mut o,
        "mqtt_connected",
        "gauge",
        "1 if the MQTT broker connection is up",
        i32::from(s.mqtt_status.is_connected()),
    );
    metric(
        &mut o,
        "mqtt_messages_published_total",
        "counter",
        "MQTT messages handed to the client",
        Counters::get(&c.mqtt_published),
    );
    metric(
        &mut o,
        "mqtt_messages_dropped_total",
        "counter",
        "Live MQTT messages dropped",
        Counters::get(&c.mqtt_dropped),
    );
    o
}

async fn metrics(State(ctx): State<Ctx>) -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        render_metrics(&ctx.shared),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChannelState, Info, SensorStatus};
    use axum::body::Body;
    use tower::ServiceExt;

    fn opts(token: Option<&str>) -> Options {
        Options {
            token: token.map(String::from),
            cors_origin: None,
            ui: true,
            config: AppConfig::default().redacted(),
            channels: vec![
                ("lateralG".into(), "imu".into()),
                ("imuCh0".into(), "imu".into()),
            ],
        }
    }

    fn shared_live() -> Arc<Shared> {
        let shared = Shared::new(
            Info {
                version: "test",
                ..Info::default()
            },
            false,
        );
        let mut s = shared.write();
        let mut st = SensorStatus::new("imu", "mpu6500", "I2C /dev/i2c-1 @ 0x68".into(), true);
        st.connected = true;
        st.last_read_at = Some(Instant::now());
        s.sensor_statuses.insert("imu".into(), st);
        s.last_sample_at = Some(Instant::now());
        s.channels.insert(
            "lateralG".into(),
            ChannelState {
                value: 0.25,
                captured_ms: 1,
                sensor: "imu".into(),
                at: Instant::now(),
            },
        );
        s.sensor_data.insert(
            "imu".into(),
            crate::sensors::SensorData {
                timestamp: chrono::Utc::now(),
                fields: [("accel_raw_x".into(), 0.25), ("nan".into(), f64::NAN)].into(),
            },
        );
        drop(s);
        shared
    }

    async fn get(app: &Router, path: &str, token: Option<&str>) -> (StatusCode, String) {
        let mut req = Request::builder().uri(path);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn health_metrics_open_api_needs_token() {
        let app = router(shared_live(), opts(Some("t")));
        let (code, body) = get(&app, "/health", None).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        let (_, body) = get(&app, "/metrics", None).await;
        assert!(
            body.contains("sensor_connected{sensor=\"imu\",driver=\"mpu6500\"} 1"),
            "{body}"
        );
        assert!(
            body.contains("sensor_channel{name=\"lateralG\",sensor=\"imu\"} 0.25"),
            "{body}"
        );
        assert_eq!(
            get(&app, "/api/v1/live", None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            get(&app, "/api/v1/live", Some("x")).await.0,
            StatusCode::UNAUTHORIZED
        );
        let (code, body) = get(&app, "/api/v1/live", Some("t")).await;
        assert_eq!(code, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["channels"]["lateralG"]["value"], 0.25);
        assert_eq!(v["channels"]["lateralG"]["sensor"], "imu");
        assert_eq!(v["fields"]["imu"]["accel_raw_x"], 0.25);
        assert!(v["fields"]["imu"].get("nan").is_none());
        assert_eq!(v["sensors"][0]["name"], "imu");
        let (_, body) = get(&app, "/api/v1/status", Some("t")).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["health"]["status"], "ok");
        assert_eq!(v["sensors"][0]["connected"], true);
    }

    #[tokio::test]
    async fn catalog_config_and_unhealthy_state() {
        let shared = Shared::new(Info::default(), false);
        let app = router(shared, opts(None));
        assert_eq!(
            get(&app, "/health", None).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let (_, body) = get(&app, "/api/v1/channels", None).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v[0]["name"], "lateralG");
        assert_eq!(v[0]["unit"], "g");
        assert_eq!(
            v[1]["label"], "imuCh0",
            "sensor-named channels are labelled by name"
        );
        let (_, body) = get(&app, "/api/v1/config", None).await;
        assert!(body.contains("\"base_topic\":\"/telemetry/sensors\""));
    }

    #[test]
    fn constant_time_compare_and_label_escaping() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert_eq!(label("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }
}
