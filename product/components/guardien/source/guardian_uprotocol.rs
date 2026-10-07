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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::guardian_faults::DiagnosticKey;
    use crate::guardian_model::{DetectionClass, DetectionLevel};

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
                    "utilization": 1.75
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
