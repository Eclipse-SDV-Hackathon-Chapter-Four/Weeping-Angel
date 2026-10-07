use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::guardian_config::GuardianConfig;
use crate::guardian_model::{
    evaluate_generation_gap, evaluate_sample, BatterySample, Detection, DetectionClass,
    DetectionKey, DetectionLevel, StuckDetector,
};

#[derive(Debug, Default)]
pub struct GuardianRuntime {
    pending_samples: VecDeque<BatterySample>,
    latest_sample: Option<BatterySample>,
    previous_sample: Option<BatterySample>,
    /// Highest source timestamp evaluated so far; baseline of the gap check.
    last_timestamp_ms: Option<u64>,
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
    /// Samples are queued so that every received sample is evaluated exactly once,
    /// even when several arrive within one evaluation period.
    pub fn receive_sample(&mut self, sample: BatterySample) {
        self.latest_sample_generation = self
            .latest_sample_generation
            .checked_add(1)
            .expect("Guardian sample generation counter overflowed");
        self.last_receive_time = Some(sample.received_at);
        self.latest_sample = Some(sample.clone());
        self.pending_samples.push_back(sample);
    }

    /// Execute exactly one periodic Guardian cycle.
    pub fn cycle(&mut self, now: Instant, config: &GuardianConfig) -> Vec<Detection> {
        let Some(last_receive_time) = self.last_receive_time else {
            return Vec::new();
        };

        let mut transitions = Vec::new();
        while let Some(sample) = self.pending_samples.pop_front() {
            transitions.extend(self.evaluate(sample, config, now));
        }
        self.last_evaluated_generation = self.latest_sample_generation;

        let age = now
            .checked_duration_since(last_receive_time)
            .unwrap_or_default();
        if age > config.missing_packet_timeout() {
            if !self.stream_stale {
                self.stream_stale = true;
                let observed_ms = age.as_secs_f32() * 1_000.0;
                let limit_ms = config.missing_packet_timeout().as_secs_f32() * 1_000.0;
                let mut stale = Detection::triggered(
                    DetectionClass::StreamStale,
                    DetectionLevel::Violation,
                    None,
                    Some(observed_ms),
                    Some(limit_ms),
                    Some(observed_ms - limit_ms),
                    None,
                    now,
                );
                stale.sample_timestamp_ms = self
                    .last_timestamp_ms
                    .map(|last| last + projected_age_ms(age, config));
                transitions.push(stale);
            }
        } else if self.stream_stale {
            self.stream_stale = false;
            let mut cleared = Detection::cleared(
                DetectionKey {
                    class: DetectionClass::StreamStale,
                    level: DetectionLevel::Violation,
                    signal: None,
                },
                now,
            );
            cleared.sample_timestamp_ms = self.last_timestamp_ms;
            transitions.push(cleared);
        }
        transitions
    }

