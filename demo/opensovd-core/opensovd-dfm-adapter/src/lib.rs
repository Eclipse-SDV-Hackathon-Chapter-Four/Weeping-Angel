// SPDX-FileCopyrightText: Copyright (c) 2026 Contributors to the Eclipse Foundation
// SPDX-License-Identifier: Apache-2.0

//! Adapter from fault-lib's DFM query interface to OpenSOVD Core.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use dfm_lib::{DfmQueryApi, Iceoryx2DfmQuery};
use opensovd_core::{Fault, FaultError, FaultProvider, FaultResult, FaultStatus};

/// Exposes one DFM catalog path as an OpenSOVD fault provider.
pub struct DfmFaultProvider<Q> {
    query: Arc<Q>,
    entity_path: String,
}

impl<Q> DfmFaultProvider<Q> {
    #[must_use]
    pub fn new(query: Arc<Q>, entity_path: impl Into<String>) -> Self {
        Self {
            query,
            entity_path: entity_path.into(),
        }
    }
}

impl DfmFaultProvider<Iceoryx2DfmQuery> {
    /// Connects to the DFM `dfm/query` iceoryx2 service.
    ///
    /// # Errors
    ///
    /// Returns an error when the IPC client cannot be created.
    pub fn connect(entity_path: impl Into<String>) -> FaultResult<Self> {
        let query = Iceoryx2DfmQuery::new().map_err(map_error)?;
        Ok(Self::new(Arc::new(query), entity_path))
    }
}

#[async_trait]
impl<Q> FaultProvider for DfmFaultProvider<Q>
where
    Q: DfmQueryApi + Send + Sync + 'static,
{
    async fn list(&self) -> FaultResult<Vec<Fault>> {
        let query = Arc::clone(&self.query);
        let entity_path = self.entity_path.clone();
        tokio::task::spawn_blocking(move || query.get_all_faults(&entity_path))
            .await
            .map_err(|error| FaultError::Internal(format!("DFM query task failed: {error}")))?
            .map_err(map_error)
            .map(|faults| faults.into_iter().map(fault).collect())
    }

    async fn get(&self, fault_code: &str) -> FaultResult<(Fault, BTreeMap<String, String>)> {
        let query = Arc::clone(&self.query);
        let entity_path = self.entity_path.clone();
        let fault_code = fault_code.to_owned();
        tokio::task::spawn_blocking(move || query.get_fault(&entity_path, &fault_code))
            .await
            .map_err(|error| FaultError::Internal(format!("DFM query task failed: {error}")))?
            .map_err(map_error)
            .map(|(source, environment)| (fault(source), environment.into_iter().collect()))
    }

    async fn clear_all(&self) -> FaultResult<()> {
        let query = Arc::clone(&self.query);
        let entity_path = self.entity_path.clone();
        tokio::task::spawn_blocking(move || query.delete_all_faults(&entity_path))
            .await
            .map_err(|error| FaultError::Internal(format!("DFM query task failed: {error}")))?
            .map_err(map_error)
    }

    async fn clear(&self, fault_code: &str) -> FaultResult<()> {
        let query = Arc::clone(&self.query);
        let entity_path = self.entity_path.clone();
        let fault_code = fault_code.to_owned();
        tokio::task::spawn_blocking(move || query.delete_fault(&entity_path, &fault_code))
            .await
            .map_err(|error| FaultError::Internal(format!("DFM query task failed: {error}")))?
            .map_err(map_error)
    }
}

fn map_error(error: dfm_lib::sovd_fault_manager::Error) -> FaultError {
    match error {
        dfm_lib::sovd_fault_manager::Error::BadArgument => {
            FaultError::BadRequest("invalid DFM query".into())
        }
        dfm_lib::sovd_fault_manager::Error::NotFound => {
            FaultError::NotFound("fault not found".into())
        }
        dfm_lib::sovd_fault_manager::Error::Storage(message) => FaultError::Unavailable(message),
        _ => FaultError::Internal("unsupported DFM error".into()),
    }
}

fn fault(source: dfm_lib::sovd_fault_manager::SovdFault) -> Fault {
    let status = source.typed_status.unwrap_or_default();
    Fault {
        code: source.code,
        display_code: source.display_code,
        scope: source.scope,
        name: source.fault_name,
        translation_id: source.fault_translation_id,
        severity: source.severity,
        status: FaultStatus {
            test_failed: status.test_failed.unwrap_or(false),
            test_failed_this_operation_cycle: status
                .test_failed_this_operation_cycle
                .unwrap_or(false),
            pending_dtc: status.pending_dtc.unwrap_or(false),
            confirmed_dtc: status.confirmed_dtc.unwrap_or(false),
            test_not_completed_since_last_clear: status
                .test_not_completed_since_last_clear
                .unwrap_or(false),
            test_failed_since_last_clear: status.test_failed_since_last_clear.unwrap_or(false),
            test_not_completed_this_operation_cycle: status
                .test_not_completed_this_operation_cycle
                .unwrap_or(false),
            warning_indicator_requested: status.warning_indicator_requested.unwrap_or(false),
        },
        symptom: source.symptom,
        symptom_translation_id: source.symptom_translation_id,
        schema: source.schema,
        occurrence_counter: source.occurrence_counter.unwrap_or_default(),
        aging_counter: source.aging_counter.unwrap_or_default(),
        healing_counter: source.healing_counter.unwrap_or_default(),
        first_occurrence: source.first_occurrence,
        last_occurrence: source.last_occurrence,
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use dfm_lib::sovd_fault_manager::{Error, SovdFault, SovdFaultStatus};

    use super::*;

    #[test]
    fn storage_errors_map_to_unavailable() {
        let error = map_error(Error::Storage("DFM is offline".into()));
        assert!(matches!(error, FaultError::Unavailable(message) if message == "DFM is offline"));
    }

    #[test]
    fn sovd_fault_is_mapped_to_core_fault() {
        let source = SovdFault {
            code: "P1001".into(),
            display_code: "P1001".into(),
            scope: "ecu".into(),
            fault_name: "Intake actuator".into(),
            fault_translation_id: "fault.P1001".into(),
            severity: 2,
            typed_status: Some(SovdFaultStatus {
                test_failed: Some(true),
                confirmed_dtc: Some(true),
                ..SovdFaultStatus::default()
            }),
            occurrence_counter: Some(3),
            ..SovdFault::default()
        };

        let mapped = fault(source);
        assert_eq!(mapped.code, "P1001");
        assert!(mapped.status.test_failed);
        assert!(mapped.status.confirmed_dtc);
        assert_eq!(mapped.occurrence_counter, 3);
    }
}
