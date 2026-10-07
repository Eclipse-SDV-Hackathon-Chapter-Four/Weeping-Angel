//! Guardian → DFM fault reporting.
//!
//! The `fault_lib` [`Reporter`] wraps non-`Send` iceoryx2 IPC ports, so all
//! reporters live on a dedicated OS thread. The async Guardian only holds a
//! cheap, cloneable [`FaultReporterHandle`] and forwards [`Detection`]
//! transitions to it. The worker publishes `Failed`/`Passed` fault records to
//! the Diagnostic Fault Manager.
//!
//! DFM faults are a configured projection of detection class and level. A
//! mapped class/level pair can be active for several signals at once; its fault
//! is `Failed` while at least one signal is active and `Passed` once the last
//! one cleared. Unmapped detections remain internal Guardian observations.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use common::fault::{FaultId, LifecyclePhase, LifecycleStage};
use common::ids::SourceId;
use common::types::{MetadataVec, ShortString};
use fault_lib::catalog::FaultCatalogBuilder;
use fault_lib::reporter::{Reporter, ReporterApi, ReporterConfig};
use fault_lib::utils::to_static_short_string;
use fault_lib::FaultApi;
use tracing::{debug, error, info, warn};

use crate::guardian_model::{Detection, DetectionClass, DetectionLevel, Signal};

/// Maximum number of env-data entries attached to a fault record.
const MAX_ENV_ENTRIES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DiagnosticKey {
    class: DetectionClass,
    level: DetectionLevel,
}

