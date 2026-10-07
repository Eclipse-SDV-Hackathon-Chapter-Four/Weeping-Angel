//! Guardian → DFM fault reporting.
//!
//! The `fault_lib` [`Reporter`] wraps non-`Send` iceoryx2 IPC ports, so all
//! reporters live on a dedicated OS thread. The async Guardian only holds a
//! cheap, cloneable [`FaultReporterHandle`] and forwards [`Detection`]
//! transitions to it. The worker publishes `Failed`/`Passed` fault records to
//! the Diagnostic Fault Manager.
//!
//! One DFM fault exists per [`DetectionClass`]. A class can be active for
//! several signals at once; its fault is `Failed` while at least one signal is
//! active and `Passed` once the last one cleared.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
use tracing::{error, info, warn};

use crate::guardian_model::{Detection, DetectionClass, Signal};

/// Maximum number of env-data entries attached to a fault record.
const MAX_ENV_ENTRIES: usize = 8;

/// All detection classes, i.e. all faults of the Guardian catalog.
const ALL_CLASSES: [DetectionClass; 9] = [
    DetectionClass::StreamStale,
    DetectionClass::PhysicalTempAbsoluteLimit,
    DetectionClass::PhysicalTempOrdering,
    DetectionClass::PhysicalTempSpread,
    DetectionClass::PhysicalTempHotspot,
    DetectionClass::PhysicalTempRate,
    DetectionClass::PhysicalSocRange,
    DetectionClass::PhysicalSocRate,
    DetectionClass::SignalStuck,
];

/// Fault key of a detection class — must match the `Text` fault ids in
/// `product/config/catalog/battery_guardian.json`.
pub const fn fault_key(class: DetectionClass) -> &'static str {
    match class {
        DetectionClass::StreamStale => "BatteryTempStreamStale",
        DetectionClass::PhysicalTempAbsoluteLimit => "BatteryTempAbsoluteLimit",
        DetectionClass::PhysicalTempOrdering => "BatteryTempOrdering",
        DetectionClass::PhysicalTempSpread => "BatteryTempSpread",
        DetectionClass::PhysicalTempHotspot => "BatteryTempHotspot",
        DetectionClass::PhysicalTempRate => "BatteryTempRate",
        DetectionClass::PhysicalSocRange => "BatterySocRange",
        DetectionClass::PhysicalSocRate => "BatterySocRate",
        DetectionClass::SignalStuck => "BatterySignalStuck",
    }
}

/// A detection transition forwarded to the worker thread.
struct FaultCommand {
    class: DetectionClass,
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
            signal: detection.signal,
            active: detection.active,
            env: evidence(detection),
        };
        if self.tx.send(command).is_err() {
            warn!(
                class = detection.class.as_str(),
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
    for (name, value) in [
        ("observed", detection.observed),
        ("limit", detection.limit),
        ("residual", detection.residual),
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

fn build_catalog(catalog_path: &PathBuf) -> fault_lib::catalog::FaultCatalog {
    FaultCatalogBuilder::new()
        .json_file(catalog_path.clone())
        .expect("load fault catalog json")
        .build()
}

/// Tracks which signals are active per class and yields fault-level changes.
#[derive(Default)]
struct FaultState {
    active_signals: HashMap<DetectionClass, HashSet<Option<Signal>>>,
}

impl FaultState {
    /// Apply a transition; returns the new failed state of the class' fault
    /// if it changed.
    fn apply(
        &mut self,
        class: DetectionClass,
        signal: Option<Signal>,
        active: bool,
    ) -> Option<bool> {
        let signals = self.active_signals.entry(class).or_default();
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

fn worker(
    catalog_path: &PathBuf,
    sovd_path: &str,
    rx: &Receiver<FaultCommand>,
    ready: &AtomicBool,
) {
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
                if attempt == 1 || attempt % 10 == 0 {
                    warn!(%error, attempt, "DFM not ready; retrying");
                }
                thread::sleep(Duration::from_millis(500));
            }
        }
    };

    let config = reporter_config();
    let mut reporters: HashMap<DetectionClass, Reporter> = HashMap::new();
    for class in ALL_CLASSES {
        let key = fault_key(class);
        let id = FaultId::Text(to_static_short_string(key).expect("fault key fits ShortString"));
        match Reporter::new(&id, config.clone()) {
            Ok(reporter) => {
                reporters.insert(class, reporter);
            }
            Err(error) => error!(key, %error, "failed to create fault reporter"),
        }
    }

    // Publish an initial all-clear baseline so the DFM store is populated
    // before the first detection.
    for (class, reporter) in &mut reporters {
        let record = reporter.create_record(LifecycleStage::Passed);
        if let Err(error) = reporter.publish(sovd_path, record) {
            error!(key = fault_key(*class), %error, "initial baseline publish failed");
        }
    }
    info!(
        faults = reporters.len(),
        "published initial all-clear baseline"
    );
    ready.store(true, Ordering::Relaxed);

    let mut state = FaultState::default();
    while let Ok(command) = rx.recv() {
        let Some(failed) = state.apply(command.class, command.signal, command.active) else {
            continue;
        };
        let key = fault_key(command.class);
        let Some(reporter) = reporters.get_mut(&command.class) else {
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
        let keys: HashSet<_> = ALL_CLASSES.iter().map(|class| fault_key(*class)).collect();
        assert_eq!(keys.len(), ALL_CLASSES.len());
    }

    #[test]
    fn catalog_contains_every_fault_key() {
        let catalog: serde_json::Value = serde_json::from_str(include_str!(
            "../../../config/catalog/battery_guardian.json"
        ))
        .expect("catalog json");
        let ids: HashSet<&str> = catalog["faults"]
            .as_array()
            .expect("faults array")
            .iter()
            .filter_map(|fault| fault["id"]["Text"].as_str())
            .collect();
        for class in ALL_CLASSES {
            assert!(ids.contains(fault_key(class)), "{}", fault_key(class));
        }
        assert_eq!(ids.len(), ALL_CLASSES.len());
    }

    #[test]
    fn fault_stays_failed_until_last_signal_clears() {
        let mut state = FaultState::default();
        let class = DetectionClass::PhysicalTempRate;
        assert_eq!(state.apply(class, Some(Signal::TempMin), true), Some(true));
        assert_eq!(state.apply(class, Some(Signal::TempMax), true), None);
        assert_eq!(state.apply(class, Some(Signal::TempMin), false), None);
        assert_eq!(state.apply(class, Some(Signal::TempMax), false), Some(false));
    }

    #[test]
    fn clearing_inactive_fault_is_not_a_change() {
        let mut state = FaultState::default();
        assert_eq!(state.apply(DetectionClass::SignalStuck, None, false), None);
    }
}
