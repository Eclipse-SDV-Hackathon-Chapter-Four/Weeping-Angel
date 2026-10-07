use std::collections::VecDeque;
use std::time::Instant;

use crate::guardian_config::{GuardianConfig, ThermalLimitConfig};

#[derive(Debug, Clone)]
pub struct BatterySample {
    pub temp_min: f32,
    pub temp_avg: f32,
    pub temp_max: f32,
    pub soc: f32,
    pub received_at: Instant,
}

impl BatterySample {
    pub fn new(
        temp_min: f32,
        temp_avg: f32,
        temp_max: f32,
        soc: f32,
        received_at: Instant,
    ) -> Self {
        Self {
            temp_min,
            temp_avg,
            temp_max,
            soc,
            received_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetectionClass {
    StreamStale,
    ThermalLimit,
    PhysicalTempAbsoluteLimit,
    PhysicalTempOrdering,
    PhysicalTempSpread,
    PhysicalTempHotspot,
    PhysicalTempRate,
    PhysicalSocRange,
    PhysicalSocRate,
    SignalStuck,
}

impl DetectionClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StreamStale => "STREAM_STALE",
            Self::ThermalLimit => "THERMAL_LIMIT",
            Self::PhysicalTempAbsoluteLimit => "PHYSICAL_TEMP_ABSOLUTE_LIMIT",
            Self::PhysicalTempOrdering => "PHYSICAL_TEMP_ORDERING",
            Self::PhysicalTempSpread => "PHYSICAL_TEMP_SPREAD",
            Self::PhysicalTempHotspot => "PHYSICAL_TEMP_HOTSPOT",
            Self::PhysicalTempRate => "PHYSICAL_TEMP_RATE",
            Self::PhysicalSocRange => "PHYSICAL_SOC_RANGE",
            Self::PhysicalSocRate => "PHYSICAL_SOC_RATE",
            Self::SignalStuck => "SIGNAL_STUCK",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetectionLevel {
    Warning,
    Violation,
    Critical,
}

impl DetectionLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "WARNING",
            Self::Violation => "VIOLATION",
            Self::Critical => "CRITICAL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signal {
    TempMin,
    TempAvg,
    TempMax,
    Soc,
}

impl Signal {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TempMin => "temp_min",
            Self::TempAvg => "temp_avg",
            Self::TempMax => "temp_max",
            Self::Soc => "soc",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Detection {
    pub class: DetectionClass,
    pub level: DetectionLevel,
    pub signal: Option<Signal>,
    pub observed: Option<f32>,
    pub limit: Option<f32>,
    pub residual: Option<f32>,
    pub utilization: Option<f32>,
    pub detected_at: Instant,
    pub active: bool,
}

impl Detection {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn triggered(
        class: DetectionClass,
        level: DetectionLevel,
        signal: Option<Signal>,
        observed: Option<f32>,
        limit: Option<f32>,
        residual: Option<f32>,
        utilization: Option<f32>,
        detected_at: Instant,
    ) -> Self {
        Self {
            class,
            level,
            signal,
            observed,
            limit,
            residual,
            utilization,
            detected_at,
            active: true,
        }
    }

    pub(crate) fn cleared(key: DetectionKey, detected_at: Instant) -> Self {
        Self {
            class: key.class,
            level: key.level,
            signal: key.signal,
            observed: None,
            limit: None,
            residual: None,
            utilization: None,
            detected_at,
            active: false,
        }
    }

    pub(crate) fn key(&self) -> DetectionKey {
        DetectionKey {
            class: self.class,
            level: self.level,
            signal: self.signal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DetectionKey {
    pub class: DetectionClass,
    pub level: DetectionLevel,
    pub signal: Option<Signal>,
}

pub fn evaluate_sample(
    current: &BatterySample,
    previous: Option<&BatterySample>,
    config: &GuardianConfig,
    now: Instant,
) -> Vec<Detection> {
    let mut detections = Vec::new();
    evaluate_thermal_operating_state(current, config, now, &mut detections);
    evaluate_absolute_temperature(current, config, now, &mut detections);
    evaluate_temperature_ordering(current, now, &mut detections);
    evaluate_spatial_consistency(current, config, now, &mut detections);

    if let Some(previous) = previous {
        evaluate_temperature_rates(current, previous, config, now, &mut detections);
    }

    evaluate_soc_range(current, config, now, &mut detections);

    if let Some(previous) = previous {
        evaluate_soc_step(current, previous, config, now, &mut detections);
    }

    detections
}

fn evaluate_thermal_operating_state(
    sample: &BatterySample,
    config: &GuardianConfig,
    now: Instant,
    detections: &mut Vec<Detection>,
) {
    let temperature = &config.temperature;
    let critical_threshold = temperature.critical_threshold_c();
    let warning_threshold = temperature.warning_threshold_c();
    let (level, limit) = if sample.temp_max >= critical_threshold {
        (DetectionLevel::Critical, critical_threshold)
    } else if sample.temp_max >= warning_threshold {
        (DetectionLevel::Warning, warning_threshold)
    } else {
        return;
    };

    detections.push(Detection::triggered(
        DetectionClass::ThermalLimit,
        level,
        Some(Signal::TempMax),
        Some(sample.temp_max),
        Some(limit),
        Some(sample.temp_max - limit),
        None,
        now,
    ));
}

fn evaluate_absolute_temperature(
    sample: &BatterySample,
    config: &GuardianConfig,
    now: Instant,
    detections: &mut Vec<Detection>,
) {
    let temperature = &config.temperature;
    if sample.temp_min < temperature.absolute_min_c {
        detections.push(Detection::triggered(
            DetectionClass::PhysicalTempAbsoluteLimit,
            DetectionLevel::Violation,
            Some(Signal::TempMin),
            Some(sample.temp_min),
            Some(temperature.absolute_min_c),
            Some(temperature.absolute_min_c - sample.temp_min),
            None,
            now,
        ));
    }
    if sample.temp_max > temperature.absolute_max_c {
        detections.push(Detection::triggered(
            DetectionClass::PhysicalTempAbsoluteLimit,
            DetectionLevel::Violation,
            Some(Signal::TempMax),
            Some(sample.temp_max),
            Some(temperature.absolute_max_c),
            Some(sample.temp_max - temperature.absolute_max_c),
            None,
            now,
        ));
    }
}

fn evaluate_temperature_ordering(
    sample: &BatterySample,
    now: Instant,
    detections: &mut Vec<Detection>,
) {
    if sample.temp_min > sample.temp_avg {
        detections.push(Detection::triggered(
            DetectionClass::PhysicalTempOrdering,
            DetectionLevel::Violation,
            Some(Signal::TempMin),
            Some(sample.temp_min),
            Some(sample.temp_avg),
            Some(sample.temp_min - sample.temp_avg),
            None,
            now,
        ));
    }
    if sample.temp_avg > sample.temp_max {
        detections.push(Detection::triggered(
            DetectionClass::PhysicalTempOrdering,
            DetectionLevel::Violation,
            Some(Signal::TempAvg),
            Some(sample.temp_avg),
            Some(sample.temp_max),
            Some(sample.temp_avg - sample.temp_max),
            None,
            now,
        ));
    }
}

fn evaluate_spatial_consistency(
    sample: &BatterySample,
    config: &GuardianConfig,
    now: Instant,
    detections: &mut Vec<Detection>,
) {
    let theta = thermal_state(sample.temp_avg, config);
    let spread_limit = thermal_limit(&config.temperature.spread, theta);
    let observed_spread = sample.temp_max - sample.temp_min;
    evaluate_continuous_upper_bound(
        DetectionClass::PhysicalTempSpread,
        None,
        observed_spread,
        spread_limit,
        config,
        now,
        detections,
    );

    let hotspot_limit = thermal_limit(&config.temperature.hotspot, theta);
    let observed_hotspot = sample.temp_max - sample.temp_avg;
    evaluate_continuous_upper_bound(
        DetectionClass::PhysicalTempHotspot,
        Some(Signal::TempMax),
        observed_hotspot,
        hotspot_limit,
        config,
        now,
        detections,
    );
}

#[allow(clippy::too_many_arguments)]
fn evaluate_continuous_upper_bound(
    class: DetectionClass,
    signal: Option<Signal>,
    observed: f32,
    limit: f32,
    config: &GuardianConfig,
    now: Instant,
    detections: &mut Vec<Detection>,
) {
    let utilization = observed / limit;
    let level = if utilization < config.warning.utilization_threshold {
        return;
    } else if utilization <= 1.0 {
        DetectionLevel::Warning
    } else {
        DetectionLevel::Violation
    };
    detections.push(Detection::triggered(
        class,
        level,
        signal,
        Some(observed),
        Some(limit),
        Some(observed - limit),
        Some(utilization),
        now,
    ));
}

fn evaluate_temperature_rates(
    current: &BatterySample,
    previous: &BatterySample,
    config: &GuardianConfig,
    now: Instant,
    detections: &mut Vec<Detection>,
) {
    let Some(elapsed) = current
        .received_at
        .checked_duration_since(previous.received_at)
    else {
        return;
    };
    let elapsed_seconds = elapsed.as_secs_f32();
    if elapsed_seconds <= 0.0 {
        return;
    }

    let dynamics = &config.temperature.dynamics;
    let theta = thermal_state(previous.temp_avg, config);
    let heating = &dynamics.heating_rate_c_per_s;
    let mut heating_limit = heating.cold - (heating.cold - heating.hot) * theta;
    if dynamics.soc_coupling.enabled {
        let soc_rate = ((current.soc - previous.soc) / elapsed_seconds).abs();
        let excitation = soc_rate.min(dynamics.soc_coupling.rate_cap_pp_per_s);
        heating_limit += dynamics.soc_coupling.gain_c_per_pp * excitation;
    }

    let temperatures = [
        (Signal::TempMin, current.temp_min, previous.temp_min),
        (Signal::TempAvg, current.temp_avg, previous.temp_avg),
        (Signal::TempMax, current.temp_max, previous.temp_max),
    ];
    for (signal, current_value, previous_value) in temperatures {
        let rate = (current_value - previous_value) / elapsed_seconds;
        let (observed, limit) = if rate >= 0.0 {
            (rate, heating_limit)
        } else {
            (-rate, dynamics.cooling_rate_c_per_s)
        };
        evaluate_continuous_upper_bound(
            DetectionClass::PhysicalTempRate,
            Some(signal),
            observed,
            limit,
            config,
            now,
            detections,
        );
    }
}

fn evaluate_soc_range(
    sample: &BatterySample,
    config: &GuardianConfig,
    now: Instant,
    detections: &mut Vec<Detection>,
) {
    if sample.soc < config.soc.min_percent {
        detections.push(Detection::triggered(
            DetectionClass::PhysicalSocRange,
            DetectionLevel::Violation,
            Some(Signal::Soc),
            Some(sample.soc),
            Some(config.soc.min_percent),
            Some(config.soc.min_percent - sample.soc),
            None,
            now,
        ));
    }
    if sample.soc > config.soc.max_percent {
        detections.push(Detection::triggered(
            DetectionClass::PhysicalSocRange,
            DetectionLevel::Violation,
            Some(Signal::Soc),
            Some(sample.soc),
            Some(config.soc.max_percent),
            Some(sample.soc - config.soc.max_percent),
            None,
            now,
        ));
    }
}

fn evaluate_soc_step(
    current: &BatterySample,
    previous: &BatterySample,
    config: &GuardianConfig,
    now: Instant,
    detections: &mut Vec<Detection>,
) {
    let observed_step = (current.soc - previous.soc).abs();
    let residual = observed_step - config.soc.max_step_pp;
    if residual > 0.0 {
        detections.push(Detection::triggered(
            DetectionClass::PhysicalSocRate,
            DetectionLevel::Violation,
            Some(Signal::Soc),
            Some(observed_step),
            Some(config.soc.max_step_pp),
            Some(residual),
            None,
            now,
        ));
    }
}

fn thermal_state(temperature: f32, config: &GuardianConfig) -> f32 {
    let temperature_config = &config.temperature;
    ((temperature - temperature_config.reference_c)
        / (temperature_config.hot_state_c - temperature_config.reference_c))
        .clamp(0.0, 1.0)
}

fn thermal_limit(limit: &ThermalLimitConfig, theta: f32) -> f32 {
    limit.cold_c - (limit.cold_c - limit.hot_c) * theta
}

#[derive(Debug, Default)]
pub(crate) struct StuckDetector {
    history: VecDeque<SampleValues>,
}

#[derive(Debug, Clone, Copy)]
struct SampleValues {
    temp_min: f32,
    temp_avg: f32,
    temp_max: f32,
    soc: f32,
}

impl From<&BatterySample> for SampleValues {
    fn from(sample: &BatterySample) -> Self {
        Self {
            temp_min: sample.temp_min,
            temp_avg: sample.temp_avg,
            temp_max: sample.temp_max,
            soc: sample.soc,
        }
    }
}

impl StuckDetector {
    pub(crate) fn evaluate(
        &mut self,
        sample: &BatterySample,
        config: &GuardianConfig,
        now: Instant,
    ) -> Vec<Detection> {
        if !config.stuck.enabled {
            self.history.clear();
            return Vec::new();
        }

        self.history.push_back(sample.into());
        while self.history.len() > config.stuck.window_samples {
            self.history.pop_front();
        }
        if self.history.len() < config.stuck.window_samples {
            return Vec::new();
        }

        let amplitudes = [
            (
                Signal::TempMin,
                amplitude(self.history.iter().map(|sample| sample.temp_min)),
            ),
            (
                Signal::TempAvg,
                amplitude(self.history.iter().map(|sample| sample.temp_avg)),
            ),
            (
                Signal::TempMax,
                amplitude(self.history.iter().map(|sample| sample.temp_max)),
            ),
        ];
        let soc_amplitude = amplitude(self.history.iter().map(|sample| sample.soc));
        let mut detections = Vec::new();

        for (signal, signal_amplitude) in amplitudes {
            let independently_excited = amplitudes.iter().any(|(other, other_amplitude)| {
                *other != signal && *other_amplitude >= config.stuck.temperature_excitation_c
            }) || soc_amplitude >= config.stuck.soc_excitation_pp;
            if signal_amplitude <= config.stuck.flatness_epsilon_c && independently_excited {
                detections.push(Detection::triggered(
                    DetectionClass::SignalStuck,
                    DetectionLevel::Violation,
                    Some(signal),
                    Some(signal_amplitude),
                    Some(config.stuck.flatness_epsilon_c),
                    None,
                    None,
                    now,
                ));
            }
        }

        detections
    }
}

fn amplitude(values: impl Iterator<Item = f32>) -> f32 {
    let (minimum, maximum) = values.fold(
        (f32::INFINITY, f32::NEG_INFINITY),
        |(minimum, maximum), value| (minimum.min(value), maximum.max(value)),
    );
    maximum - minimum
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn config() -> GuardianConfig {
        GuardianConfig::from_yaml_str(include_str!(
            "../../../config/battery_guardian/guardian_model.yaml"
        ))
        .expect("test configuration")
    }

    fn sample(at: Instant, temp_min: f32, temp_avg: f32, temp_max: f32, soc: f32) -> BatterySample {
        BatterySample::new(temp_min, temp_avg, temp_max, soc, at)
    }

    fn nominal(at: Instant) -> BatterySample {
        sample(at, 18.0, 20.0, 22.0, 50.0)
    }

    fn thermal_sample(at: Instant, temp_max: f32) -> BatterySample {
        sample(at, temp_max - 2.0, temp_max - 1.0, temp_max, 50.0)
    }

    fn has(detections: &[Detection], class: DetectionClass) -> bool {
        detections.iter().any(|detection| detection.class == class)
    }

    fn has_level(detections: &[Detection], class: DetectionClass, level: DetectionLevel) -> bool {
        detections
            .iter()
            .any(|detection| detection.class == class && detection.level == level)
    }

    #[test]
    fn nominal_sample_produces_no_detection() {
        let now = Instant::now();
        assert!(evaluate_sample(&nominal(now), None, &config(), now).is_empty());
    }

    #[test]
    fn below_warning_threshold_has_no_thermal_detection() {
        let now = Instant::now();
        let detections = evaluate_sample(&thermal_sample(now, 59.9), None, &config(), now);
        assert!(!has(&detections, DetectionClass::ThermalLimit));
    }

    #[test]
    fn warning_band_boundaries_are_warning_only() {
        let now = Instant::now();
        for temp_max in [60.0, 69.9] {
            let detections = evaluate_sample(&thermal_sample(now, temp_max), None, &config(), now);
            assert!(has_level(
                &detections,
                DetectionClass::ThermalLimit,
                DetectionLevel::Warning
            ));
            assert!(!has_level(
                &detections,
                DetectionClass::ThermalLimit,
                DetectionLevel::Critical
            ));
        }
    }

    #[test]
    fn critical_boundary_is_not_an_absolute_limit_violation() {
        let now = Instant::now();
        let detections = evaluate_sample(&thermal_sample(now, 70.0), None, &config(), now);
        assert!(!has_level(
            &detections,
            DetectionClass::ThermalLimit,
            DetectionLevel::Warning
        ));
        assert!(has_level(
            &detections,
            DetectionClass::ThermalLimit,
            DetectionLevel::Critical
        ));
        assert!(!has(&detections, DetectionClass::PhysicalTempAbsoluteLimit));
    }

    #[test]
    fn above_absolute_maximum_is_critical_and_a_physical_violation() {
        let now = Instant::now();
        let detections = evaluate_sample(&thermal_sample(now, 70.1), None, &config(), now);
        assert!(!has_level(
            &detections,
            DetectionClass::ThermalLimit,
            DetectionLevel::Warning
        ));
        assert!(has_level(
            &detections,
            DetectionClass::ThermalLimit,
            DetectionLevel::Critical
        ));
        assert!(has(&detections, DetectionClass::PhysicalTempAbsoluteLimit));
    }

    #[test]
    fn detects_absolute_minimum_violation() {
        let now = Instant::now();
        let cfg = config();
        let current = sample(now, cfg.temperature.absolute_min_c - 0.5, 20.0, 22.0, 50.0);
        let detections = evaluate_sample(&current, None, &cfg, now);
        assert!(detections.iter().any(|detection| {
            detection.class == DetectionClass::PhysicalTempAbsoluteLimit
                && detection.signal == Some(Signal::TempMin)
        }));
    }

    #[test]
    fn detects_absolute_maximum_violation() {
        let now = Instant::now();
        let cfg = config();
        let current = sample(now, 20.0, 22.0, cfg.temperature.absolute_max_c + 0.5, 50.0);
        let detections = evaluate_sample(&current, None, &cfg, now);
        assert!(detections.iter().any(|detection| {
            detection.class == DetectionClass::PhysicalTempAbsoluteLimit
                && detection.signal == Some(Signal::TempMax)
        }));
    }

    #[test]
    fn detects_invalid_temperature_ordering() {
        let now = Instant::now();
        let current = sample(now, 25.0, 20.0, 19.0, 50.0);
        let detections = evaluate_sample(&current, None, &config(), now);
        assert_eq!(
            detections
                .iter()
                .filter(|detection| detection.class == DetectionClass::PhysicalTempOrdering)
                .count(),
            2
        );
    }

    #[test]
    fn spread_warning_and_violation_boundaries_include_utilization() {
        let now = Instant::now();
        let mut cfg = config();
        cfg.temperature.spread.cold_c = 10.0;
        cfg.temperature.spread.hot_c = 10.0;
        let spread = 10.0;
        for (utilization, expected_level) in [
            (0.79, None),
            (0.8, Some(DetectionLevel::Warning)),
            (1.0, Some(DetectionLevel::Warning)),
            (1.01, Some(DetectionLevel::Violation)),
        ] {
            let observed = spread * utilization;
            let current = sample(
                now,
                20.0 - observed / 2.0,
                20.0,
                20.0 + observed / 2.0,
                50.0,
            );
            let detections = evaluate_sample(&current, None, &cfg, now);
            let detection = detections
                .iter()
                .find(|detection| detection.class == DetectionClass::PhysicalTempSpread);
            assert_eq!(detection.map(|detection| detection.level), expected_level);
            if let Some(detection) = detection {
                assert!((detection.utilization.expect("utilization") - utilization).abs() < 0.001);
                assert!(
                    (detection.residual.expect("residual") - (observed - spread)).abs() < 0.001
                );
            }
        }
    }

    #[test]
    fn hotspot_at_limit_is_a_warning() {
        let now = Instant::now();
        let cfg = config();
        let hotspot = cfg.temperature.hotspot.cold_c;
        let current = sample(now, 15.0, 20.0, 20.0 + hotspot, 50.0);
        let detections = evaluate_sample(&current, None, &cfg, now);
        assert!(has_level(
            &detections,
            DetectionClass::PhysicalTempHotspot,
            DetectionLevel::Warning
        ));
    }

    #[test]
    fn hotspot_above_limit_is_detected() {
        let now = Instant::now();
        let cfg = config();
        let hotspot = cfg.temperature.hotspot.cold_c;
        let current = sample(now, 15.0, 20.0, 20.1 + hotspot, 50.0);
        let detections = evaluate_sample(&current, None, &cfg, now);
        assert!(has_level(
            &detections,
            DetectionClass::PhysicalTempHotspot,
            DetectionLevel::Violation
        ));
    }

    #[test]
    fn positive_temperature_rate_at_limit_is_a_warning() {
        let base = Instant::now();
        let cfg = config();
        let previous = nominal(base);
        let step = cfg.temperature.dynamics.heating_rate_c_per_s.cold;
        let current = sample(
            base + Duration::from_secs(1),
            18.0 + step,
            20.0 + step,
            22.0 + step,
            50.0,
        );
        let detections = evaluate_sample(&current, Some(&previous), &cfg, current.received_at);
        assert!(has_level(
            &detections,
            DetectionClass::PhysicalTempRate,
            DetectionLevel::Warning
        ));
    }

    #[test]
    fn excessive_positive_temperature_rate_is_detected() {
        let base = Instant::now();
        let cfg = config();
        let previous = nominal(base);
        let step = cfg.temperature.dynamics.heating_rate_c_per_s.cold + 0.5;
        let current = sample(
            base + Duration::from_secs(1),
            18.0 + step,
            20.0 + step,
            22.0 + step,
            50.0,
        );
        let detections = evaluate_sample(&current, Some(&previous), &cfg, current.received_at);
        assert!(has_level(
            &detections,
            DetectionClass::PhysicalTempRate,
            DetectionLevel::Violation
        ));
    }

    #[test]
    fn cooling_rate_uses_positive_magnitude_for_warning_and_violation() {
        let base = Instant::now();
        let cfg = config();
        let previous = nominal(base);
        let limit = cfg.temperature.dynamics.cooling_rate_c_per_s;
        let at_limit = sample(
            base + Duration::from_secs(1),
            18.0 - limit,
            20.0 - limit,
            22.0 - limit,
            50.0,
        );
        let boundary = evaluate_sample(&at_limit, Some(&previous), &cfg, at_limit.received_at);
        assert!(has_level(
            &boundary,
            DetectionClass::PhysicalTempRate,
            DetectionLevel::Warning
        ));
        assert!(boundary
            .iter()
            .filter(|detection| detection.class == DetectionClass::PhysicalTempRate)
            .all(|detection| {
                detection.observed == Some(limit)
                    && detection.limit == Some(limit)
                    && detection.utilization == Some(1.0)
            }));
        let excessive = sample(
            base + Duration::from_secs(1),
            17.5 - limit,
            19.5 - limit,
            21.5 - limit,
            50.0,
        );
        assert!(has_level(
            &evaluate_sample(&excessive, Some(&previous), &cfg, excessive.received_at),
            DetectionClass::PhysicalTempRate,
            DetectionLevel::Violation
        ));
    }

    #[test]
    fn detects_soc_below_and_above_range() {
        let now = Instant::now();
        let cfg = config();
        let below = sample(now, 18.0, 20.0, 22.0, cfg.soc.min_percent - 0.5);
        let above = sample(now, 18.0, 20.0, 22.0, cfg.soc.max_percent + 0.5);
        assert!(has(
            &evaluate_sample(&below, None, &cfg, now),
            DetectionClass::PhysicalSocRange
        ));
        assert!(has(
            &evaluate_sample(&above, None, &cfg, now),
            DetectionClass::PhysicalSocRange
        ));
    }

    #[test]
    fn soc_step_boundary_is_valid_and_excess_is_detected() {
        let base = Instant::now();
        let cfg = config();
        let previous = nominal(base);
        let legal = sample(
            base + Duration::from_secs(1),
            18.0,
            20.0,
            22.0,
            50.0 + cfg.soc.max_step_pp,
        );
        assert!(!has(
            &evaluate_sample(&legal, Some(&previous), &cfg, legal.received_at),
            DetectionClass::PhysicalSocRate
        ));
        let excessive = sample(
            base + Duration::from_secs(1),
            18.0,
            20.0,
            22.0,
            50.1 + cfg.soc.max_step_pp,
        );
        assert!(has(
            &evaluate_sample(&excessive, Some(&previous), &cfg, excessive.received_at),
            DetectionClass::PhysicalSocRate
        ));
    }

    #[test]
    fn first_sample_does_not_trigger_temporal_checks() {
        let now = Instant::now();
        let current = sample(now, 18.0, 20.0, 22.0, 50.0);
        let detections = evaluate_sample(&current, None, &config(), now);
        assert!(!has(&detections, DetectionClass::PhysicalTempRate));
        assert!(!has(&detections, DetectionClass::PhysicalSocRate));
    }

    #[test]
    fn flat_temperature_without_excitation_is_not_stuck() {
        let base = Instant::now();
        let cfg = config();
        let mut detector = StuckDetector::default();
        let mut detections = Vec::new();
        for index in 0..cfg.stuck.window_samples {
            let current = nominal(base + Duration::from_millis(index as u64));
            detections = detector.evaluate(&current, &cfg, current.received_at);
        }
        assert!(!has(&detections, DetectionClass::SignalStuck));
    }

    #[test]
    fn flat_temperature_with_independent_excitation_is_stuck() {
        let base = Instant::now();
        let cfg = config();
        let mut detector = StuckDetector::default();
        let mut detections = Vec::new();
        let denominator = (cfg.stuck.window_samples - 1) as f32;
        for index in 0..cfg.stuck.window_samples {
            let excitation = cfg.stuck.temperature_excitation_c * index as f32 / denominator;
            let current = sample(
                base + Duration::from_millis(index as u64),
                18.0,
                20.0 + excitation,
                22.0 + excitation,
                50.0,
            );
            detections = detector.evaluate(&current, &cfg, current.received_at);
        }
        assert!(detections.iter().any(|detection| {
            detection.class == DetectionClass::SignalStuck
                && detection.signal == Some(Signal::TempMin)
        }));
    }
}
