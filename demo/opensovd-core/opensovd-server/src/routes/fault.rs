// SPDX-FileCopyrightText: Copyright (c) 2026 Contributors to the Eclipse Foundation
// SPDX-License-Identifier: Apache-2.0

//! Fault resource endpoints.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::get,
};
use opensovd_core::{Fault as CoreFault, Topology};
use opensovd_models::{
    Response,
    fault::{Fault, FaultStatus, Faults},
};

use super::AppState;
use super::error::{Error, Result};

pub fn routes<V>() -> Router<AppState<V>>
where
    V: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/components/{component_id}/faults",
            get(component_fault_list).delete(component_fault_clear_all),
        )
        .route(
            "/components/{component_id}/faults/{fault_code}",
            get(component_fault_get).delete(component_fault_clear),
        )
        .route(
            "/apps/{app_id}/faults",
            get(app_fault_list).delete(app_fault_clear_all),
        )
        .route(
            "/apps/{app_id}/faults/{fault_code}",
            get(app_fault_get).delete(app_fault_clear),
        )
}

async fn component_fault_list(
    State(topology): State<Topology>,
    Path(component_id): Path<String>,
) -> Result<Json<Response<Faults>>> {
    let topology = topology.read().await;
    let entity = topology
        .get_component(&component_id)
        .map_err(|_| Error::EntityNotFound(component_id.clone()))?;
    let provider = entity
        .fault_provider()
        .ok_or_else(|| Error::ProviderNotAvailable("faults".into()))?;
    let items = provider.list().await?.into_iter().map(fault).collect();
    Ok(Json(Response {
        data: Faults { items },
        schema: None,
    }))
}

async fn component_fault_get(
    State(topology): State<Topology>,
    Path((component_id, fault_code)): Path<(String, String)>,
) -> Result<Json<Response<Fault>>> {
    let topology = topology.read().await;
    let entity = topology
        .get_component(&component_id)
        .map_err(|_| Error::EntityNotFound(component_id.clone()))?;
    let provider = entity
        .fault_provider()
        .ok_or_else(|| Error::ProviderNotAvailable("faults".into()))?;
    let (fault, environment_data) = provider.get(&fault_code).await?;
    Ok(Json(Response {
        data: fault_with_environment(fault, environment_data),
        schema: None,
    }))
}

