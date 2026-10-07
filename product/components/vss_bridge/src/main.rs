//! VSS Bridge — KUKSA Databroker → uProtocol/Zenoh
//!
//! Subscribes to battery temperature VSS signals from kuksa-databroker via gRPC,
//! assembles BatteryTempEvent, and publishes over uProtocol/Zenoh so the Guardian
//! receives the same message format as the sim path.
//!
//! `timestamp_ms` is the CAN source timestamp (VSS `SourceTimestamp`), not the
//! bridge's wall clock (ADR-013). The feeder sends one update per signal in DBC
//! order, `TimeStamp` first: each `SourceTimestamp` update opens a new sample,
//! which is published once all four values have arrived.

use std::sync::Arc;
use tokio_stream::StreamExt;
use tracing::{info, warn};
use vss_publisher::{
    BatteryTempEvent, HighTempAlert, HIGH_TEMP_THRESHOLD, make_uri_provider,
    open_up_transport, publish_json_event, vss_battery_high_temp_uri, vss_battery_temp_uri,
};

// Generated from proto/kuksa/val/v1/
mod kuksa {
    pub mod val {
        pub mod v1 {
            tonic::include_proto!("kuksa.val.v1");
        }
    }
}

use kuksa::val::v1::{datapoint::Value, val_client::ValClient, Field, SubscribeEntry, SubscribeRequest, View};

const VSS_SOURCE_TS: &str = "Vehicle.Powertrain.TractionBattery.SourceTimestamp";
const VSS_TEMP_MAX: &str = "Vehicle.Powertrain.TractionBattery.Temperature.Max";
const VSS_TEMP_AVG: &str = "Vehicle.Powertrain.TractionBattery.Temperature.Average";
const VSS_TEMP_MIN: &str = "Vehicle.Powertrain.TractionBattery.Temperature.Min";
const VSS_SOC:      &str = "Vehicle.Powertrain.TractionBattery.StateOfCharge.Current";

/// One frame's values, opened by its source timestamp.
#[derive(Debug, Default)]
struct PendingSample {
    timestamp_ms: u64,
    temp_max: Option<f32>,
    temp_avg: Option<f32>,
    temp_min: Option<f32>,
    soc: Option<f32>,
}

impl PendingSample {
    fn complete(&self) -> Option<BatteryTempEvent> {
        Some(BatteryTempEvent {
            temp_max: self.temp_max?,
            temp_avg: self.temp_avg?,
            temp_min: self.temp_min?,
            soc: self.soc?,
            timestamp_ms: self.timestamp_ms,
        })
    }
}

/// Groups per-signal VSS updates into one BatteryTempEvent per source timestamp.
#[derive(Debug, Default)]
struct SampleAssembler {
    pending: Option<PendingSample>,
}

impl SampleAssembler {
    /// Starts a new sample; an unfinished previous one is dropped and logged.
    fn source_timestamp(&mut self, timestamp_ms: u64) {
        if let Some(old) = self.pending.take() {
            warn!("[VssBridge] Dropping incomplete sample ts={} ms: {:?}", old.timestamp_ms, old);
        }
        self.pending = Some(PendingSample { timestamp_ms, ..Default::default() });
    }

