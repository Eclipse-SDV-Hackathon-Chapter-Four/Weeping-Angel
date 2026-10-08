// Copyright (c) 2026 Matthias Knöfel
// Copyright (c) 2026 Alwin Berger
//
// This program and the accompanying materials are made available under
// the terms of the Eclipse Public License 2.0 which accompanies this
// distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
//
// AI Disclosure: This file was mostly AI-generated.
//
// SPDX-License-Identifier: EPL-2.0 and CC0-1.0
// Assisted-by: Claude Opus 5.5, GLM-5.3-flash
//! Guardian → uProtocol fault-event publishing (ADR-007).
//!
//! Mirrors every fault-level change that is reported to the DFM as a JSON
//! `GuardianFaultEvent` on the uProtocol topic [`fault_event_uri`]. Only
//! transitions are published (plus the startup all-clear baseline); there is
//! no periodic re-publication. The channel is independent of the DFM: a
//! dedicated task drains an unbounded queue, so neither a missing DFM nor a
//! failing Zenoh send ever blocks the Guardian cycle or the other channel.

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::mpsc::{self, UnboundedSender};
use tracing::{debug, error, info, warn};
use up_rust::{UMessageBuilder, UPayloadFormat, UTransport, UUri};

use crate::guardian_faults::{
    FaultEvent, FaultEvidence, FaultSource, FaultStage, DFM_MAPPINGS, GUARDIAN_SOURCE,
};
use crate::guardian_model::{Detection, Signal};

/// uEntity of the Guardian (must match the transport's local URI provider).
pub const GUARDIAN_AUTHORITY: &str = "guardian";
pub const GUARDIAN_UE_ID: u32 = 0x1001;
pub const GUARDIAN_UE_VERSION: u8 = 0x01;
/// Resource id of the `GuardianFaultEvent` topic.
pub const RID_GUARDIAN_FAULT_EVENT: u16 = 0x8001;

/// Topic `//guardian/1001/1/8001` carrying `GuardianFaultEvent` JSON.
pub fn fault_event_uri() -> UUri {
    UUri::try_from_parts(
        GUARDIAN_AUTHORITY,
        GUARDIAN_UE_ID,
        GUARDIAN_UE_VERSION,
        RID_GUARDIAN_FAULT_EVENT,
    )
    .expect("static GuardianFaultEvent URI is valid")
}

/// JSON payload of a Guardian fault event; the field set is documented in
/// `product/interfaces/battery_fault_contract.yaml` (`guardian_fault_event`).
#[derive(Debug, Serialize)]
pub struct GuardianFaultEvent<'a> {
    pub fault_id: &'static str,
    pub detection_class: &'static str,
    pub level: &'static str,
    pub stage: FaultStage,
    pub baseline: bool,
    pub sovd_path: &'a str,
    pub source: FaultSource,
    pub evidence: &'a FaultEvidence,
}

/// Queued item: a fault change or the startup baseline of one fault.
struct Outgoing {
    event: FaultEvent,
    baseline: bool,
}

/// Cheap, cloneable handle used by the Guardian cycle to publish fault events.
#[derive(Clone)]
pub struct FaultEventPublisherHandle {
    tx: UnboundedSender<Outgoing>,
}

impl FaultEventPublisherHandle {
    /// Queue a fault-level change for publication; never blocks.
    pub fn publish(&self, event: &FaultEvent) {
        self.enqueue(event.clone(), false);
    }

    fn enqueue(&self, event: FaultEvent, baseline: bool) {
        if self.tx.send(Outgoing { event, baseline }).is_err() {
            warn!("uProtocol fault publisher gone; dropping fault event");
        }
    }
}

/// Spawn the publishing task and queue the all-clear baseline for every fault.
pub fn spawn(transport: Arc<dyn UTransport>, sovd_path: String) -> FaultEventPublisherHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<Outgoing>();
    let handle = FaultEventPublisherHandle { tx };
    for (diagnostic, fault_id) in DFM_MAPPINGS {
        handle.enqueue(
            FaultEvent {
                fault_id,
                diagnostic,
                stage: FaultStage::Passed,
                evidence: FaultEvidence::default(),
            },
            true,
        );
    }

    let topic = fault_event_uri();
    info!(uri = %topic.to_uri(false), "publishing Guardian fault events");
    tokio::spawn(async move {
        while let Some(outgoing) = rx.recv().await {
            let key = outgoing.event.fault_id;
            let payload = encode(&outgoing.event, outgoing.baseline, &sovd_path);
            let message = match UMessageBuilder::publish(topic.clone())
                .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_JSON)
            {
                Ok(message) => message,
                Err(error) => {
                    error!(key, %error, "build GuardianFaultEvent message failed");
                    continue;
                }
            };
            match transport.send(message).await {
                Ok(()) => debug!(key, stage = ?outgoing.event.stage, "fault event -> uProtocol"),
                Err(status) => error!(key, ?status, "publish to uProtocol failed"),
            }
        }
    });
    handle
}

