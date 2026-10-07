//! Battery Guardian transport adapter and HTTP service.
//!
//! The uProtocol listener only stores observations. All physical evaluation is
//! performed by the periodic Guardian task in the transport-independent core.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use battery_guardian::guardian_reporting::{self, FaultReporterHandle};
use battery_guardian::{BatterySample, Detection, GuardianConfig, GuardianRuntime};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};
use up_rust::{LocalUriProvider, StaticUriProvider, UListener, UMessage, UTransport, UUri};
use up_transport_zenoh::UPTransportZenoh;

const RID_BATTERY_TEMP_EVENT: u16 = 0x9001;

#[derive(Debug, Deserialize)]
struct BatteryTempEvent {
    temp_min: f32,
    temp_avg: f32,
    temp_max: f32,
    soc: f32,
    #[serde(default)]
    _timestamp_ms: Option<u64>,
}

#[derive(Clone)]
struct AppState {
    runtime: Arc<Mutex<GuardianRuntime>>,
}

struct BatteryTempListener {
    state: AppState,
}

#[async_trait]
impl UListener for BatteryTempListener {
    async fn on_receive(&self, message: UMessage) {
        let event = match decode_battery_event(&message) {
            Ok(event) => event,
            Err(error) => {
                warn!(%error, "discarding malformed battery temperature event");
                return;
            }
        };

        let received_at = Instant::now();
        let sample = BatterySample::new(
            event.temp_min,
            event.temp_avg,
            event.temp_max,
            event.soc,
            received_at,
        );
        self.state.runtime.lock().await.receive_sample(sample);
    }
}

fn decode_battery_event(message: &UMessage) -> Result<BatteryTempEvent> {
    let payload = message
        .payload
        .as_ref()
        .context("battery temperature event has no payload")?;
    serde_json::from_slice(payload).context("decode BatteryTempEvent JSON")
}

fn report_detection(detection: &Detection, faults: &FaultReporterHandle) {
    faults.report(detection);
    let signal = detection
        .signal
        .map(|signal| signal.as_str())
        .unwrap_or("-");
    if detection.active {
        warn!(
            class = detection.class.as_str(),
            signal,
            observed = ?detection.observed,
            limit = ?detection.limit,
            residual = ?detection.residual,
            "Guardian detection active"
        );
    } else {
        info!(
            class = detection.class.as_str(),
            signal, "Guardian detection cleared"
        );
    }
}

fn start_periodic_guardian(
    state: AppState,
    config: GuardianConfig,
    faults: FaultReporterHandle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(config.evaluation_period());
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let detections = state.runtime.lock().await.cycle(Instant::now(), &config);
            for detection in detections {
                report_detection(&detection, &faults);
            }
        }
    })
}

#[derive(Serialize)]
struct SensorSnapshot {
    temp_min: f32,
    temp_avg: f32,
    temp_max: f32,
    soc: f32,
}

#[derive(Serialize)]
struct GuardianSnapshot {
    stream: &'static str,
    latest_sample_generation: u64,
    last_evaluated_generation: u64,
    active_detection_count: usize,
    sample: Option<SensorSnapshot>,
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn get_state(State(state): State<AppState>) -> Json<GuardianSnapshot> {
    let runtime = state.runtime.lock().await;
    let stream = if runtime.latest_sample().is_none() {
        "WAITING_FOR_SAMPLE"
    } else if runtime.stream_is_stale() {
        "STALE"
    } else {
        "FRESH"
    };
    let sample = runtime.latest_sample().map(|sample| SensorSnapshot {
        temp_min: sample.temp_min,
        temp_avg: sample.temp_avg,
        temp_max: sample.temp_max,
        soc: sample.soc,
    });
    Json(GuardianSnapshot {
        stream,
        latest_sample_generation: runtime.latest_sample_generation(),
        last_evaluated_generation: runtime.last_evaluated_generation(),
        active_detection_count: runtime.active_detection_count(),
        sample,
    })
}

fn battery_temp_uri() -> UUri {
    UUri::try_from_parts("battery-vss", 0x9001, 0x01, RID_BATTERY_TEMP_EVENT)
        .expect("static BatteryTempEvent URI is valid")
}

fn make_uri_provider() -> Arc<dyn LocalUriProvider> {
    Arc::new(StaticUriProvider::new("guardian", 0x1001, 0x01))
}

async fn open_up_transport(uri_provider: Arc<dyn LocalUriProvider>) -> Result<Arc<dyn UTransport>> {
    UPTransportZenoh::try_init_log_from_env();
    let mut config = zenoh::Config::default();
    if let Ok(endpoint) = std::env::var("ZENOH_CONNECT") {
        config
            .insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
            .map_err(|error| anyhow::anyhow!("Zenoh connect endpoint: {error}"))?;
    }
    if let Ok(endpoint) = std::env::var("ZENOH_LISTEN") {
        config
            .insert_json5("listen/endpoints", &format!("[\"{endpoint}\"]"))
            .map_err(|error| anyhow::anyhow!("Zenoh listen endpoint: {error}"))?;
    }
    let transport = UPTransportZenoh::builder(uri_provider.get_authority())
        .context("create Zenoh transport builder")?
        .with_config(config)
        .build()
        .await
        .map_err(|error| anyhow::anyhow!("build uProtocol Zenoh transport: {error}"))?;
    Ok(Arc::new(transport))
}

fn fault_catalog_path() -> PathBuf {
    std::env::var_os("GUARDIAN_FAULT_CATALOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/battery_guardian/guardian_diagnostics.json")
        })
}

fn configuration_path() -> PathBuf {
    std::env::var_os("GUARDIAN_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/battery_guardian")
                .join("guardian_model.yaml")
        })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "guardian=info,battery_guardian=info,info".to_string()),
        )
        .init();

    let config_path = configuration_path();
    let config = GuardianConfig::load(&config_path)?;
    info!(
        path = %config_path.display(),
        evaluation_period_ms = config.evaluation_period_ms,
        missing_packet_timeout_ms = config.missing_packet_timeout_ms,
        "loaded Guardian configuration"
    );

    let state = AppState {
        runtime: Arc::new(Mutex::new(GuardianRuntime::new())),
    };
    let transport = open_up_transport(make_uri_provider()).await?;
    transport
        .register_listener(
            &battery_temp_uri(),
            None,
            Arc::new(BatteryTempListener {
                state: state.clone(),
            }),
        )
        .await
        .map_err(|error| anyhow::anyhow!("register BatteryTempEvent listener: {error}"))?;
    info!(uri = %battery_temp_uri().to_uri(false), "subscribed to battery temperature events");

    let sovd_path =
        std::env::var("GUARDIAN_SOVD_PATH").unwrap_or_else(|_| "battery_guardian".to_string());
    let faults = guardian_reporting::spawn(fault_catalog_path(), sovd_path);
    let _periodic_guardian = start_periodic_guardian(state.clone(), config, faults);

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let address = format!("{host}:{port}");
    let router = Router::new()
        .route("/health", get(health))
        .route("/state", get(get_state))
        .with_state(state);
    let listener = TcpListener::bind(&address)
        .await
        .with_context(|| format!("bind Guardian HTTP server to {address}"))?;
    info!(%address, "Guardian HTTP server listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("serve Guardian HTTP API")?;
    Ok(())
}
