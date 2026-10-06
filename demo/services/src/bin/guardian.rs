//! Battery Thermal Guardian
//!
//! Subscribes to BatteryTempEvent over the uProtocol/Zenoh bus,
//! logs received values, and exposes /health + /state HTTP endpoints.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use dr_whodunit_services::fault_reporter::{self, keys, FaultReporterHandle};
use dr_whodunit_services::{
    decode_json_payload, evaluate_thermal_state, make_uri_provider, now_ms,
    open_up_transport, vss_battery_temp_uri, vss_battery_high_temp_uri,
    BatteryTempEvent, GuardianSnapshot, GuardianState, HighTempAlert,
};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{info, warn};

// =============================================================================
// Detection thresholds
// =============================================================================

/// WARNING band lower bound (deg C).
const WARN_THRESHOLD: f32 = 45.0;
/// CRITICAL band lower bound (deg C).
const CRIT_THRESHOLD: f32 = 55.0;
/// Plausible sensor range from the CAN DBC (deg C).
const PLAUSIBLE_MIN: f32 = -40.0;
const PLAUSIBLE_MAX: f32 = 125.0;
/// Max believable change between two consecutive samples (deg C).
const MAX_STEP: f32 = 20.0;
/// Consecutive identical samples that indicate a stuck signal.
const STUCK_LIMIT: u32 = 5;
/// Freshness deadline before the temperature stream is considered stale (ms).
const STALE_TIMEOUT_MS: u64 = 2000;
/// How often the staleness watchdog runs (ms).
const WATCHDOG_INTERVAL_MS: u64 = 500;
use up_rust::communication::{InMemoryRpcClient};
use up_rust::{UListener, UMessage, UTransport};

// =============================================================================
// App state
// =============================================================================

#[derive(Clone)]
struct AppState {
    data: Arc<Mutex<GuardianRuntime>>,
}

struct GuardianRuntime {
    temp_max: f32,
    temp_avg: f32,
    soc: f32,
    current_state: GuardianState,
    prev_temp: Option<f32>,
    prev_time: Option<Instant>,
    faults: FaultReporterHandle,
    last_event_at: Instant,
    stuck_count: u32,
    got_first_event: bool,
}

impl GuardianRuntime {
    fn new(faults: FaultReporterHandle) -> Self {
        Self {
            temp_max: 0.0,
            temp_avg: 0.0,
            soc: 0.0,
            current_state: GuardianState::Clear,
            prev_temp: None,
            prev_time: None,
            faults,
            last_event_at: Instant::now(),
            stuck_count: 0,
            got_first_event: false,
        }
    }

    fn apply_event(&mut self, event: BatteryTempEvent) {
        self.temp_max = event.temp_max;
        self.temp_avg = event.temp_avg;
        self.soc = event.soc;

        let rate = match (self.prev_temp, self.prev_time) {
            (Some(prev), Some(t)) => {
                let mins = t.elapsed().as_secs_f32() / 60.0;
                if mins > 0.0 { (self.temp_max - prev) / mins } else { 0.0 }
            }
            _ => 0.0,
        };

        let new_state = if self.current_state == GuardianState::Clear {
            GuardianState::Monitoring
        } else {
            evaluate_thermal_state(self.temp_max)
        };

        info!(
            "TempMax: {:.1}C | TempAvg: {:.1}C | SoC: {:.0}% | Rate: {:.2}C/min -> {:?}",
            self.temp_max, self.temp_avg, self.soc, rate, new_state
        );

        self.evaluate_faults(event.temp_max);

        self.current_state = new_state;
        self.prev_temp = Some(self.temp_max);
        self.prev_time = Some(Instant::now());
        self.last_event_at = Instant::now();
        self.got_first_event = true;
    }

    /// Map the freshly received sample onto DFM fault conditions.
    fn evaluate_faults(&mut self, temp_max: f32) {
        // A fresh sample arrived -> the stream is not stale.
        self.faults.set(keys::SIGNAL_STALE, false, Vec::new());

        // ---- Implausible: out of range or an impossible jump between samples.
        let step = self.prev_temp.map(|p| (temp_max - p).abs()).unwrap_or(0.0);
        let out_of_range = !(PLAUSIBLE_MIN..=PLAUSIBLE_MAX).contains(&temp_max);
        let implausible = out_of_range || step > MAX_STEP;
        self.faults.set(
            keys::IMPLAUSIBLE,
            implausible,
            vec![
                ("temp_max".into(), format!("{temp_max:.1}")),
                ("step_c".into(), format!("{step:.1}")),
            ],
        );

        // ---- Stuck: exact same value across consecutive samples.
        if self.prev_temp == Some(temp_max) {
            self.stuck_count = self.stuck_count.saturating_add(1);
        } else {
            self.stuck_count = 0;
        }
        let stuck = self.stuck_count >= STUCK_LIMIT;
        self.faults.set(
            keys::SIGNAL_STUCK,
            stuck,
            vec![
                ("value_c".into(), format!("{temp_max:.1}")),
                ("repeat".into(), self.stuck_count.to_string()),
            ],
        );

        // ---- Over-temperature warning / critical.
        // Implausible samples are not trustworthy for thermal escalation.
        let warning = !implausible && temp_max >= WARN_THRESHOLD;
        let critical = !implausible && temp_max >= CRIT_THRESHOLD;
        self.faults.set(
            keys::OVERTEMP_WARNING,
            warning,
            vec![("temp_max".into(), format!("{temp_max:.1}"))],
        );
        self.faults.set(
            keys::OVERTEMP_CRITICAL,
            critical,
            vec![("temp_max".into(), format!("{temp_max:.1}"))],
        );
    }

