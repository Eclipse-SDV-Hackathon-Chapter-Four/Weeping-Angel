//! Detection → fault-level aggregation shared by all reporting channels.
//!
//! Faults are a configured projection of detection class and level. A mapped
//! class/level pair can be active for several signals at once; its fault is
//! `Failed` while at least one signal is active and `Passed` once the last one
//! cleared. Unmapped detections remain internal Guardian observations.
//! [`FaultAggregator`] turns detection transitions into fault-level
//! [`FaultEvent`]s, which are fanned out independently to the DFM and the
//! uProtocol publisher (ADR-007).

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::guardian_model::{Detection, DetectionClass, DetectionLevel, Signal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiagnosticKey {
    pub class: DetectionClass,
    pub level: DetectionLevel,
}

/// The complete configured Guardian-to-DFM projection.
pub const DFM_MAPPINGS: [(DiagnosticKey, &str); 11] = [
    (
        DiagnosticKey {
            class: DetectionClass::StreamStale,
            level: DetectionLevel::Violation,
        },
        "BatteryTempStreamStale",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::ThermalLimit,
            level: DetectionLevel::Warning,
        },
        "BatteryOverTempWarning",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::ThermalLimit,
            level: DetectionLevel::Critical,
        },
        "BatteryOverTempCritical",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::PhysicalTempAbsoluteLimit,
            level: DetectionLevel::Violation,
        },
        "BatteryTempAbsoluteLimit",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::PhysicalTempOrdering,
            level: DetectionLevel::Violation,
        },
        "BatteryTempOrdering",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::PhysicalTempSpread,
            level: DetectionLevel::Violation,
        },
        "BatteryTempSpread",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::PhysicalTempHotspot,
            level: DetectionLevel::Violation,
        },
        "BatteryTempHotspot",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::PhysicalTempRate,
            level: DetectionLevel::Violation,
        },
        "BatteryTempRate",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::PhysicalSocRange,
            level: DetectionLevel::Violation,
        },
        "BatterySocRange",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::PhysicalSocRate,
            level: DetectionLevel::Violation,
        },
        "BatterySocRate",
    ),
    (
        DiagnosticKey {
            class: DetectionClass::SignalStuck,
            level: DetectionLevel::Violation,
        },
        "BatterySignalStuck",
    ),
];

/// Project a Guardian detection onto a DFM fault key. Keys must match the `Text` fault ids in
/// `product/config/battery_guardian/guardian_diagnostics.json`.
pub fn fault_key(class: DetectionClass, level: DetectionLevel) -> Option<&'static str> {
    DFM_MAPPINGS.iter().find_map(|(diagnostic, key)| {
        (diagnostic.class == class && diagnostic.level == level).then_some(*key)
    })
}

/// Reporting identity of the Guardian, shared by every channel.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FaultSource {
    pub entity: &'static str,
    pub ecu: &'static str,
    pub domain: &'static str,
    pub sw_component: &'static str,
    pub instance: &'static str,
}

pub const GUARDIAN_SOURCE: FaultSource = FaultSource {
    entity: "BatteryThermalGuardian",
    ecu: "HPC",
    domain: "Powertrain",
    sw_component: "Guardian",
    instance: "0",
};

/// Fault lifecycle stage reported on a fault-level change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FaultStage {
    Failed,
    Passed,
}

/// Evidence of the detection transition that caused a fault-level change.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct FaultEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub residual: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utilization: Option<f32>,
}

impl FaultEvidence {
    fn of(detection: &Detection) -> Self {
        Self {
            signal: detection.signal.map(Signal::as_str),
            observed: detection.observed,
            limit: detection.limit,
            residual: detection.residual,
            utilization: detection.utilization,
        }
    }
}

/// A fault-level change: what every reporting channel publishes.
#[derive(Debug, Clone, PartialEq)]
pub struct FaultEvent {
    pub fault_id: &'static str,
    pub diagnostic: DiagnosticKey,
    pub stage: FaultStage,
    pub evidence: FaultEvidence,
}

impl FaultEvent {
    /// Evidence as key/value strings for DFM env data.
    pub fn env_pairs(&self) -> Vec<(String, String)> {
        let mut env = Vec::new();
        if let Some(signal) = self.evidence.signal {
            env.push(("signal".to_string(), signal.to_string()));
        }
        env.push((
            "level".to_string(),
            self.diagnostic.level.as_str().to_string(),
        ));
        for (name, value) in [
            ("observed", self.evidence.observed),
            ("limit", self.evidence.limit),
            ("residual", self.evidence.residual),
            ("utilization", self.evidence.utilization),
        ] {
            if let Some(value) = value {
                env.push((name.to_string(), format!("{value:.3}")));
            }
        }
        env
    }
}

/// Tracks which signals are active per mapped class/level pair and yields
/// fault-level changes.
#[derive(Debug, Default)]
pub struct FaultAggregator {
    active_signals: HashMap<DiagnosticKey, HashSet<Option<Signal>>>,
}