/// Serialize a fault event as `GuardianFaultEvent` JSON.
fn encode(event: &FaultEvent, baseline: bool, sovd_path: &str) -> Vec<u8> {
    serde_json::to_vec(&GuardianFaultEvent {
        fault_id: event.fault_id,
        detection_class: event.diagnostic.class.as_str(),
        level: event.diagnostic.level.as_str(),
        stage: event.stage,
        baseline,
        sovd_path,
        source: GUARDIAN_SOURCE,
        evidence: &event.evidence,
    })
    .expect("GuardianFaultEvent is always serializable")
}

/// Authority and uEntity of the raw `GuardianEvidenceEvent` topic
/// `//guardian-vss/9000/1/9003` (ADR-007). Guardian-vss is a shared event
/// entity, not the Guardian's transport entity: the topic is a Zenoh resource
/// the raw decision stream publishes on.
pub const EVIDENCE_AUTHORITY: &str = "guardian-vss";
pub const EVIDENCE_UE_ID: u32 = 0x9000;
/// Resource id of the `GuardianEvidenceEvent` topic.
pub const RID_GUARDIAN_EVIDENCE_EVENT: u16 = 0x9003;

/// Topic `//guardian-vss/9000/1/9003` carrying `GuardianEvidenceEvent` JSON.
pub fn evidence_event_uri() -> UUri {
    UUri::try_from_parts(
        EVIDENCE_AUTHORITY,
        EVIDENCE_UE_ID,
        0x01,
        RID_GUARDIAN_EVIDENCE_EVENT,
    )
    .expect("static GuardianEvidenceEvent URI is valid")
}

/// JSON payload of a raw evidence event; the field set is documented in
/// `product/interfaces/battery_fault_contract.yaml`
/// (`guardian_evidence_event`). Every detection transition is published,
/// mapped or not.
#[derive(Debug, Serialize)]
pub struct GuardianEvidenceEvent<'a> {
    /// Campaign run id (Guardian startup configuration).
    pub run_id: &'a str,
    pub detection_class: &'a str,
    pub level: &'a str,
    /// `"active"` or `"cleared"` (detection state, not DFM fault level).
    pub stage: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<RawEvidence>,
    /// Source timestamp of the causing sample (ADR-017): omitted when there
    /// is none (e.g. the synthetic STREAM_STALE transition).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_ms: Option<u64>,
}

/// Measured values of the transition; each field is omitted when absent.
#[derive(Debug, Serialize)]
pub struct RawEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub residual: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utilization: Option<f32>,
    /// Source/generation interval Δτ of a temporal check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
}

impl RawEvidence {
    fn of(detection: &Detection) -> Option<Self> {
        let values = Self {
            observed: detection.observed,
            limit: detection.limit,
            residual: detection.residual,
            utilization: detection.utilization,
            interval_ms: detection.interval_ms,
        };
        (values.observed.is_some()
            || values.limit.is_some()
            || values.residual.is_some()
            || values.utilization.is_some()
            || values.interval_ms.is_some())
        .then_some(values)
    }
}

/// Queued raw evidence item.
struct OutgoingEvidence {
    event: Detection,
}

/// Cheap, cloneable handle used by the Guardian cycle to publish raw
/// evidence events; never blocks.
#[derive(Clone)]
pub struct EvidenceEventPublisherHandle {
    tx: UnboundedSender<OutgoingEvidence>,
}

impl EvidenceEventPublisherHandle {
    /// Queue a detection transition for publication on the raw stream.
    pub fn publish(&self, event: &Detection) {
        if self
            .tx
            .send(OutgoingEvidence {
                event: event.clone(),
            })
            .is_err()
        {
            warn!("uProtocol evidence publisher gone; dropping evidence event");
        }
    }
}

/// Spawn the raw evidence publishing task. `run_id` is the campaign run id
/// from the Guardian's startup configuration (ADR-007).
pub fn spawn_evidence(
    transport: Arc<dyn UTransport>,
    run_id: String,
) -> EvidenceEventPublisherHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<OutgoingEvidence>();
    let topic = evidence_event_uri();
    info!(uri = %topic.to_uri(false), "publishing raw Guardian evidence events");
    tokio::spawn(async move {
        while let Some(outgoing) = rx.recv().await {
            let payload = encode_evidence(&outgoing.event, &run_id);
            let message = match UMessageBuilder::publish(topic.clone())
                .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_JSON)
            {
                Ok(message) => message,
                Err(error) => {
                    error!(class = outgoing.event.class.as_str(), %error, "build GuardianEvidenceEvent message failed");
                    continue;
                }
            };
            match transport.send(message).await {
                Ok(()) => {
                    debug!(
                        class = outgoing.event.class.as_str(),
                        stage = if outgoing.event.active {
                            "active"
                        } else {
                            "cleared"
                        },
                        "evidence event -> uProtocol"
                    )
                }
                Err(status) => error!(?status, "raw evidence publish to uProtocol failed"),
            }
        }
    });
    EvidenceEventPublisherHandle { tx }
}

