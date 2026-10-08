// Copyright (c) 2026 Matthias Knöfel
// Copyright (c) 2026 Peter Ulbrich
//
// This program and the accompanying materials are made available under
// the terms of the Eclipse Public License 2.0 which accompanies this
// distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
//
// AI Disclosure: This file was mostly AI-generated.
//
// SPDX-License-Identifier: EPL-2.0 and CC0-1.0
// Assisted-by: Claude Opus 5.5
//! Guardian → DFM fault reporting.
//!
//! The `fault_lib` [`Reporter`] wraps non-`Send` iceoryx2 IPC ports, so all
//! reporters live on a dedicated OS thread. The async Guardian only holds a
//! cheap, cloneable [`FaultReporterHandle`] and forwards fault-level
//! [`FaultEvent`]s (projected and aggregated by [`crate::guardian_faults`]) to
//! it. The worker publishes `Failed`/`Passed` fault records to the Diagnostic
//! Fault Manager.

use std::collections::HashMap;
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
use tracing::{error, info, warn};

use crate::guardian_faults::{
    DiagnosticKey, FaultEvent, FaultStage, DFM_MAPPINGS, GUARDIAN_SOURCE,
};

/// Maximum number of env-data entries attached to a fault record.
const MAX_ENV_ENTRIES: usize = 8;

/// Pause between baseline publishes. The DFM's iceoryx2 subscriber buffers
/// only 2 events (iceoryx2 default) and drains them every 10 ms; a burst of all
/// baseline records would silently overwrite all but the last two.
const BASELINE_PUBLISH_PAUSE: Duration = Duration::from_millis(20);

/// Cheap, cloneable handle used by the async Guardian to report detections.
#[derive(Clone)]
pub struct FaultReporterHandle {
    tx: Sender<FaultEvent>,
    ready: Arc<AtomicBool>,
}

impl FaultReporterHandle {
    /// Forward a fault-level change to the DFM worker.
    pub fn report(&self, event: &FaultEvent) {
        if self.tx.send(event.clone()).is_err() {
            warn!(
                key = event.fault_id,
                "fault reporter worker gone; dropping fault event"
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
    let (tx, rx) = mpsc::channel::<FaultEvent>();
    let ready = Arc::new(AtomicBool::new(false));
    let ready_worker = Arc::clone(&ready);

    thread::Builder::new()
        .name("fault-reporter".into())
        .spawn(move || worker(&catalog_path, &sovd_path, &rx, &ready_worker))
        .expect("spawn fault-reporter thread");

    FaultReporterHandle { tx, ready }
}

fn reporter_config() -> ReporterConfig {
    ReporterConfig {
        source: SourceId {
            entity: to_static_short_string(GUARDIAN_SOURCE.entity)
                .expect("entity name fits ShortString"),
            ecu: to_static_short_string(GUARDIAN_SOURCE.ecu).ok(),
            domain: to_static_short_string(GUARDIAN_SOURCE.domain).ok(),
            sw_component: to_static_short_string(GUARDIAN_SOURCE.sw_component).ok(),
            instance: to_static_short_string(GUARDIAN_SOURCE.instance).ok(),
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

fn worker(catalog_path: &Path, sovd_path: &str, rx: &Receiver<FaultEvent>, ready: &AtomicBool) {
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
    let mut reporters: HashMap<DiagnosticKey, (&'static str, Reporter)> = HashMap::new();
    for (diagnostic, key) in DFM_MAPPINGS {
        let id = FaultId::Text(to_static_short_string(key).expect("fault key fits ShortString"));
        match Reporter::new(&id, config.clone()) {
            Ok(reporter) => {
                reporters.insert(diagnostic, (key, reporter));
            }
            Err(error) => error!(key, %error, "failed to create fault reporter"),
        }
    }

    // Publish an initial all-clear baseline so the DFM store is populated
    // before the first detection.
    for (key, reporter) in reporters.values_mut() {
        let record = reporter.create_record(LifecycleStage::Passed);
        if let Err(error) = reporter.publish(sovd_path, record) {
            error!(key = *key, %error, "initial baseline publish failed");
        }
        thread::sleep(BASELINE_PUBLISH_PAUSE);
    }
    info!(
        faults = reporters.len(),
        "published initial all-clear baseline"
    );
    ready.store(true, Ordering::Relaxed);

    while let Ok(event) = rx.recv() {
        let key = event.fault_id;
        let Some((_, reporter)) = reporters.get_mut(&event.diagnostic) else {
            warn!(key, "no reporter for fault");
            continue;
        };

        let failed = event.stage == FaultStage::Failed;
        let stage = if failed {
            LifecycleStage::Failed
        } else {
            LifecycleStage::Passed
        };
        let mut record = reporter.create_record(stage);
        record.env_data = env_to_metadata(&event.env_pairs());

        match reporter.publish(sovd_path, record) {
            Ok(()) if failed => warn!(key, "raised fault -> DFM"),
            Ok(()) => info!(key, "cleared fault -> DFM"),
            Err(error) => error!(key, %error, "publish to DFM failed"),
        }
    }
    info!("command channel closed; fault reporter worker exiting");
}