async fn component_fault_clear_all(
    State(topology): State<Topology>,
    Path(component_id): Path<String>,
) -> Result<StatusCode> {
    let topology = topology.read().await;
    let entity = topology
        .get_component(&component_id)
        .map_err(|_| Error::EntityNotFound(component_id.clone()))?;
    let provider = entity
        .fault_provider()
        .ok_or_else(|| Error::ProviderNotAvailable("faults".into()))?;
    provider.clear_all().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn component_fault_clear(
    State(topology): State<Topology>,
    Path((component_id, fault_code)): Path<(String, String)>,
) -> Result<StatusCode> {
    let topology = topology.read().await;
    let entity = topology
        .get_component(&component_id)
        .map_err(|_| Error::EntityNotFound(component_id.clone()))?;
    let provider = entity
        .fault_provider()
        .ok_or_else(|| Error::ProviderNotAvailable("faults".into()))?;
    provider.clear(&fault_code).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn app_fault_list(
    State(topology): State<Topology>,
    Path(app_id): Path<String>,
) -> Result<Json<Response<Faults>>> {
    let topology = topology.read().await;
    let entity = topology
        .get_app(&app_id)
        .map_err(|_| Error::EntityNotFound(app_id.clone()))?;
    let provider = entity
        .fault_provider()
        .ok_or_else(|| Error::ProviderNotAvailable("faults".into()))?;
    let items = provider.list().await?.into_iter().map(fault).collect();
    Ok(Json(Response {
        data: Faults { items },
        schema: None,
    }))
}

async fn app_fault_get(
    State(topology): State<Topology>,
    Path((app_id, fault_code)): Path<(String, String)>,
) -> Result<Json<Response<Fault>>> {
    let topology = topology.read().await;
    let entity = topology
        .get_app(&app_id)
        .map_err(|_| Error::EntityNotFound(app_id.clone()))?;
    let provider = entity
        .fault_provider()
        .ok_or_else(|| Error::ProviderNotAvailable("faults".into()))?;
    let (fault, environment_data) = provider.get(&fault_code).await?;
    Ok(Json(Response {
        data: fault_with_environment(fault, environment_data),
        schema: None,
    }))
}

async fn app_fault_clear_all(
    State(topology): State<Topology>,
    Path(app_id): Path<String>,
) -> Result<StatusCode> {
    let topology = topology.read().await;
    let entity = topology
        .get_app(&app_id)
        .map_err(|_| Error::EntityNotFound(app_id.clone()))?;
    let provider = entity
        .fault_provider()
        .ok_or_else(|| Error::ProviderNotAvailable("faults".into()))?;
    provider.clear_all().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn app_fault_clear(
    State(topology): State<Topology>,
    Path((app_id, fault_code)): Path<(String, String)>,
) -> Result<StatusCode> {
    let topology = topology.read().await;
    let entity = topology
        .get_app(&app_id)
        .map_err(|_| Error::EntityNotFound(app_id.clone()))?;
    let provider = entity
        .fault_provider()
        .ok_or_else(|| Error::ProviderNotAvailable("faults".into()))?;
    provider.clear(&fault_code).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn fault(source: CoreFault) -> Fault {
    Fault {
        code: source.code,
        display_code: source.display_code,
        scope: source.scope,
        fault_name: source.name,
        fault_translation_id: source.translation_id,
        severity: source.severity,
        status: FaultStatus {
            test_failed: source.status.test_failed,
            test_failed_this_operation_cycle: source.status.test_failed_this_operation_cycle,
            pending_dtc: source.status.pending_dtc,
            confirmed_dtc: source.status.confirmed_dtc,
            test_not_completed_since_last_clear: source.status.test_not_completed_since_last_clear,
            test_failed_since_last_clear: source.status.test_failed_since_last_clear,
            test_not_completed_this_operation_cycle: source
                .status
                .test_not_completed_this_operation_cycle,
            warning_indicator_requested: source.status.warning_indicator_requested,
            mask: format!("0x{:02X}", source.status.mask()),
        },
        symptom: source.symptom,
        symptom_translation_id: source.symptom_translation_id,
        schema: source.schema,
        occurrence_counter: source.occurrence_counter,
        aging_counter: source.aging_counter,
        healing_counter: source.healing_counter,
        first_occurrence: source.first_occurrence,
        last_occurrence: source.last_occurrence,
        environment_data: None,
    }
}

fn fault_with_environment(
    source: CoreFault,
    environment_data: std::collections::BTreeMap<String, String>,
) -> Fault {
    let mut result = fault(source);
    result.environment_data = (!environment_data.is_empty()).then_some(environment_data);
    result
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use opensovd_core::{App, Component, FaultError, FaultProvider, FaultResult, FaultStatus};
    use tower::ServiceExt;

    use super::*;

    #[derive(Clone)]
    struct TestProvider {
        clear_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl FaultProvider for TestProvider {
        async fn list(&self) -> FaultResult<Vec<CoreFault>> {
            Ok(vec![test_fault()])
        }

        async fn get(
            &self,
            fault_code: &str,
        ) -> FaultResult<(CoreFault, BTreeMap<String, String>)> {
            if fault_code != "P1001" {
                return Err(FaultError::NotFound(fault_code.into()));
            }
            Ok((
                test_fault(),
                BTreeMap::from([("rpm".into(), "1200".into())]),
            ))
        }

        async fn clear_all(&self) -> FaultResult<()> {
            self.clear_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn clear(&self, _fault_code: &str) -> FaultResult<()> {
            self.clear_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn test_fault() -> CoreFault {
        CoreFault {
            code: "P1001".into(),
            display_code: "P1001".into(),
            scope: "ecu".into(),
            name: "Intake actuator".into(),
            translation_id: "fault.P1001".into(),
            severity: 2,
            status: FaultStatus {
                test_failed: true,
                confirmed_dtc: true,
                ..FaultStatus::default()
            },
            symptom: Some("Actuator stuck".into()),
            symptom_translation_id: None,
            schema: None,
            occurrence_counter: 3,
            aging_counter: 1,
            healing_counter: 0,
            first_occurrence: None,
            last_occurrence: None,
        }
    }

    async fn topology_with_component(provider: TestProvider) -> Topology {
        let topology = Topology::new();
        topology
            .write()
            .await
            .add_component(Component::new("ecu", "ECU").with_fault_provider(provider));
        topology
    }

    #[tokio::test]
    async fn component_faults_are_listed_and_read() {
        let provider = TestProvider {
            clear_calls: Arc::new(AtomicUsize::new(0)),
        };
        let state = AppState::<()> {
            vendor_info: None,
            topology: topology_with_component(provider).await,
        };

        let response = routes::<()>()
            .with_state(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/components/ecu/faults")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["items"][0]["code"], "P1001");
        assert_eq!(json["items"][0]["status"]["mask"], "0x09");

        let response = routes::<()>()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/components/ecu/faults/P1001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["environment_data"]["rpm"], "1200");
    }

    #[tokio::test]
    async fn component_faults_are_cleared() {
        let clear_calls = Arc::new(AtomicUsize::new(0));
        let provider = TestProvider {
            clear_calls: Arc::clone(&clear_calls),
        };
        let state = AppState::<()> {
            vendor_info: None,
            topology: topology_with_component(provider).await,
        };

        let response = routes::<()>()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/components/ecu/faults/P1001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(clear_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn app_faults_are_listed() {
        let topology = Topology::new();
        topology
            .write()
            .await
            .add_app(
                App::new("app", "App", "ecu").with_fault_provider(TestProvider {
                    clear_calls: Arc::new(AtomicUsize::new(0)),
                }),
            );
        let state = AppState::<()> {
            vendor_info: None,
            topology,
        };

        let response = routes::<()>()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/apps/app/faults")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_fault_provider_returns_not_found() {
        let topology = Topology::new();
        topology
            .write()
            .await
            .add_component(Component::new("ecu", "ECU"));
        let state = AppState::<()> {
            vendor_info: None,
            topology,
        };

        let response = routes::<()>()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/components/ecu/faults")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
