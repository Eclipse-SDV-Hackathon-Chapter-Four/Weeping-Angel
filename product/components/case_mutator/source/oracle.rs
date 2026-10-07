use std::time::{Duration, Instant};

use battery_guardian::{BatterySample, Detection, GuardianConfig, GuardianRuntime};
use serde::Serialize;

use crate::asc::CaseFrame;
use crate::request::ObservationSpec;

#[derive(Debug, Clone, Serialize)]
pub struct PredictedObservation {
    pub class: String,
    pub level: String,
    pub state: String,
    pub signal: Option<String>,
    pub observed: Option<f32>,
    pub limit: Option<f32>,
    pub residual: Option<f32>,
    pub utilization: Option<f32>,
    pub predicted_at_ms: u64,
    pub source_timestamp_ms: Option<u64>,
}

impl PredictedObservation {
    pub fn specification(&self) -> ObservationSpec {
        ObservationSpec {
            class: self.class.clone(),
            level: self.level.clone(),
        }
    }
}

pub fn evaluate_trajectory(
    frames: &[CaseFrame],
    config: &GuardianConfig,
    simulation_end_ms: u64,
) -> Vec<PredictedObservation> {
    let visible: Vec<&CaseFrame> = frames.iter().filter(|frame| !frame.removed).collect();
    if visible.is_empty() {
        return Vec::new();
    }

    let origin_ms = visible[0].arrival_ms;
    let base = Instant::now();
    let period_ms = config.evaluation_period_ms;
    let mut runtime = GuardianRuntime::new();
    let mut next_cycle_ms = origin_ms;
    let mut latest_source_ms = None;
    let mut observations = Vec::new();

    for frame in visible {
        while next_cycle_ms < frame.arrival_ms {
            let now = instant_at(base, origin_ms, next_cycle_ms);
            collect(
                runtime.cycle(now, config),
                next_cycle_ms,
                latest_source_ms,
                &mut observations,
            );
            next_cycle_ms = next_cycle_ms.saturating_add(period_ms);
        }

        let received_at = instant_at(base, origin_ms, frame.arrival_ms);
        runtime.receive_sample(BatterySample::new(
            frame.values.temp_min,
            frame.values.temp_avg,
            frame.values.temp_max,
            frame.values.soc,
            received_at,
        ));
        latest_source_ms = Some(frame.source_ms);

        if next_cycle_ms == frame.arrival_ms {
            collect(
                runtime.cycle(received_at, config),
                next_cycle_ms,
                latest_source_ms,
                &mut observations,
            );
            next_cycle_ms = next_cycle_ms.saturating_add(period_ms);
        }
    }

    while next_cycle_ms <= simulation_end_ms {
        let now = instant_at(base, origin_ms, next_cycle_ms);
        collect(
            runtime.cycle(now, config),
            next_cycle_ms,
            latest_source_ms,
            &mut observations,
        );
        next_cycle_ms = next_cycle_ms.saturating_add(period_ms);
    }

    observations
}

fn instant_at(base: Instant, origin_ms: u64, timestamp_ms: u64) -> Instant {
    base + Duration::from_millis(timestamp_ms.saturating_sub(origin_ms))
}

fn collect(
    detections: Vec<Detection>,
    predicted_at_ms: u64,
    source_timestamp_ms: Option<u64>,
    output: &mut Vec<PredictedObservation>,
) {
    output.extend(detections.into_iter().map(|detection| {
        PredictedObservation {
            class: detection.class.as_str().to_owned(),
            level: detection.level.as_str().to_owned(),
            state: if detection.active {
                "active"
            } else {
                "cleared"
            }
            .to_owned(),
            signal: detection.signal.map(|signal| signal.as_str().to_owned()),
            observed: detection.observed,
            limit: detection.limit,
            residual: detection.residual,
            utilization: detection.utilization,
            predicted_at_ms,
            source_timestamp_ms,
        }
    }));
}
