use std::collections::HashMap;
use std::time::Instant;

use crate::guardian_config::GuardianConfig;
use crate::guardian_model::{
    evaluate_sample, BatterySample, Detection, DetectionClass, DetectionKey, StuckDetector,
};

#[derive(Debug, Default)]
pub struct GuardianRuntime {
    latest_sample: Option<BatterySample>,
    previous_sample: Option<BatterySample>,
    latest_sample_generation: u64,
    last_evaluated_generation: u64,
    last_receive_time: Option<Instant>,
    stuck_detector: StuckDetector,
    active_detections: HashMap<DetectionKey, Detection>,
    stream_stale: bool,
}

impl GuardianRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a valid observation. Model evaluation deliberately happens only in `cycle`.
    pub fn receive_sample(&mut self, sample: BatterySample) {
        self.latest_sample_generation = self
            .latest_sample_generation
            .checked_add(1)
            .expect("Guardian sample generation counter overflowed");
        self.last_receive_time = Some(sample.received_at);
        self.latest_sample = Some(sample);
    }

    /// Execute exactly one periodic Guardian cycle.
    pub fn cycle(&mut self, now: Instant, config: &GuardianConfig) -> Vec<Detection> {
        let Some(last_receive_time) = self.last_receive_time else {
            return Vec::new();
        };

        let age = now
            .checked_duration_since(last_receive_time)
            .unwrap_or_default();
        if age > config.missing_packet_timeout() {
            if self.stream_stale {
                return Vec::new();
            }
            self.stream_stale = true;
            let observed_ms = age.as_secs_f32() * 1_000.0;
            let limit_ms = config.missing_packet_timeout().as_secs_f32() * 1_000.0;
            return vec![Detection::triggered(
                DetectionClass::StreamStale,
                None,
                Some(observed_ms),
                Some(limit_ms),
                Some(observed_ms - limit_ms),
                now,
            )];
        }

        let mut transitions = Vec::new();
        if self.stream_stale {
            self.stream_stale = false;
            transitions.push(Detection::cleared(
                DetectionKey {
                    class: DetectionClass::StreamStale,
                    signal: None,
                },
                now,
            ));
        }

        if self.latest_sample_generation == self.last_evaluated_generation {
            return transitions;
        }

        let current = self
            .latest_sample
            .as_ref()
            .expect("last_receive_time exists only with a latest sample")
            .clone();
        let mut current_detections =
            evaluate_sample(&current, self.previous_sample.as_ref(), config, now);
        current_detections.extend(self.stuck_detector.evaluate(&current, config, now));
        transitions.extend(self.model_transitions(current_detections, now));

        self.previous_sample = Some(current);
        self.last_evaluated_generation = self.latest_sample_generation;
        transitions
    }

    fn model_transitions(&mut self, detections: Vec<Detection>, now: Instant) -> Vec<Detection> {
        let current: HashMap<DetectionKey, Detection> = detections
            .into_iter()
            .map(|detection| (detection.key(), detection))
            .collect();
        let mut transitions = Vec::new();

        for (key, detection) in &current {
            if !self.active_detections.contains_key(key) {
                transitions.push(detection.clone());
            }
        }
        for key in self.active_detections.keys() {
            if !current.contains_key(key) {
                transitions.push(Detection::cleared(*key, now));
            }
        }

        self.active_detections = current;
        transitions
    }

    pub fn latest_sample(&self) -> Option<&BatterySample> {
        self.latest_sample.as_ref()
    }

    pub fn latest_sample_generation(&self) -> u64 {
        self.latest_sample_generation
    }

    pub fn last_evaluated_generation(&self) -> u64 {
        self.last_evaluated_generation
    }

    pub fn stream_is_stale(&self) -> bool {
        self.stream_stale
    }

    pub fn active_detection_count(&self) -> usize {
        self.active_detections.len() + usize::from(self.stream_stale)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn config() -> GuardianConfig {
        GuardianConfig::from_yaml_str(include_str!("../../../config/battery_guardian.yaml"))
            .expect("test configuration")
    }

    fn nominal(at: Instant) -> BatterySample {
        BatterySample::new(18.0, 20.0, 22.0, 50.0, at)
    }

    #[test]
    fn no_sample_means_no_checks_and_no_stale_fault() {
        let mut runtime = GuardianRuntime::new();
        assert!(runtime.cycle(Instant::now(), &config()).is_empty());
    }

    #[test]
    fn missing_packet_timeout_is_strict_and_transition_based() {
        let base = Instant::now();
        let cfg = config();
        let mut runtime = GuardianRuntime::new();
        runtime.receive_sample(nominal(base));
        assert!(runtime.cycle(base, &cfg).is_empty());

        let boundary = base + cfg.missing_packet_timeout();
        assert!(runtime.cycle(boundary, &cfg).is_empty());

        let stale_at = boundary + Duration::from_millis(1);
        let first = runtime.cycle(stale_at, &cfg);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].class, DetectionClass::StreamStale);
        assert!(first[0].active);

        assert!(runtime
            .cycle(stale_at + cfg.evaluation_period(), &cfg)
            .is_empty());
    }

    #[test]
    fn fresh_sample_clears_stale_state() {
        let base = Instant::now();
        let cfg = config();
        let mut runtime = GuardianRuntime::new();
        runtime.receive_sample(nominal(base));
        runtime.cycle(base, &cfg);
        runtime.cycle(
            base + cfg.missing_packet_timeout() + Duration::from_millis(1),
            &cfg,
        );

        let fresh_at = base + cfg.missing_packet_timeout() + Duration::from_millis(2);
        runtime.receive_sample(nominal(fresh_at));
        let transitions = runtime.cycle(fresh_at, &cfg);
        assert!(transitions.iter().any(|detection| {
            detection.class == DetectionClass::StreamStale && !detection.active
        }));
        assert!(!runtime.stream_is_stale());
    }

    #[test]
    fn each_generation_is_evaluated_at_most_once() {
        let base = Instant::now();
        let cfg = config();
        let mut runtime = GuardianRuntime::new();
        runtime.receive_sample(BatterySample::new(
            cfg.temperature.absolute_min_c - 1.0,
            20.0,
            22.0,
            50.0,
            base,
        ));
        let first = runtime.cycle(base, &cfg);
        assert!(first
            .iter()
            .any(|detection| detection.class == DetectionClass::PhysicalTempAbsoluteLimit));
        assert!(runtime
            .cycle(base + cfg.evaluation_period(), &cfg)
            .is_empty());
        assert_eq!(
            runtime.latest_sample_generation(),
            runtime.last_evaluated_generation()
        );
    }
}
