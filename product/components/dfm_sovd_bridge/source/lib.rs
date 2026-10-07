//! DFM → OpenSOVD bridge.
//!
//! Exposes the faults the DFM stores for one SOVD entity path (e.g.
//! `battery_guardian`) as data resources of an OpenSOVD component. The DFM is
//! queried live over its `dfm/query` IPC service on every read.

pub mod dfm_client;
pub mod fault_view;

use async_trait::async_trait;
use dfm_lib::sovd_fault_manager::Error as DfmError;
use opensovd_core::{Component, DataError};
use opensovd_models::data::DataCategory;
use opensovd_providers::data::{DataProviderBuilder, ReadableDataResource};

pub use dfm_client::DfmClient;
pub use fault_view::{FaultDetail, FaultList, FaultView};

/// Data category of all bridge resources (SOVD custom category).
pub const FAULT_CATEGORY: &str = "x-dfm-faults";

fn to_data_error(e: DfmError) -> DataError {
    match e {
        DfmError::NotFound => DataError::NotFound("fault not found in DFM".into()),
        other => DataError::Internal(format!("DFM query failed: {other}")),
    }
}

/// `faults` / `faults.active`: all faults of the entity path, optionally only
/// those whose current test result is failed.
struct FaultListResource {
    dfm: DfmClient,
    path: String,
    active_only: bool,
}

#[async_trait]
impl ReadableDataResource for FaultListResource {
    type Value = FaultList;

    async fn read(&self) -> Result<Self::Value, DataError> {
        let faults = self
            .dfm
            .all_faults(&self.path)
            .await
            .map_err(to_data_error)?;
        let items = faults
            .iter()
            .map(FaultView::from)
            .filter(|f| !self.active_only || f.is_active())
            .collect();
        Ok(FaultList { items })
    }
}

/// `fault.<code>`: one fault including its environment data.
struct FaultDetailResource {
    dfm: DfmClient,
    path: String,
    code: String,
}

#[async_trait]
impl ReadableDataResource for FaultDetailResource {
    type Value = FaultDetail;

    async fn read(&self) -> Result<Self::Value, DataError> {
        let (fault, env) = self
            .dfm
            .fault(&self.path, &self.code)
            .await
            .map_err(to_data_error)?;
        Ok(FaultDetail::new(&fault, &env))
    }
}

/// Build the SOVD component for `path`.
///
/// `fault_codes` are the DFM fault codes known at startup; each one gets its
/// own `fault.<code>` resource. `faults` and `faults.active` always exist.
pub fn build_component(
    dfm: &DfmClient,
    path: &str,
    name: &str,
    fault_codes: &[String],
) -> anyhow::Result<Component> {
    let category = DataCategory::Custom(FAULT_CATEGORY.into());
    let mut builder = DataProviderBuilder::new()
        .read_data(
            "faults",
            "All DFM faults",
            &category,
            FaultListResource {
                dfm: dfm.clone(),
                path: path.into(),
                active_only: false,
            },
        )
        .read_data(
            "faults.active",
            "Active DFM faults (testFailed)",
            &category,
            FaultListResource {
                dfm: dfm.clone(),
                path: path.into(),
                active_only: true,
            },
        );
    for code in fault_codes {
        builder = builder.read_data(
            format!("fault.{code}"),
            code.clone(),
            &category,
            FaultDetailResource {
                dfm: dfm.clone(),
                path: path.into(),
                code: code.clone(),
            },
        );
    }
    let provider = builder.build()?;
    Ok(Component::new(path, name).with_data_provider(provider))
}
