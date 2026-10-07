//! Subscriptions over uProtocol/Zenoh: Guardian fault events and battery events.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::sync::mpsc;
use up_rust::{UListener, UMessage, UTransport, UUri};
use up_transport_zenoh::UPTransportZenoh;

use crate::{decode_message, Message};

/// `GuardianFaultEvent` topic (`guardian_fault_event.uri` in the contract).
pub const DEFAULT_FAULT_TOPIC: &str = "//guardian/1001/1/8001";
/// `BatteryTempEvent` topic of the VSS bridge (ADR-007).
pub const DEFAULT_BATTERY_TOPIC: &str = "//battery-vss/9001/1/9001";

/// When to stop listening.
pub struct StopCondition {
    /// Battery timeline position (ms, relative to the first battery event)
    /// at which the replay is complete.
    pub end_ms: u64,
    /// Stop when nothing arrived for this long (only after the first battery
    /// event); also the grace period for late fault events after `end_ms`.
    pub idle_timeout: Duration,
}

/// Forwards every decodable message into one channel, so fault and battery
/// messages keep their arrival order relative to each other.
struct Forwarder {
    tx: mpsc::UnboundedSender<Message>,
}

#[async_trait]
impl UListener for Forwarder {
    async fn on_receive(&self, message: UMessage) {
        let Some(payload) = message.payload.as_deref() else {
            eprintln!("warning: message without payload ignored");
            return;
        };
        match decode_message(payload) {
            Ok(m) => {
                let _ = self.tx.send(m);
            }
            Err(e) => eprintln!("warning: undecodable message ignored: {e}"),
        }
    }
}

async fn open_transport() -> Result<Arc<dyn UTransport>> {
    UPTransportZenoh::try_init_log_from_env();
    let mut config = zenoh::Config::default();
    if let Ok(endpoint) = std::env::var("ZENOH_CONNECT") {
        config
            .insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
            .map_err(|e| anyhow!("Zenoh config: {e}"))?;
    }
    let transport = UPTransportZenoh::builder("evidence-collector")
        .map_err(|e| anyhow!("invalid authority: {e}"))?
        .with_config(config)
        .build()
        .await?;
    Ok(Arc::new(transport))
}

/// Subscribes to both topics and collects messages in arrival order until
/// `stop` is met or Ctrl-C is pressed.
pub async fn collect(topics: &[UUri], stop: &StopCondition) -> Result<Vec<Message>> {
    let transport = open_transport().await?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let listener: Arc<dyn UListener> = Arc::new(Forwarder { tx });
    for topic in topics {
        transport
            .register_listener(topic, None, listener.clone())
            .await
            .map_err(|e| anyhow!("subscribing to {}: {e}", topic.to_uri(true)))?;
        eprintln!("listening on {}", topic.to_uri(true));
    }

    let mut messages = Vec::new();
    let mut origin_ms = None;
    let mut deadline = None;
    loop {
        let next = async {
            match (origin_ms, deadline) {
                (None, _) => rx.recv().await,
                (Some(_), Some(at)) => tokio::time::timeout_at(at, rx.recv()).await.ok().flatten(),
                (Some(_), None) => {
                    tokio::time::timeout(stop.idle_timeout, rx.recv()).await.ok().flatten()
                }
            }
        };
        tokio::select! {
            message = next => match message {
                Some(message) => {
                    if let Message::Battery(b) = &message {
                        let origin = *origin_ms.get_or_insert(b.timestamp_ms);
                        let t = b.timestamp_ms.saturating_sub(origin);
                        if deadline.is_none() && t + crate::REPLAY_END_TOLERANCE_MS >= stop.end_ms {
                            eprintln!("replay end reached at {t} ms, waiting {:?} for late events", stop.idle_timeout);
                            deadline = Some(tokio::time::Instant::now() + stop.idle_timeout);
                        }
                    } else if let Message::Fault(f) = &message {
                        if !f.baseline {
                            eprintln!("fault event: {} {} {:?}", f.detection_class, f.level, f.stage);
                        }
                    }
                    messages.push(message);
                }
                None if deadline.is_some() => break,
                None => {
                    eprintln!("nothing received for {:?}, stopping", stop.idle_timeout);
                    break;
                }
            },
            _ = tokio::signal::ctrl_c() => {
                eprintln!("interrupted, stopping");
                break;
            }
        }
    }

    for topic in topics {
        let _ = transport.unregister_listener(topic, None, listener.clone()).await;
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fault_topic_matches_contract() {
        let contract: serde_yaml::Value =
            serde_yaml::from_str(crate::CONTRACT).expect("contract parses");
        assert_eq!(
            contract["guardian_fault_event"]["uri"].as_str(),
            Some(DEFAULT_FAULT_TOPIC)
        );
    }
}
