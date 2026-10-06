//! Guardian → DFM fault reporting bridge.
//!
//! Owns the `fault_lib` [`Reporter`] instances (which wrap non-`Send`
//! iceoryx2 IPC ports) on a dedicated OS thread and exposes a cheap,
//! `Send + Sync + Clone` handle to the async Guardian. Detection logic in
//! the Guardian sends [`FaultCommand`]s; the worker translates them into
//! `Failed`/`Passed` fault-lifecycle records published to the Diagnostic
//! Fault Manager over IPC.

use std::collections::HashMap;
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

/// Fault keys — must match the `Text` fault ids in the DFM catalog JSON.
pub mod keys {
    pub const OVERTEMP_WARNING: &str = "BatteryOverTempWarning";
    pub const OVERTEMP_CRITICAL: &str = "BatteryOverTempCritical";
    pub const SIGNAL_STALE: &str = "BatteryTempSignalStale";
    pub const SIGNAL_STUCK: &str = "BatteryTempSignalStuck";
    pub const IMPLAUSIBLE: &str = "BatteryTempImplausible";

    /// All fault keys, in catalog order.
    pub const ALL: &[&str] = &[
        OVERTEMP_WARNING,
        OVERTEMP_CRITICAL,
        SIGNAL_STALE,
        SIGNAL_STUCK,
        IMPLAUSIBLE,
    ];
}

/// A request to set a fault's condition (failed = active, else healed).
struct FaultCommand {
    key: &'static str,
    failed: bool,
    env: Vec<(String, String)>,
}

/// Cheap, cloneable handle used by the async Guardian to raise/clear faults.
#[derive(Clone)]
pub struct FaultReporterHandle {
    tx: Sender<FaultCommand>,
    ready: Arc<AtomicBool>,
}

impl FaultReporterHandle {
    /// Raise (`failed = true`) or heal (`failed = false`) a fault, attaching
    /// optional environment/evidence key-value pairs (max 8 are kept).
    pub fn set(&self, key: &'static str, failed: bool, env: Vec<(String, String)>) {
        if self.tx.send(FaultCommand { key, failed, env }).is_err() {
            warn!("[FaultReporter] worker thread gone; dropping {key} command");
        }
    }

    /// Convenience: raise a fault as failed with evidence.
    pub fn raise(&self, key: &'static str, env: Vec<(String, String)>) {
        self.set(key, true, env);
    }

    /// Convenience: heal a fault (report Passed).
    pub fn clear(&self, key: &'static str) {
        self.set(key, false, Vec::new());
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
/// DFM loaded). `sovd_path` is the SOVD entity path / catalog id used when
/// publishing (e.g. `"battery_guardian"`).
#[must_use]
pub fn spawn(catalog_path: PathBuf, sovd_path: String) -> FaultReporterHandle {
    let (tx, rx) = mpsc::channel::<FaultCommand>();
    let ready = Arc::new(AtomicBool::new(false));
    let ready_worker = Arc::clone(&ready);

    thread::Builder::new()
        .name("fault-reporter".into())
        .spawn(move || worker(catalog_path, sovd_path, rx, ready_worker))
        .expect("spawn fault-reporter thread");

    FaultReporterHandle { tx, ready }
}

fn build_catalog(catalog_path: &PathBuf) -> fault_lib::catalog::FaultCatalog {
    FaultCatalogBuilder::new()
        .json_file(catalog_path.clone())
        .expect("load fault catalog json")
        .build()
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
        .take(8)
        .filter_map(|(k, v)| {
            Some((to_static_short_string(k).ok()?, to_static_short_string(v).ok()?))
        })
        .collect();
    MetadataVec::try_from(&pairs[..]).unwrap_or_else(|_| MetadataVec::new())
}

fn worker(
    catalog_path: PathBuf,
    sovd_path: String,
    rx: Receiver<FaultCommand>,
    ready: Arc<AtomicBool>,
) {
    // FaultApi initialisation requires the DFM to be up (IPC sink + catalog
    // hash verification). Retry until it succeeds.
    let mut attempt: u32 = 0;
    let _api = loop {
        attempt += 1;
        match FaultApi::try_new(build_catalog(&catalog_path)) {
            Ok(api) => {
                info!("[FaultReporter] Connected to DFM; fault reporting active");
                break api;
            }
            Err(e) => {
                if attempt == 1 || attempt % 10 == 0 {
                    warn!("[FaultReporter] DFM not ready ({e}); retrying (attempt {attempt})");
                }
                thread::sleep(Duration::from_millis(500));
            }
        }
    };

    let config = reporter_config();
    let mut reporters: HashMap<&'static str, Reporter> = HashMap::new();
    for &key in keys::ALL {
        let id = FaultId::Text(to_static_short_string(key).expect("fault key fits ShortString"));
        match Reporter::new(&id, config.clone()) {
            Ok(r) => {
                reporters.insert(key, r);
            }
            Err(e) => error!("[FaultReporter] failed to create reporter for {key}: {e}"),
        }
    }

    // De-duplicate: only publish when a fault's failed/healed state changes.
    let mut last_failed: HashMap<&'static str, bool> = HashMap::new();

    // Publish an initial all-clear (Passed) baseline for every known fault so
    // the DFM store is populated and the OpenSOVD fault interface is queryable
    // immediately, before any temperature events have been evaluated.
    for (&key, reporter) in reporters.iter_mut() {
        let record = reporter.create_record(LifecycleStage::Passed);
        match reporter.publish(&sovd_path, record) {
            Ok(()) => {
                last_failed.insert(key, false);
            }
            Err(e) => error!("[FaultReporter] initial baseline publish {key} failed: {e}"),
        }
    }
    info!("[FaultReporter] published initial all-clear baseline for {} faults", reporters.len());

    ready.store(true, Ordering::Relaxed);

    while let Ok(cmd) = rx.recv() {
        if last_failed.get(cmd.key) == Some(&cmd.failed) {
            continue;
        }
        let Some(reporter) = reporters.get_mut(cmd.key) else {
            warn!("[FaultReporter] unknown fault key {}", cmd.key);
            continue;
        };

        let stage = if cmd.failed {
            LifecycleStage::Failed
        } else {
            LifecycleStage::Passed
        };
        let mut record = reporter.create_record(stage);
        record.env_data = env_to_metadata(&cmd.env);

        match reporter.publish(&sovd_path, record) {
            Ok(()) => {
                last_failed.insert(cmd.key, cmd.failed);
                if cmd.failed {
                    warn!("[FaultReporter] RAISED fault {} -> DFM", cmd.key);
                } else {
                    info!("[FaultReporter] cleared fault {} -> DFM", cmd.key);
                }
            }
            Err(e) => error!("[FaultReporter] publish {} failed: {e}", cmd.key),
        }
    }

    info!("[FaultReporter] command channel closed; worker exiting");
}
