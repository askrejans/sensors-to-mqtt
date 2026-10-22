//! Outputs. Each subscribes to the bus independently; a slow or failing output
//! never affects the sensors or the other outputs.

pub mod health;
pub mod http;
pub mod mqtt;
pub mod web;

use crate::config::AppConfig;
use crate::models::Shared;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::error;

pub fn describe(config: &AppConfig) -> Vec<String> {
    let mut out = Vec::new();
    if config.mqtt.enabled {
        let formats: Vec<String> = config
            .mqtt
            .formats
            .iter()
            .map(|f| format!("{f:?}").to_lowercase())
            .collect();
        out.push(format!(
            "mqtt://{} {} [{}]",
            config.mqtt.address(),
            config.mqtt.base_topic,
            formats.join(",")
        ));
    }
    if config.http.enabled {
        out.push(format!("http://{}:{}", config.http.bind, config.http.port));
    }
    out
}

pub fn spawn_all(
    config: &Arc<AppConfig>,
    shared: &Arc<Shared>,
    cancel: &CancellationToken,
    set: &mut JoinSet<()>,
) {
    fn guard(name: &'static str, r: anyhow::Result<()>) {
        if let Err(e) = r {
            error!("{name} output stopped: {e:#}");
        }
    }
    if config.mqtt.enabled {
        let (c, s, k) = (config.clone(), shared.clone(), cancel.clone());
        set.spawn(async move { guard("MQTT", mqtt::run(c, s, k).await) });
    }
    if config.http.enabled {
        let (c, s, k) = (config.clone(), shared.clone(), cancel.clone());
        set.spawn(async move { guard("HTTP", http::run(c, s, k).await) });
    }
}