/// The complete configured Guardian-to-DFM projection.
const DFM_MAPPINGS: [(DiagnosticKey, &str); 11] = [
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

/// A detection transition forwarded to the worker thread.
struct FaultCommand {
    class: DetectionClass,
    level: DetectionLevel,
    signal: Option<Signal>,
    active: bool,
    env: Vec<(String, String)>,
}

/// Cheap, cloneable handle used by the async Guardian to report detections.
#[derive(Clone)]
pub struct FaultReporterHandle {
    tx: Sender<FaultCommand>,
    ready: Arc<AtomicBool>,
}

impl FaultReporterHandle {
    /// Forward a detection transition (active or cleared) to the DFM worker.
    pub fn report(&self, detection: &Detection) {
        let command = FaultCommand {
            class: detection.class,
            level: detection.level,
            signal: detection.signal,
            active: detection.active,
            env: evidence(detection),
        };
        if self.tx.send(command).is_err() {
            warn!(
                class = detection.class.as_str(),
                level = detection.level.as_str(),
                "fault reporter worker gone; dropping detection"
            );
        }
    }

    /// True once the DFM connection is established and reporters are live.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}

/// Spawn the fault-reporting worker thread.
///
/// `catalog_path` is the DFM fault catalog JSON (must match the catalog the
/// DFM loaded). `sovd_path` is the path used when publishing, normally the
/// catalog id (`"battery_guardian"`).
pub fn spawn(catalog_path: PathBuf, sovd_path: String) -> FaultReporterHandle {
    let (tx, rx) = mpsc::channel::<FaultCommand>();
    let ready = Arc::new(AtomicBool::new(false));
    let ready_worker = Arc::clone(&ready);

    thread::Builder::new()
        .name("fault-reporter".into())
        .spawn(move || worker(&catalog_path, &sovd_path, &rx, &ready_worker))
        .expect("spawn fault-reporter thread");

    FaultReporterHandle { tx, ready }
}

/// Evidence key/value pairs describing a detection.
fn evidence(detection: &Detection) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(signal) = detection.signal {
        env.push(("signal".to_string(), signal.as_str().to_string()));
    }
    env.push(("level".to_string(), detection.level.as_str().to_string()));
    for (name, value) in [
        ("observed", detection.observed),
        ("limit", detection.limit),
        ("residual", detection.residual),
        ("utilization", detection.utilization),
    ] {
        if let Some(value) = value {
            env.push((name.to_string(), format!("{value:.3}")));
        }
    }
    env
}

fn reporter_config() -> ReporterConfig {
    ReporterConfig {
        source: SourceId {
            entity: to_static_short_string("BatteryThermalGuardian")
                .expect("entity name fits ShortString"),
            ecu: to_static_short_string("HPC").ok(),
            domain: to_static_short_string("Powertrain").ok(),
            sw_component: to_static_short_string("Guardian").ok(),
            instance: to_static_short_string("0").ok(),
        },
        lifecycle_phase: LifecyclePhase::Running,
        default_env_data: MetadataVec::new(),
    }
}

fn env_to_metadata(env: &[(String, String)]) -> MetadataVec {
    let pairs: Vec<(ShortString, ShortString)> = env
        .iter()
        .take(MAX_ENV_ENTRIES)
        .filter_map(|(key, value)| {
            Some((
                to_static_short_string(key).ok()?,
                to_static_short_string(value).ok()?,
            ))
        })
        .collect();
    MetadataVec::try_from(&pairs[..]).unwrap_or_else(|_| MetadataVec::new())
}

fn build_catalog(catalog_path: &Path) -> fault_lib::catalog::FaultCatalog {
    FaultCatalogBuilder::new()
        .json_file(catalog_path.to_path_buf())
        .expect("load fault catalog json")
        .build()
}

/// Tracks which signals are active per mapped class/level pair.
#[derive(Default)]
struct FaultState {
    active_signals: HashMap<DiagnosticKey, HashSet<Option<Signal>>>,
}

impl FaultState {
    /// Apply a transition; returns the new failed state of the class' fault
    /// if it changed.
    fn apply(
        &mut self,
        diagnostic: DiagnosticKey,
        signal: Option<Signal>,
        active: bool,
    ) -> Option<bool> {
        let signals = self.active_signals.entry(diagnostic).or_default();
        let was_failed = !signals.is_empty();
        if active {
            signals.insert(signal);
        } else {
            signals.remove(&signal);
        }
        let is_failed = !signals.is_empty();
        (was_failed != is_failed).then_some(is_failed)
    }
}

fn worker(catalog_path: &Path, sovd_path: &str, rx: &Receiver<FaultCommand>, ready: &AtomicBool) {
    // FaultApi initialisation requires the DFM to be up (IPC sink + catalog
    // hash verification). Retry until it succeeds.
    let mut attempt: u32 = 0;
    let _api = loop {
        attempt += 1;
        match FaultApi::try_new(build_catalog(catalog_path)) {
            Ok(api) => {
                info!("connected to DFM; fault reporting active");
                break api;
            }
            Err(error) => {
                if attempt == 1 || attempt.is_multiple_of(10) {
                    warn!(%error, attempt, "DFM not ready; retrying");
                }
                thread::sleep(Duration::from_millis(500));
            }
        }
    };

    let config = reporter_config();
    let mut reporters: HashMap<DiagnosticKey, Reporter> = HashMap::new();
    for (diagnostic, key) in DFM_MAPPINGS {
        let id = FaultId::Text(to_static_short_string(key).expect("fault key fits ShortString"));
        match Reporter::new(&id, config.clone()) {
            Ok(reporter) => {
                reporters.insert(diagnostic, reporter);
            }
            Err(error) => error!(key, %error, "failed to create fault reporter"),
        }
    }

    // Publish an initial all-clear baseline so the DFM store is populated
    // before the first detection.
    for (diagnostic, reporter) in &mut reporters {
        let record = reporter.create_record(LifecycleStage::Passed);
        if let Err(error) = reporter.publish(sovd_path, record) {
            error!(key = fault_key(diagnostic.class, diagnostic.level), %error, "initial baseline publish failed");
        }
    }
    info!(
        faults = reporters.len(),
        "published initial all-clear baseline"
    );
    ready.store(true, Ordering::Relaxed);

    let mut state = FaultState::default();
    while let Ok(command) = rx.recv() {
        let diagnostic = DiagnosticKey {
            class: command.class,
            level: command.level,
        };
        let Some(key) = fault_key(command.class, command.level) else {
            debug!(
                class = command.class.as_str(),
                level = command.level.as_str(),
                "Guardian detection has no configured DFM projection"
            );
            continue;
        };
        let Some(failed) = state.apply(diagnostic, command.signal, command.active) else {
            continue;
        };
        let Some(reporter) = reporters.get_mut(&diagnostic) else {
            warn!(key, "no reporter for fault");
            continue;
        };

        let stage = if failed {
            LifecycleStage::Failed
        } else {
            LifecycleStage::Passed
        };
        let mut record = reporter.create_record(stage);
        record.env_data = env_to_metadata(&command.env);

        match reporter.publish(sovd_path, record) {
            Ok(()) if failed => warn!(key, "raised fault -> DFM"),
            Ok(()) => info!(key, "cleared fault -> DFM"),
            Err(error) => error!(key, %error, "publish to DFM failed"),
        }
    }
    info!("command channel closed; fault reporter worker exiting");
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut state = FaultState::default();
        let diagnostic = DiagnosticKey {
            class: DetectionClass::PhysicalTempRate,
            level: DetectionLevel::Violation,
        };
        assert_eq!(
            state.apply(diagnostic, Some(Signal::TempMin), true),
            Some(true)
        );
        assert_eq!(state.apply(diagnostic, Some(Signal::TempMax), true), None);
        assert_eq!(state.apply(diagnostic, Some(Signal::TempMin), false), None);
        assert_eq!(
            state.apply(diagnostic, Some(Signal::TempMax), false),
            Some(false)
        );
    }

    #[test]
    fn clearing_inactive_fault_is_not_a_change() {
        let mut state = FaultState::default();
        let diagnostic = DiagnosticKey {
            class: DetectionClass::SignalStuck,
            level: DetectionLevel::Violation,
        };
        assert_eq!(state.apply(diagnostic, None, false), None);
    }

    #[test]
    fn thermal_faults_activate_and_clear_through_existing_state_logic() {
        let mut state = FaultState::default();
        for (level, key) in [
            (DetectionLevel::Warning, "BatteryOverTempWarning"),
            (DetectionLevel::Critical, "BatteryOverTempCritical"),
        ] {
            let diagnostic = DiagnosticKey {
                class: DetectionClass::ThermalLimit,
                level,
            };
            assert_eq!(fault_key(diagnostic.class, diagnostic.level), Some(key));
            assert_eq!(
                state.apply(diagnostic, Some(Signal::TempMax), true),
                Some(true)
            );
            assert_eq!(
                state.apply(diagnostic, Some(Signal::TempMax), false),
                Some(false)
            );
        }
    }

    #[test]
    fn utilization_warnings_remain_unmapped() {
        for class in [
            DetectionClass::PhysicalTempSpread,
            DetectionClass::PhysicalTempHotspot,
            DetectionClass::PhysicalTempRate,
        ] {
            assert_eq!(fault_key(class, DetectionLevel::Warning), None);
            assert!(fault_key(class, DetectionLevel::Violation).is_some());
        }
    }
}
