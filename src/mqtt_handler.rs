//! Async MQTT handler using rumqttc.
//!
//! Runs the rumqttc event loop in a background Tokio task.
//! Publishes are done via a bounded mpsc channel so callers never block.

use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions, QoS};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::config::MqttConfig;
use crate::models::MqttStatus;

// ---------------------------------------------------------------------------
// Publish message
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PublishMsg {
    pub topic: String,
    pub payload: String,
}

// ---------------------------------------------------------------------------
// Handle returned to callers
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct MqttHandle {
    tx: mpsc::Sender<PublishMsg>,
    pub counter: Arc<AtomicU64>,
    pub status: Arc<RwLock<MqttStatus>>,
}

impl MqttHandle {
    /// Queue a publish.  Returns immediately; drops message if channel is full.
    pub async fn publish(&self, topic: impl Into<String>, payload: impl Into<String>) {
        let msg = PublishMsg {
            topic: topic.into(),
            payload: payload.into(),
        };
        if self.tx.try_send(msg).is_ok() {
            self.counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub async fn is_connected(&self) -> bool {
        self.status.read().await.is_connected()
    }
}

// ---------------------------------------------------------------------------
// Start the MQTT event-loop task.  Returns a handle usable from any task.
// ---------------------------------------------------------------------------

pub fn spawn_mqtt_task(cfg: &MqttConfig) -> MqttHandle {
    let (tx, rx) = mpsc::channel::<PublishMsg>(1000);
    let counter = Arc::new(AtomicU64::new(0));
    let status = Arc::new(RwLock::new(MqttStatus::Connecting));

    let handle = MqttHandle {
        tx,
        counter: Arc::clone(&counter),
        status: Arc::clone(&status),
    };

    let mut opts = MqttOptions::new(cfg.client_id.clone(), cfg.host.clone(), cfg.port);
    opts.set_keep_alive(std::time::Duration::from_secs(20));
    opts.set_clean_session(true);

    if let (Some(u), Some(p)) = (cfg.username.clone(), cfg.password.clone()) {
        opts.set_credentials(u, p);
    }

    let qos = QoS::AtLeastOnce;

    let (client, event_loop) = AsyncClient::new(opts, 100);

    tokio::spawn(run_event_loop(event_loop, status.clone()));
    tokio::spawn(run_publish_loop(client, rx, qos));

    handle
}

async fn run_event_loop(mut evl: EventLoop, status: Arc<RwLock<MqttStatus>>) {
    loop {
        match evl.poll().await {
            Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                info!("MQTT connected");
                *status.write().await = MqttStatus::Connected;
            }
            Ok(Event::Incoming(Incoming::Disconnect)) => {
                warn!("MQTT disconnected");
                *status.write().await = MqttStatus::Disconnected;
            }
            Err(e) => {
                error!("MQTT error: {}", e);
                *status.write().await = MqttStatus::Error(e.to_string());
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            _ => {}
        }
    }
}

async fn run_publish_loop(client: AsyncClient, mut rx: mpsc::Receiver<PublishMsg>, qos: QoS) {
    while let Some(msg) = rx.recv().await {
        if let Err(e) = client
            .publish(&msg.topic, qos, false, msg.payload.as_bytes())
            .await
        {
            warn!("MQTT publish error on {}: {}", msg.topic, e);
        }
    }
}
