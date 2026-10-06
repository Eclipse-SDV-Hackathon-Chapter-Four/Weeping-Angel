//! Shared types, URI builders, and transport helpers.

pub mod fault_reporter;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use up_rust::{
    LocalUriProvider, StaticUriProvider, UMessage, UMessageBuilder, UPayloadFormat,
    UTransport, UUri,
};
use up_transport_zenoh::UPTransportZenoh;

// =============================================================================
// Resource IDs
// =============================================================================

pub const RID_BATTERY_TEMP_EVENT: u16      = 0x9001;
pub const RID_GUARDIAN_STATE_EVENT: u16    = 0x9002;
pub const RID_BATTERY_HIGH_TEMP_EVENT: u16 = 0x9003;

pub const HIGH_TEMP_THRESHOLD: f32 = 50.0;

// Zenoh key constants retained for reference; the Rust binaries use uProtocol URIs.
pub const ZENOH_KEY_BATTERY_TEMP: &str =
    "sdv/Vehicle/Powertrain/TractionBattery/temperature";
pub const ZENOH_KEY_HIGH_TEMP_ALERT: &str =
    "sdv/Vehicle/Powertrain/TractionBattery/high-temp-alert";

// =============================================================================
// Shared types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryTempEvent {
    pub temp_max: f32,
    pub temp_avg: f32,
    pub temp_min: f32,
    pub soc: f32,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighTempAlert {
    pub value: f32,
    pub severity: String,
    pub unit: String,
    pub source: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GuardianState {
    Clear,
    Monitoring,
    Warning,
    Critical,
    Mitigating,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianSnapshot {
    pub state: GuardianState,
    pub temp_max: f32,
    pub temp_avg: f32,
    pub soc: f32,
    pub timestamp_ms: u64,
}

// =============================================================================
// URI builders
// =============================================================================

pub fn vss_battery_temp_uri() -> UUri {
    UUri::try_from_parts("battery-vss", 0x9001, 0x01, RID_BATTERY_TEMP_EVENT).unwrap()
}

pub fn vss_guardian_state_uri() -> UUri {
    UUri::try_from_parts("guardian-vss", 0x9000, 0x01, RID_GUARDIAN_STATE_EVENT).unwrap()
}

pub fn vss_battery_high_temp_uri() -> UUri {
    UUri::try_from_parts("battery-vss", 0x9001, 0x01, RID_BATTERY_HIGH_TEMP_EVENT).unwrap()
}

// =============================================================================
// Transport helpers
// =============================================================================

pub fn make_uri_provider(
    authority: &str,
    entity_id: u32,
    major_version: u8,
) -> Arc<dyn LocalUriProvider> {
    Arc::new(StaticUriProvider::new(authority, entity_id, major_version))
}

pub async fn open_up_transport(
    uri_provider: Arc<dyn LocalUriProvider>,
) -> anyhow::Result<Arc<dyn UTransport>> {
    UPTransportZenoh::try_init_log_from_env();
    let mut config = zenoh::Config::default();
    if let Ok(endpoint) = std::env::var("ZENOH_CONNECT") {
        config
            .insert_json5("connect/endpoints", &format!("[\"{}\"]", endpoint))
            .map_err(|e| anyhow::anyhow!("Zenoh config: {}", e))?;
    }
    if let Ok(endpoint) = std::env::var("ZENOH_LISTEN") {
        config
            .insert_json5("listen/endpoints", &format!("[\"{}\"]", endpoint))
            .map_err(|e| anyhow::anyhow!("Zenoh listen config: {}", e))?;
    }
    let transport = UPTransportZenoh::builder(uri_provider.get_authority())
        .expect("invalid authority name")
        .with_config(config)
        .build()
        .await
        .map(Arc::new)?;
    Ok(transport)
}

pub fn decode_json_payload<T: serde::de::DeserializeOwned>(
    message: &UMessage,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
    let Some(payload) = message.payload.clone() else {
        return Err("missing payload".into());
    };
    Ok(serde_json::from_slice::<T>(&payload)?)
}

pub async fn publish_json_event<T: Serialize>(
    transport: Arc<dyn UTransport>,
    topic: UUri,
    data: &T,
) -> Result<(), up_rust::UStatus> {
    use up_rust::communication::UPayload;
    let bytes = serde_json::to_vec(data)
        .map_err(|e| up_rust::UStatus::fail_with_code(up_rust::UCode::INVALID_ARGUMENT, e.to_string()))?;
    let payload = UPayload::new(bytes, UPayloadFormat::UPAYLOAD_FORMAT_JSON);
    let fmt = payload.payload_format();
    let message = UMessageBuilder::publish(topic)
        .build_with_payload(payload.payload(), fmt)
        .map_err(|e| up_rust::UStatus::fail_with_code(up_rust::UCode::INVALID_ARGUMENT, e.to_string()))?;
    transport.send(message).await
}

// =============================================================================
// Utilities
// =============================================================================

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn evaluate_thermal_state(temp_max: f32) -> GuardianState {
    if temp_max >= 55.0 {
        GuardianState::Critical
    } else if temp_max >= 45.0 {
        GuardianState::Warning
    } else {
        GuardianState::Monitoring
    }
}