    fn evaluate(
        &mut self,
        current: BatterySample,
        config: &GuardianConfig,
        now: Instant,
    ) -> Vec<Detection> {
        // A source timeline that restarts behind the last timestamp after a
        // silence is a replay/source restart, not a gap: start a new baseline.
        if self.stream_stale
            && self
                .last_timestamp_ms
                .is_some_and(|last| current.timestamp_ms < last)
        {
            self.previous_sample = None;
            self.last_timestamp_ms = None;
            self.stuck_detector = StuckDetector::default();
        }

        let mut current_detections =
            evaluate_sample(&current, self.previous_sample.as_ref(), config, now);
        current_detections.extend(evaluate_generation_gap(
            current.timestamp_ms,
            self.last_timestamp_ms,
            config,
            now,
        ));
        current_detections.extend(self.stuck_detector.evaluate(&current, config, now));
        let mut transitions = self.model_transitions(current_detections, now);
        for transition in &mut transitions {
            transition.sample_timestamp_ms = Some(current.timestamp_ms);
        }

        self.last_timestamp_ms = Some(
            self.last_timestamp_ms
                .map_or(current.timestamp_ms, |last| last.max(current.timestamp_ms)),
        );
        self.previous_sample = Some(current);
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

/// Receive age projected onto the source timeline (ADR-013): rounded up to
/// whole evaluation periods, since staleness is decided once per cycle. A
/// stale fault has no causing sample, so it is placed at the last source
/// timestamp plus this age instead of at the last sample itself.
fn projected_age_ms(age: std::time::Duration, config: &GuardianConfig) -> u64 {
    let period_ms = config.evaluation_period().as_millis().max(1) as u64;
    (age.as_millis() as u64).div_ceil(period_ms) * period_ms
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;
    use std::time::Duration;

    use super::*;

    fn config() -> GuardianConfig {
        GuardianConfig::from_yaml_str(include_str!(
            "../../../config/battery_guardian/guardian_model.yaml"
        ))
        .expect("test configuration")
    }

    /// Origin of the relative time base; samples are generated without
    /// transport delay unless a test passes a later receive instant.
    fn origin() -> Instant {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        *ORIGIN.get_or_init(Instant::now)
    }

    fn source_ms(at: Instant) -> u64 {
        at.duration_since(origin()).as_millis() as u64
    }

    fn nominal(at: Instant) -> BatterySample {
        BatterySample::new(18.0, 20.0, 22.0, 50.0, source_ms(at), at)
    }

    fn generated(timestamp_ms: u64, received_at: Instant) -> BatterySample {
        BatterySample::new(18.0, 20.0, 22.0, 50.0, timestamp_ms, received_at)
    }

    fn thermal_sample(at: Instant, temp_max: f32) -> BatterySample {
        BatterySample::new(
            temp_max - 2.0,
            temp_max - 1.0,
            temp_max,
            50.0,
            source_ms(at),
            at,
        )
    }

    fn gap_transitions(detections: &[Detection]) -> Vec<bool> {
        detections
            .iter()
            .filter(|detection| detection.class == DetectionClass::StreamGenerationGap)
            .map(|detection| detection.active)
            .collect()
    }

    fn thermal_transitions(detections: &[Detection]) -> Vec<(DetectionLevel, bool)> {
        detections
            .iter()
            .filter(|detection| detection.class == DetectionClass::ThermalLimit)
            .map(|detection| (detection.level, detection.active))
            .collect()
    }

    #[test]
    fn no_sample_means_no_checks_and_no_stale_fault() {
        let mut runtime = GuardianRuntime::new();
        assert!(runtime.cycle(origin(), &config()).is_empty());
    }

    #[test]
    fn missing_packet_timeout_is_strict_and_transition_based() {
        let base = origin();
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
        assert_eq!(first[0].level, DetectionLevel::Violation);
        assert!(first[0].active);

        assert!(runtime
            .cycle(stale_at + cfg.evaluation_period(), &cfg)
            .is_empty());
    }

    #[test]
    fn stale_is_placed_at_the_projected_source_time() {
        let base = origin();
        let cfg = config();
        // Detected at the first cycle after the timeout, wherever that cycle
        // falls relative to the last arrival: always 600 ms on the source time.
        for late_ms in [501, 570, 600] {
            let mut rt = GuardianRuntime::new();
            rt.receive_sample(generated(7_900, base));
            rt.cycle(base, &cfg);
            let stale = rt.cycle(base + Duration::from_millis(late_ms), &cfg);
            assert_eq!(stale[0].class, DetectionClass::StreamStale);
            assert_eq!(stale[0].sample_timestamp_ms, Some(8_500), "detected after {late_ms} ms");
        }
    }

    #[test]
    fn fresh_sample_clears_stale_state() {
        let base = origin();
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
        let base = origin();
        let cfg = config();
        let mut runtime = GuardianRuntime::new();
        runtime.receive_sample(BatterySample::new(
            cfg.temperature.absolute_min_c - 1.0,
            20.0,
            22.0,
            50.0,
            0,
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

    #[test]
    fn thermal_state_transitions_are_mutually_exclusive() {
        let base = origin();
        let cfg = config();
        let mut runtime = GuardianRuntime::new();

        runtime.receive_sample(thermal_sample(base, 59.0));
        assert!(thermal_transitions(&runtime.cycle(base, &cfg)).is_empty());

        let warning_at = base + cfg.evaluation_period();
        runtime.receive_sample(thermal_sample(warning_at, 60.0));
        assert_eq!(
            thermal_transitions(&runtime.cycle(warning_at, &cfg)),
            vec![(DetectionLevel::Warning, true)]
        );

        let critical_at = warning_at + cfg.evaluation_period();
        runtime.receive_sample(thermal_sample(critical_at, 70.0));
        let critical = thermal_transitions(&runtime.cycle(critical_at, &cfg));
        assert!(critical.contains(&(DetectionLevel::Warning, false)));
        assert!(critical.contains(&(DetectionLevel::Critical, true)));
        assert_eq!(critical.len(), 2);

        let warning_again_at = critical_at + cfg.evaluation_period();
        runtime.receive_sample(thermal_sample(warning_again_at, 69.0));
        let warning_again = thermal_transitions(&runtime.cycle(warning_again_at, &cfg));
        assert!(warning_again.contains(&(DetectionLevel::Critical, false)));
        assert!(warning_again.contains(&(DetectionLevel::Warning, true)));
        assert_eq!(warning_again.len(), 2);

        let normal_at = warning_again_at + cfg.evaluation_period();
        runtime.receive_sample(thermal_sample(normal_at, 59.0));
        assert_eq!(
            thermal_transitions(&runtime.cycle(normal_at, &cfg)),
            vec![(DetectionLevel::Warning, false)]
        );
    }

    #[test]
    fn every_sample_of_a_burst_is_evaluated_without_gap() {
        let base = origin();
        let cfg = config();
        let mut runtime = GuardianRuntime::new();
        runtime.receive_sample(generated(0, base));
        runtime.cycle(base, &cfg);

        // Two generations arrive within one evaluation period.
        let burst_at = base + Duration::from_millis(201);
        runtime.receive_sample(generated(100, burst_at));
        runtime.receive_sample(generated(200, burst_at));
        let transitions = runtime.cycle(burst_at, &cfg);
        assert!(gap_transitions(&transitions).is_empty());
        assert!(!transitions
            .iter()
            .any(|detection| detection.class == DetectionClass::PhysicalTempRate));
        assert_eq!(
            runtime.latest_sample_generation(),
            runtime.last_evaluated_generation()
        );
    }

    #[test]
    fn lost_generation_fails_and_next_regular_sample_passes() {
        let base = origin();
        let cfg = config();
        let mut runtime = GuardianRuntime::new();
        runtime.receive_sample(generated(0, base));
        runtime.cycle(base, &cfg);

        // Generation 100 is lost.
        let after_loss = base + Duration::from_millis(200);
        runtime.receive_sample(generated(200, after_loss));
        let failed = runtime.cycle(after_loss, &cfg);
        assert_eq!(gap_transitions(&failed), vec![true]);
        let gap = failed
            .iter()
            .find(|detection| detection.class == DetectionClass::StreamGenerationGap)
            .expect("gap detection");
        assert_eq!(gap.interval_ms, Some(200));
        assert_eq!(gap.sample_timestamp_ms, Some(200));

        let regular = base + Duration::from_millis(300);
        runtime.receive_sample(generated(300, regular));
        assert_eq!(gap_transitions(&runtime.cycle(regular, &cfg)), vec![false]);
    }

    #[test]
    fn sustained_loss_is_stale_and_a_generation_gap() {
        let base = origin();
        let cfg = config();
        let mut runtime = GuardianRuntime::new();
        runtime.receive_sample(generated(0, base));
        runtime.cycle(base, &cfg);

        let silent = base + cfg.missing_packet_timeout() + Duration::from_millis(1);
        assert!(runtime
            .cycle(silent, &cfg)
            .iter()
            .any(|detection| detection.class == DetectionClass::StreamStale && detection.active));

        let resumed = base + Duration::from_millis(2_000);
        runtime.receive_sample(generated(2_000, resumed));
        let transitions = runtime.cycle(resumed, &cfg);
        assert_eq!(gap_transitions(&transitions), vec![true]);
        assert!(transitions.iter().any(|detection| {
            detection.class == DetectionClass::StreamStale && !detection.active
        }));
    }

    #[test]
    fn source_restart_after_silence_is_not_a_gap() {
        let base = origin();
        let cfg = config();
        let mut runtime = GuardianRuntime::new();
        for (index, timestamp_ms) in [0, 100, 200].into_iter().enumerate() {
            let at = base + Duration::from_millis(index as u64 * 100);
            runtime.receive_sample(generated(timestamp_ms, at));
            runtime.cycle(at, &cfg);
        }
        let silent = base + Duration::from_millis(200) + cfg.missing_packet_timeout();
        runtime.cycle(silent + Duration::from_millis(1), &cfg);
        assert!(runtime.stream_is_stale());

        // Replay restarts at source time 0 and continues on the nominal grid.
        let restart = silent + Duration::from_millis(100);
        runtime.receive_sample(generated(0, restart));
        let restarted = runtime.cycle(restart, &cfg);
        assert!(gap_transitions(&restarted).is_empty());
        assert!(!runtime.stream_is_stale());

        let next = restart + Duration::from_millis(100);
        runtime.receive_sample(generated(100, next));
        assert!(gap_transitions(&runtime.cycle(next, &cfg)).is_empty());
    }
}
