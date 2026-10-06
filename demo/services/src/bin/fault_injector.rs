//! Fault Injector — scenario-driven battery temperature source.
//!
//! Publishes `BatteryTempEvent` over the same uProtocol/Zenoh topic the
//! Guardian subscribes to, shaping the signal to exercise a specific fault
//! class. Selected with the `SCENARIO` environment variable:
//!
//!   nominal   – safe ramp that stays below the WARNING threshold (no faults)
//!   overtemp  – ramp through WARNING (45 C) and CRITICAL (55 C)
//!   stuck     – emit a constant value (frozen signal)
//!   spike     – emit an out-of-range / implausible value
//!   stale     – emit a few samples then stop (freshness deadline elapses)
//!
//! Transport-level faults (delay / drop of otherwise-valid samples) are
//! injected out-of-band with Toxiproxy on the Zenoh link; this binary only
//! shapes the *signal*.

use std::time::Duration;

use dr_whodunit_services::{
    make_uri_provider, now_ms, open_up_transport, publish_json_event, vss_battery_high_temp_uri,
    vss_battery_temp_uri, BatteryTempEvent, HighTempAlert, HIGH_TEMP_THRESHOLD,
};
use tokio::time::{sleep, interval};
use tracing::info;

fn temp_event(temp_max: f32) -> BatteryTempEvent {
    BatteryTempEvent {
        temp_max,
        temp_avg: temp_max - 2.0,
        temp_min: temp_max - 7.0,
        soc: 75.0,
        timestamp_ms: now_ms(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "fault_injector=info,info".to_string()),
        )
        .init();

    let scenario = std::env::var("SCENARIO").unwrap_or_else(|_| "nominal".to_string());
    let interval_ms: u64 = std::env::var("PUBLISH_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);

    info!("===========================================");
    info!("  Fault Injector — scenario '{}'", scenario);
    info!("  Topic: {}", vss_battery_temp_uri().to_uri(false));
    info!("===========================================");

    let transport = open_up_transport(make_uri_provider("injector", 0x0002, 0x01)).await?;

    // Give the Zenoh session time to establish with the subscriber.
    sleep(Duration::from_millis(1500)).await;

    // Build the temperature series for the selected scenario.
    let series: Vec<f32> = match scenario.as_str() {
        "overtemp" => {
            // 30 -> 60 in 2C steps, then oscillate 60/58 to stay CRITICAL
            // without tripping the stuck-signal detector.
            let mut v: Vec<f32> = (0..=15).map(|i| 30.0 + i as f32 * 2.0).collect();
            for i in 0..10 {
                v.push(if i % 2 == 0 { 58.0 } else { 60.0 });
            }
            v
        }
        "stuck" => vec![40.0; 12],
        "spike" => vec![38.0, 39.0, 40.0, 300.0, 41.0, 42.0, 43.0, 320.0],
        "stale" => vec![35.0, 36.0, 37.0, 38.0], // then stop -> watchdog fires
        "stream" => {
            // Long, safe oscillation (30..40) used as a steady source while a
            // transport fault (e.g. Toxiproxy latency/timeout) is injected on
            // the Zenoh link. Never trips a signal-level fault on its own.
            let steps: usize = std::env::var("STEPS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(200);
            (0..steps)
                .map(|i| 30.0 + (i % 10) as f32)
                .collect()
        }
        _ /* nominal */ => {
            let up: Vec<f32> = (0..=14).map(|i| 30.0 + i as f32).collect(); // 30..44
            let down: Vec<f32> = (0..=14).rev().map(|i| 30.0 + i as f32).collect();
            [up, down].concat()
        }
    };

    let mut tick = interval(Duration::from_millis(interval_ms));
    for (i, &temp_max) in series.iter().enumerate() {
        tick.tick().await;
        let event = temp_event(temp_max);
        info!("[Inject] step={} TempMax={:.1}C", i, event.temp_max);
        let _ = publish_json_event(transport.clone(), vss_battery_temp_uri(), &event).await;

        if event.temp_max > HIGH_TEMP_THRESHOLD {
            let alert = HighTempAlert {
                value: event.temp_max,
                severity: "WARNING".to_string(),
                unit: "degC".to_string(),
                source: "injector".to_string(),
                timestamp_ms: event.timestamp_ms,
            };
            let _ = publish_json_event(transport.clone(), vss_battery_high_temp_uri(), &alert).await;
        }
    }

    info!("[Inject] scenario '{}' series complete", scenario);
    // Brief settle so the last publish is delivered before the process exits.
    sleep(Duration::from_millis(500)).await;
    Ok(())
}