    fn snapshot(&self) -> GuardianSnapshot {
        GuardianSnapshot {
            state: self.current_state,
            temp_max: self.temp_max,
            temp_avg: self.temp_avg,
            soc: self.soc,
            timestamp_ms: now_ms(),
        }
    }
}

// =============================================================================
// Listeners
// =============================================================================

struct BatteryTempListener {
    app: AppState,
    #[allow(dead_code)] // retained for future state-publish use
    transport: Arc<dyn UTransport>,
}

#[async_trait]
impl UListener for BatteryTempListener {
    async fn on_receive(&self, message: UMessage) {
        match decode_json_payload::<BatteryTempEvent>(&message) {
            Ok(event) => {
                info!("[Guardian] Battery Temperature Received = {:.1}C", event.temp_max);
                self.app.data.lock().await.apply_event(event);
            }
            Err(e) => warn!("Invalid battery event payload: {}", e),
        }
    }
}

struct HighTempListener;

#[async_trait]
impl UListener for HighTempListener {
    async fn on_receive(&self, message: UMessage) {
        match decode_json_payload::<HighTempAlert>(&message) {
            Ok(alert) => warn!(
                "[Guardian] HIGH TEMPERATURE ALERT: {:.1}C (severity: {}, source: {})",
                alert.value, alert.severity, alert.source
            ),
            Err(e) => warn!("Invalid high-temp alert payload: {}", e),
        }
    }
}

// =============================================================================
// HTTP handlers
// =============================================================================

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn get_state(State(app): State<AppState>) -> Json<GuardianSnapshot> {
    let guard = app.data.lock().await;
    Json(guard.snapshot())
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "guardian=info,info".to_string()),
        )
        .init();

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("{}:{}", host, port);

    let uri_provider = make_uri_provider("guardian", 0x1001, 0x01);
    let transport = open_up_transport(uri_provider.clone()).await?;
    // rpc_client is initialised here; wire it into a listener when mitigation RPC is needed
    let _rpc_client = Arc::new(InMemoryRpcClient::new(transport.clone(), uri_provider).await?);

    // ---- DFM fault reporting bridge --------------------------------------
    let catalog_path = PathBuf::from(
        std::env::var("FAULT_CATALOG")
            .unwrap_or_else(|_| "../diagnostics/catalog/battery_guardian.json".to_string()),
    );
    let sovd_path = std::env::var("SOVD_ENTITY").unwrap_or_else(|_| "battery_guardian".to_string());
    info!(
        "[Guardian] Fault reporting catalog={} entity={}",
        catalog_path.display(),
        sovd_path
    );
    let faults = fault_reporter::spawn(catalog_path, sovd_path);

    let app_state = AppState {
        data: Arc::new(Mutex::new(GuardianRuntime::new(faults))),
    };

    // ---- Staleness watchdog: raises SIGNAL_STALE when the temperature
    // stream goes quiet past the freshness deadline (delayed/dropped signal).
    {
        let data = app_state.data.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(WATCHDOG_INTERVAL_MS));
            loop {
                tick.tick().await;
                let guard = data.lock().await;
                if guard.got_first_event
                    && guard.last_event_at.elapsed() > Duration::from_millis(STALE_TIMEOUT_MS)
                {
                    let age = guard.last_event_at.elapsed().as_millis();
                    guard.faults.set(
                        keys::SIGNAL_STALE,
                        true,
                        vec![("age_ms".into(), age.to_string())],
                    );
                }
            }
        });
    }

    transport
        .register_listener(
            &vss_battery_temp_uri(),
            None,
            Arc::new(BatteryTempListener {
                app: app_state.clone(),
                transport: transport.clone(),
            }),
        )
        .await?;

    transport
        .register_listener(
            &vss_battery_high_temp_uri(),
            None,
            Arc::new(HighTempListener),
        )
        .await?;

    info!("[Guardian] Subscribed to {} -- waiting for VSS events", vss_battery_temp_uri().to_uri(false));

    let router = Router::new()
        .route("/health", get(health))
        .route("/state", get(get_state))
        .with_state(app_state);

    let listener = TcpListener::bind(&addr).await?;
    info!("[Guardian] HTTP server listening on {}", addr);

    axum::serve(listener, router)
        .with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; })
        .await?;

    Ok(())
}
