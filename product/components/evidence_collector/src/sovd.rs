// Copyright (c) 2026 Sebastian Russer
// Copyright (c) 2026 Alwin Berger
//
// This program and the accompanying materials are made available under
// the terms of the Eclipse Public License 2.0 which accompanies this
// distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
//
// AI Disclosure: This file was mostly AI-generated.
//
// SPDX-License-Identifier: EPL-2.0 and CC0-1.0
// Assisted-by: Claude Opus 5.5, GLM-5.3-flash
//! OpenSOVD view of the DFM faults (`dfm_sovd_bridge`), polled during a run.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::Message;

/// Fault list of the Guardian's SOVD entity, served by `dfm_sovd_bridge`.
pub const DEFAULT_URL: &str =
    "http://127.0.0.1:7690/sovd/v1/components/battery_guardian/data/faults";
/// Poll period; also the time resolution of OpenSOVD evidence.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// State of one fault code as seen in OpenSOVD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FaultState {
    /// `status.test_failed`: active right now.
    pub active: bool,
    /// `occurrence_counter`: increases on every activation, also for ones
    /// shorter than the poll period.
    pub occurrences: u32,
}

/// One poll result: fault code -> state.
pub type Snapshot = BTreeMap<String, FaultState>;

#[derive(Deserialize)]
struct Response {
    data: Items,
}

#[derive(Deserialize)]
struct Items {
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    code: String,
    #[serde(default)]
    occurrence_counter: Option<u32>,
    status: Status,
}

#[derive(Deserialize)]
struct Status {
    #[serde(default)]
    test_failed: bool,
}

/// Parses `{"id": "faults", "data": {"items": [...]}}`.
pub fn parse(body: &[u8]) -> Result<Snapshot> {
    let response: Response = serde_json::from_slice(body).context("unexpected SOVD fault list")?;
    Ok(response
        .data
        .items
        .into_iter()
        .map(|i| {
            let state = FaultState {
                active: i.status.test_failed,
                occurrences: i.occurrence_counter.unwrap_or(0),
            };
            (i.code, state)
        })
        .collect())
}

/// Polls `url` every [`POLL_INTERVAL`] and forwards each result (or `None`
/// for a failed poll) into the collector's message channel, so polls keep
/// their order relative to the uProtocol messages.
pub fn spawn_poller(
    url: String,
    tx: mpsc::UnboundedSender<Message>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .expect("HTTP client");
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let snapshot = async {
                let body = client
                    .get(&url)
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await?;
                parse(&body)
            }
            .await
            .ok();
            if tx
                .send(Message::Sovd(snapshot, Some(std::time::Instant::now())))
                .is_err()
            {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bridge_fault_list() {
        let body = br#"{"id":"faults","data":{"items":[
            {"aging_counter":0,"code":"BatteryTempAbsoluteLimit","display_code":"BatteryTempAbsoluteLimit",
             "fault_name":"x","healing_counter":0,"occurrence_counter":2,"scope":"ecu","severity":4,
             "status":{"confirmed_dtc":false,"mask":"0x01","pending_dtc":false,"test_failed":true}},
            {"code":"BatteryTempRate","status":{"test_failed":false}}]}}"#;
        let s = parse(body).unwrap();
        assert_eq!(
            s["BatteryTempAbsoluteLimit"],
            FaultState {
                active: true,
                occurrences: 2
            }
        );
        assert_eq!(
            s["BatteryTempRate"],
            FaultState {
                active: false,
                occurrences: 0
            }
        );
        assert!(parse(b"{}").is_err());
    }
}