impl FaultAggregator {
    /// Apply a detection transition; returns a fault event if the failed
    /// state of its mapped fault changed. Unmapped detections yield nothing.
    pub fn apply(&mut self, detection: &Detection) -> Option<FaultEvent> {
        let fault_id = fault_key(detection.class, detection.level)?;
        let diagnostic = DiagnosticKey {
            class: detection.class,
            level: detection.level,
        };
        let signals = self.active_signals.entry(diagnostic).or_default();
        let was_failed = !signals.is_empty();
        if detection.active {
            signals.insert(detection.signal);
        } else {
            signals.remove(&detection.signal);
        }
        let is_failed = !signals.is_empty();
        (was_failed != is_failed).then(|| FaultEvent {
            fault_id,
            diagnostic,
            stage: if is_failed {
                FaultStage::Failed
            } else {
                FaultStage::Passed
            },
            evidence: FaultEvidence::of(detection),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use fault_lib::catalog::FaultCatalogBuilder;

    use super::*;

    fn detection(
        class: DetectionClass,
        level: DetectionLevel,
        signal: Option<Signal>,
        active: bool,
    ) -> Detection {
        Detection {
            class,
            level,
            signal,
            observed: active.then_some(3.0),
            limit: active.then_some(2.0),
            residual: active.then_some(1.0),
            utilization: None,
            detected_at: Instant::now(),
            active,
        }
    }

    fn stage_of(event: Option<FaultEvent>) -> Option<FaultStage> {
        event.map(|event| event.stage)
    }

    #[test]
    fn fault_keys_are_unique() {
        let keys: HashSet<_> = DFM_MAPPINGS.iter().map(|(_, key)| *key).collect();
        assert_eq!(keys.len(), DFM_MAPPINGS.len());
    }

    #[test]
    fn catalog_contains_every_fault_key() {
        let contents = include_str!("../../../config/battery_guardian/guardian_diagnostics.json");
        let _catalog = FaultCatalogBuilder::new()
            .json_string(contents)
            .expect("valid DFM catalog schema")
            .build();
        let catalog: serde_json::Value = serde_json::from_str(contents).expect("catalog json");
        let ids: HashSet<&str> = catalog["faults"]
            .as_array()
            .expect("faults array")
            .iter()
            .filter_map(|fault| fault["id"]["Text"].as_str())
            .collect();
        for (_, key) in DFM_MAPPINGS {
            assert!(ids.contains(key), "{key}");
        }
        assert_eq!(ids.len(), DFM_MAPPINGS.len());
    }

    #[test]
    fn fault_stays_failed_until_last_signal_clears() {
        let mut faults = FaultAggregator::default();
        let (class, level) = (DetectionClass::PhysicalTempRate, DetectionLevel::Violation);
        let raised = faults
            .apply(&detection(class, level, Some(Signal::TempMin), true))
            .expect("first signal raises the fault");
        assert_eq!(raised.stage, FaultStage::Failed);
        assert_eq!(raised.fault_id, "BatteryTempRate");
        assert_eq!(raised.evidence.signal, Some("temp_min"));
        assert!(faults
            .apply(&detection(class, level, Some(Signal::TempMax), true))
            .is_none());
        assert!(faults
            .apply(&detection(class, level, Some(Signal::TempMin), false))
            .is_none());
        assert_eq!(
            stage_of(faults.apply(&detection(class, level, Some(Signal::TempMax), false))),
            Some(FaultStage::Passed)
        );
    }

    #[test]
    fn clearing_inactive_fault_is_not_a_change() {
        let mut faults = FaultAggregator::default();
        assert!(faults
            .apply(&detection(
                DetectionClass::SignalStuck,
                DetectionLevel::Violation,
                None,
                false
            ))
            .is_none());
    }

    #[test]
    fn thermal_faults_activate_and_clear_through_existing_state_logic() {
        let mut faults = FaultAggregator::default();
        for (level, key) in [
            (DetectionLevel::Warning, "BatteryOverTempWarning"),
            (DetectionLevel::Critical, "BatteryOverTempCritical"),
        ] {
            let class = DetectionClass::ThermalLimit;
            assert_eq!(fault_key(class, level), Some(key));
            assert_eq!(
                stage_of(faults.apply(&detection(class, level, Some(Signal::TempMax), true))),
                Some(FaultStage::Failed)
            );
            assert_eq!(
                stage_of(faults.apply(&detection(class, level, Some(Signal::TempMax), false))),
                Some(FaultStage::Passed)
            );
        }
    }

    #[test]
    fn utilization_warnings_remain_unmapped() {
        let mut faults = FaultAggregator::default();
        for class in [
            DetectionClass::PhysicalTempSpread,
            DetectionClass::PhysicalTempHotspot,
            DetectionClass::PhysicalTempRate,
        ] {
            assert_eq!(fault_key(class, DetectionLevel::Warning), None);
            assert!(fault_key(class, DetectionLevel::Violation).is_some());
            assert!(faults
                .apply(&detection(class, DetectionLevel::Warning, None, true))
                .is_none());
        }
    }

    #[test]
    fn env_pairs_match_dfm_env_format() {
        let event = FaultEvent {
            fault_id: "BatterySocRate",
            diagnostic: DiagnosticKey {
                class: DetectionClass::PhysicalSocRate,
                level: DetectionLevel::Violation,
            },
            stage: FaultStage::Failed,
            evidence: FaultEvidence {
                signal: Some("soc"),
                observed: Some(1.0),
                limit: None,
                residual: Some(0.25),
                utilization: None,
            },
        };
        assert_eq!(
            event.env_pairs(),
            vec![
                ("signal".to_string(), "soc".to_string()),
                ("level".to_string(), "VIOLATION".to_string()),
                ("observed".to_string(), "1.000".to_string()),
                ("residual".to_string(), "0.250".to_string()),
            ]
        );
    }
}