/// Serialize a detection transition as raw `GuardianEvidenceEvent` JSON.
fn encode_evidence(event: &Detection, run_id: &str) -> Vec<u8> {
    serde_json::to_vec(&GuardianEvidenceEvent {
        run_id,
        detection_class: event.class.as_str(),
        level: event.level.as_str(),
        stage: if event.active { "active" } else { "cleared" },
        signal: event.signal.map(Signal::as_str),
        evidence: RawEvidence::of(event),
        timestamp_ms: event.sample_timestamp_ms,
    })
    .expect("GuardianEvidenceEvent is always serializable")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::guardian_faults::{DiagnosticKey, FaultEvidence};
    use crate::guardian_model::{DetectionClass, DetectionLevel};
    use std::time::Instant;

    fn transition(active: bool, timestamp_ms: Option<u64>) -> Detection {
        let signal = active.then_some(Signal::TempMin);
        Detection {
            class: DetectionClass::PhysicalTempRate,
            level: DetectionLevel::Warning,
            signal,
            observed: active.then_some(3.5),
            limit: active.then_some(2.0),
            residual: active.then_some(1.0),
            utilization: None,
            interval_ms: active.then_some(100),
            sample_timestamp_ms: timestamp_ms,
            detected_at: Instant::now(),
            active,
        }
    }

    #[test]
    fn evidence_event_json_shape() {
        let value: serde_json::Value =
            serde_json::from_slice(&encode_evidence(&transition(true, Some(1_200)), "run-7"))
                .unwrap();
        assert_eq!(
            value,
            json!({
                "run_id": "run-7",
                "detection_class": "PHYSICAL_TEMP_RATE",
                "level": "WARNING",
                "stage": "active",
                "signal": "temp_min",
                "evidence": {"observed": 3.5, "limit": 2.0, "residual": 1.0, "interval_ms": 100},
                "timestamp_ms": 1200
            })
        );
    }

    #[test]
    fn evidence_event_omits_absent_optional_fields() {
        // A cleared transition without a causing sample (e.g. STREAM_STALE,
        // ADR-017): no signal, no evidence, no timestamp.
        let value: serde_json::Value =
            serde_json::from_slice(&encode_evidence(&transition(false, None), "run-7")).unwrap();
        let object = value.as_object().unwrap();
        assert!(!object.contains_key("timestamp_ms"));
        assert!(!object.contains_key("signal"));
        assert!(!object.contains_key("evidence"));
        assert_eq!(object.len(), 4);
        assert_eq!(value["stage"], "cleared");
    }

    #[test]
    fn failed_event_json_shape() {
        let event = FaultEvent {
            fault_id: "BatteryTempRate",
            diagnostic: DiagnosticKey {
                class: DetectionClass::PhysicalTempRate,
                level: DetectionLevel::Violation,
            },
            stage: FaultStage::Failed,
            evidence: FaultEvidence {
                signal: Some("temp_max"),
                observed: Some(3.5),
                limit: Some(2.0),
                residual: Some(1.5),
                utilization: Some(1.75),
                interval_ms: Some(100),
                timestamp_ms: Some(1_200),
            },
        };
        let value: serde_json::Value =
            serde_json::from_slice(&encode(&event, false, "battery_guardian")).unwrap();
        assert_eq!(
            value,
            json!({
                "fault_id": "BatteryTempRate",
                "detection_class": "PHYSICAL_TEMP_RATE",
                "level": "VIOLATION",
                "stage": "Failed",
                "baseline": false,
                "sovd_path": "battery_guardian",
                "source": {
                    "entity": "BatteryThermalGuardian",
                    "ecu": "HPC",
                    "domain": "Powertrain",
                    "sw_component": "Guardian",
                    "instance": "0"
                },
                "evidence": {
                    "signal": "temp_max",
                    "observed": 3.5,
                    "limit": 2.0,
                    "residual": 1.5,
                    "utilization": 1.75,
                    "interval_ms": 100,
                    "timestamp_ms": 1200
                }
            })
        );
    }

    #[test]
    fn baseline_event_has_empty_evidence() {
        let event = FaultEvent {
            fault_id: "BatteryTempStreamStale",
            diagnostic: DiagnosticKey {
                class: DetectionClass::StreamStale,
                level: DetectionLevel::Violation,
            },
            stage: FaultStage::Passed,
            evidence: FaultEvidence::default(),
        };
        let value: serde_json::Value =
            serde_json::from_slice(&encode(&event, true, "battery_guardian")).unwrap();
        assert_eq!(value["stage"], "Passed");
        assert_eq!(value["baseline"], true);
        assert_eq!(value["evidence"], json!({}));
    }

    #[test]
    fn topic_is_a_publish_resource_of_the_guardian_entity() {
        let uri = fault_event_uri();
        assert_eq!(uri.authority_name, GUARDIAN_AUTHORITY);
        assert!(uri.resource_id >= 0x8000 && uri.resource_id < 0xFFFF);
    }
}
