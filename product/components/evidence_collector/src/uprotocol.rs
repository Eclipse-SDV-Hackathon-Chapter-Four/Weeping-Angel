// Copyright (c) 2026 Sebastian Russer
// Copyright (c) 2026 Alwin Berger
//
// This program and the accompanying materials are made available under
// the terms of the Eclipse Public License 2.0 which accompanies this
// distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
//
// AI Disclosure: This file was mostly AI-generated.
//
// SPDX-License-Identifier: EPL-2.0 and CC0-1.0
// Assisted-by: Claude Opus 5.5, DeepSeek v4.1 Flash, GLM-5.3-flash
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
/// Raw `GuardianEvidenceEvent` topic (ADR-007 raw decision stream).
pub const DEFAULT_EVIDENCE_TOPIC: &str = "//guardian-vss/9000/1/9003";
/// `BatteryTempEvent` topic of the VSS bridge (ADR-007).
pub const DEFAULT_BATTERY_TOPIC: &str = "//battery-vss/9001/1/9001";

/// Optional live sink, invoked for every message in arrival order before it is
/// buffered. Used by the live observer (ADR-016).
pub(crate) type Sink = Option<Arc<dyn Fn(&Message) + Send + Sync>>;

/// When to stop listening.
pub struct StopCondition {
    /// Battery source time (ms) at which the replay is complete.
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
            Ok(mut m) => {
                if let Message::Battery(b) = &mut m {
                    b.received = Some(std::time::Instant::now());
                }
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

/// Subscribes to both topics (and polls OpenSOVD at `sovd_url`, if given) and
/// collects messages in arrival order until `stop` is met or Ctrl-C is pressed.
/// `sink`, when set, sees every message live before it is appended.
pub async fn collect(
    topics: &[UUri],
    sovd_url: Option<&str>,
    stop: &StopCondition,
    sink: Sink,
) -> Result<Vec<Message>> {
    let transport = open_transport().await?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let poller = sovd_url.map(|url| {
        eprintln!("polling OpenSOVD at {url}");
        crate::sovd::spawn_poller(url.to_owned(), tx.clone())
    });
    let listener: Arc<dyn UListener> = Arc::new(Forwarder { tx });
    for topic in topics {
        transport
            .register_listener(topic, None, listener.clone())
            .await
            .map_err(|e| anyhow!("subscribing to {}: {e}", topic.to_uri(true)))?;
        eprintln!("listening on {}", topic.to_uri(true));
    }

    let mut messages = Vec::new();
    let mut clock = crate::Clock::default();
    // End of replay seen: stop at this point (grace period for late events).
    let mut end_deadline: Option<tokio::time::Instant> = None;
    // Idle timeout counts only battery/fault messages, not OpenSOVD polls.
    let mut last_activity: Option<tokio::time::Instant> = None;
    loop {
        let deadline = match (last_activity, end_deadline) {
            (None, _) => None,
            (Some(a), end) => {
                let idle = a + stop.idle_timeout;
                Some(end.map_or(idle, |e| e.min(idle)))
            }
        };
        let next = async {
            match deadline {
                None => Some(rx.recv().await),
                Some(at) => tokio::time::timeout_at(at, rx.recv()).await.ok(),
            }
        };
        tokio::select! {
            message = next => match message {
                Some(Some(message)) => {
                    match &message {
                        Message::Battery(b) => {
                            last_activity = Some(tokio::time::Instant::now());
                            let t = clock.battery(b.timestamp_ms);
                            if end_deadline.is_none() && t + crate::REPLAY_END_TOLERANCE_MS >= stop.end_ms {
                                eprintln!("replay end reached at {t} ms, waiting {:?} for late events", stop.idle_timeout);
                                end_deadline = Some(tokio::time::Instant::now() + stop.idle_timeout);
                            }
                        }
                        Message::Fault(f) => {
                            if last_activity.is_some() {
                                last_activity = Some(tokio::time::Instant::now());
                            }
                            if !f.baseline {
                                eprintln!("fault event: {} {} {:?}", f.detection_class, f.level, f.stage);
                            }
                        }
                        Message::Evidence(e) => {
                            if last_activity.is_some() {
                                last_activity = Some(tokio::time::Instant::now());
                            }
                            eprintln!(
                                "evidence event: {} {} {} {}",
                                e.detection_class, e.level, e.stage, e.signal.as_deref().unwrap_or("-")
                            );
                        }
                        Message::Sovd(..) => {}
                    }
                    if let Some(sink) = &sink {
                        sink(&message);
                    }
                    messages.push(message);
                }
                Some(None) => break, // all senders gone
                None if end_deadline.is_some_and(|e| tokio::time::Instant::now() >= e) => break,
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

    if let Some(poller) = poller {
        poller.abort();
    }
    for topic in topics {
        let _ = transport
            .unregister_listener(topic, None, listener.clone())
            .await;
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

    #[test]
    fn evidence_topic_matches_contract() {
        let contract: serde_yaml::Value =
            serde_yaml::from_str(crate::CONTRACT).expect("contract parses");
        assert_eq!(
            contract["guardian_evidence_event"]["uri"].as_str(),
            Some(DEFAULT_EVIDENCE_TOPIC)
        );
    }
}