    /// Records a battery value; returns the event once the sample is complete.
    /// Values without an open sample (before the first timestamp) are ignored.
    fn value(&mut self, path: &str, value: f32) -> Option<BatteryTempEvent> {
        let sample = self.pending.as_mut()?;
        match path {
            VSS_TEMP_MAX => sample.temp_max = Some(value),
            VSS_TEMP_AVG => sample.temp_avg = Some(value),
            VSS_TEMP_MIN => sample.temp_min = Some(value),
            VSS_SOC      => sample.soc = Some(value),
            _            => return None,
        }
        let event = sample.complete()?;
        self.pending = None;
        Some(event)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "vss_bridge=info,info".to_string()),
        )
        .init();

    let databroker_addr = std::env::var("DATABROKER_ADDR")
        .unwrap_or_else(|_| "http://kuksa-databroker:55555".to_string());

    info!("[VssBridge] Connecting to databroker at {}", databroker_addr);
    let mut client = ValClient::connect(databroker_addr.clone()).await
        .map_err(|e| anyhow::anyhow!("databroker connect failed: {}", e))?;
    info!("[VssBridge] Connected to kuksa-databroker");

    let transport = open_up_transport(make_uri_provider("vss-bridge", 0x1002, 0x01)).await?;
    info!("[VssBridge] uProtocol transport ready, publishing to {}",
        vss_battery_temp_uri().to_uri(false));

    let entries = [VSS_SOURCE_TS, VSS_TEMP_MAX, VSS_TEMP_AVG, VSS_TEMP_MIN, VSS_SOC]
        .iter()
        .map(|&path| SubscribeEntry {
            path: path.to_string(),
            view: View::CurrentValue as i32,
            fields: vec![Field::Value as i32],
        })
        .collect();

    let mut stream = client
        .subscribe(SubscribeRequest { entries })
        .await
        .map_err(|e| anyhow::anyhow!("subscribe failed: {}", e))?
        .into_inner();

    info!("[VssBridge] Subscribed to VSS temperature signals");

    let mut assembler = SampleAssembler::default();

    while let Some(result) = stream.next().await {
        let response = match result {
            Ok(r) => r,
            Err(e) => {
                warn!("[VssBridge] Stream error: {}", e);
                break;
            }
        };

        // A response may hold several entries (e.g. the initial notification) in
        // any order: apply the source timestamp before the values.
        let mut entries: Vec<_> = response.updates.into_iter().filter_map(|u| u.entry).collect();
        entries.sort_by_key(|e| e.path != VSS_SOURCE_TS);

        let mut events = Vec::new();
        for entry in entries {
            match (entry.path.as_str(), entry.value.and_then(|dp| dp.value)) {
                (VSS_SOURCE_TS, Some(Value::Uint32(ts))) => assembler.source_timestamp(ts as u64),
                (VSS_SOURCE_TS, Some(Value::Uint64(ts))) => assembler.source_timestamp(ts),
                (path, Some(Value::Float(f)))  => events.extend(assembler.value(path, f)),
                (path, Some(Value::Double(d))) => events.extend(assembler.value(path, d as f32)),
                _ => {}
            }
        }

        for event in events {
            info!(
                "[VssBridge] ts={} ms TempMax={:.1} TempAvg={:.1} SoC={:.0}%",
                event.timestamp_ms, event.temp_max, event.temp_avg, event.soc
            );

            let transport = Arc::clone(&transport);
            let _ = publish_json_event(transport.clone(), vss_battery_temp_uri(), &event).await;

            if event.temp_max > HIGH_TEMP_THRESHOLD {
                let alert = HighTempAlert {
                    value: event.temp_max,
                    severity: "WARNING".to_string(),
                    unit: "degC".to_string(),
                    source: "kuksa".to_string(),
                    timestamp_ms: event.timestamp_ms,
                };
                let _ = publish_json_event(transport, vss_battery_high_temp_uri(), &alert).await;
            }
        }
    }

    warn!("[VssBridge] Stream ended");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_frame(a: &mut SampleAssembler, ts: u64, temp: f32, soc: f32) -> Option<BatteryTempEvent> {
        a.source_timestamp(ts);
        assert!(a.value(VSS_TEMP_AVG, temp).is_none());
        assert!(a.value(VSS_TEMP_MAX, temp + 1.0).is_none());
        assert!(a.value(VSS_TEMP_MIN, temp - 1.0).is_none());
        a.value(VSS_SOC, soc)
    }

    #[test]
    fn complete_frame_uses_source_timestamp() {
        let mut a = SampleAssembler::default();
        let e = feed_frame(&mut a, 1200, 30.0, 80.5).expect("complete sample");
        assert_eq!(e.timestamp_ms, 1200);
        assert_eq!((e.temp_avg, e.temp_max, e.temp_min, e.soc), (30.0, 31.0, 29.0, 80.5));
    }

    #[test]
    fn one_event_per_frame() {
        let mut a = SampleAssembler::default();
        assert_eq!(feed_frame(&mut a, 0, 25.0, 90.0).unwrap().timestamp_ms, 0);
        assert_eq!(feed_frame(&mut a, 100, 25.5, 90.0).unwrap().timestamp_ms, 100);
        // Late value after completion has no open sample.
        assert!(a.value(VSS_SOC, 1.0).is_none());
    }

    #[test]
    fn incomplete_frame_is_dropped() {
        let mut a = SampleAssembler::default();
        a.source_timestamp(0);
        a.value(VSS_TEMP_AVG, 25.0);
        a.value(VSS_TEMP_MAX, 26.0);
        // Next frame starts before Min/SoC arrived: values must not leak into it.
        a.source_timestamp(100);
        assert!(a.value(VSS_TEMP_MIN, 24.0).is_none());
        assert!(a.value(VSS_SOC, 90.0).is_none());
        assert!(a.value(VSS_TEMP_AVG, 25.5).is_none());
        assert_eq!(a.value(VSS_TEMP_MAX, 26.5).unwrap().timestamp_ms, 100);
    }

    #[test]
    fn values_before_first_timestamp_are_ignored() {
        let mut a = SampleAssembler::default();
        assert!(a.value(VSS_TEMP_AVG, 25.0).is_none());
        assert!(a.pending.is_none());
    }

    #[test]
    fn repeated_timestamp_still_opens_new_sample() {
        let mut a = SampleAssembler::default();
        assert!(feed_frame(&mut a, 500, 25.0, 90.0).is_some());
        assert_eq!(feed_frame(&mut a, 500, 25.0, 90.0).unwrap().timestamp_ms, 500);
    }
}
