//! Temperature Publisher (uProtocol/Zenoh)
//!
//! Simulates battery temperature data and publishes BatteryTempEvent over Zenoh
//! using the same uProtocol topic URI as the full CAN→KUKSA→vss_bridge pipeline.
//! Use this when KUKSA services are unavailable (e.g. corporate network restrictions).
//!
//! Scenario: temp rises 32→67°C (crossing WARNING@45°C and CRITICAL@55°C), then cools.

use std::time::Duration;

use dr_whodunit_services::{
    make_uri_provider, now_ms, open_up_transport, publish_json_event,
    vss_battery_temp_uri, vss_battery_high_temp_uri,
    BatteryTempEvent, HighTempAlert, HIGH_TEMP_THRESHOLD,
};
use tokio::time::{interval, sleep};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "temp_publisher=info,info".to_string()),
        )
        .init();

    info!("===========================================");
    info!("  Temperature Publisher  (uProtocol/Zenoh)");
    info!("===========================================");
    info!("Topic: {}", vss_battery_temp_uri().to_uri(false));

    let interval_ms: u64 = std::env::var("PUBLISH_INTERVAL_MS")
        .unwrap_or_else(|_| "500".into()).parse().unwrap_or(500);
    let temp_start: f32 = std::env::var("TEMP_START").unwrap_or_else(|_| "32.0".into()).parse().unwrap_or(32.0);
    let temp_max_val: f32 = std::env::var("TEMP_MAX").unwrap_or_else(|_| "67.0".into()).parse().unwrap_or(67.0);
    let rise_rate: f32 = std::env::var("TEMP_RISE_RATE").unwrap_or_else(|_| "0.5".into()).parse().unwrap_or(0.5);

    let transport = open_up_transport(make_uri_provider("vehicle", 0x0001, 0x01)).await?;

    info!("Waiting for Zenoh router…");
    sleep(Duration::from_secs(2)).await;
    info!("Starting temperature simulation: {:.0}°C → {:.0}°C at {:.1}°C/step", temp_start, temp_max_val, rise_rate);

    let rise_steps = ((temp_max_val - temp_start) / rise_rate) as u64;
    let cycle_len  = rise_steps * 2;
    let mut tick = interval(Duration::from_millis(interval_ms));
    let mut seq: u64 = 0;

    loop {
        tick.tick().await;
        seq += 1;

        let pos = seq % cycle_len;
        let temp_max = if pos < rise_steps {
            temp_start + pos as f32 * rise_rate
        } else {
            temp_max_val - (pos - rise_steps) as f32 * rise_rate
        }.clamp(temp_start, temp_max_val);

        let event = BatteryTempEvent {
            temp_max,
            temp_avg: temp_max - 2.0,
            temp_min: temp_max - 7.0,
            soc: (80.0_f32 - (pos as f32 * 0.1_f32).min(20.0)).max(60.0),
            timestamp_ms: now_ms(),
        };

        if seq % 10 == 0 {
            info!("[Pub] seq={} TempMax={:.1}°C TempAvg={:.1}°C SoC={:.0}%",
                seq, event.temp_max, event.temp_avg, event.soc);
        }

        let _ = publish_json_event(
            transport.clone(),
            vss_battery_temp_uri(),
            &event,
        ).await;

        // Publish high-temperature alert when threshold crossed
        if event.temp_max > HIGH_TEMP_THRESHOLD {
            let alert = HighTempAlert {
                value: event.temp_max,
                severity: "WARNING".to_string(),
                unit: "degC".to_string(),
                source: "sim".to_string(),
                timestamp_ms: event.timestamp_ms,
            };
            let _ = publish_json_event(
                transport.clone(),
                vss_battery_high_temp_uri(),
                &alert,
            ).await;
        }
    }
}
