use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};

use anyhow::{bail, Context, Result};
use battery_guardian::GuardianConfig;
use serde::Serialize;

use crate::asc::{
    quantize_signal, AscDocument, CaseFrame, SignalValues, SOC_QUANTUM_PP, TEMPERATURE_QUANTUM_C,
};
use crate::oracle::{evaluate_trajectory, PredictedObservation};
use crate::request::{
    Action, GenerationGoal, GenerationRequest, Mutation, MutationParameters, ObservationSpec,
    ResolvedInjection,
};

const MAX_COMBINATIONS: usize = 4_096;

#[derive(Debug)]
pub enum GenerationOutcome {
    Generated(Box<GeneratedCase>),
    Unsatisfiable(Unsatisfiable),
}

#[derive(Debug)]
pub struct GeneratedCase {
    pub asc: String,
    pub ground_truth: GroundTruth,
    pub oracle: OracleSidecar,
}

#[derive(Debug, Serialize)]
pub struct Unsatisfiable {
    pub status: &'static str,
    pub injection_id: String,
    pub reason: UnsatisfiableReason,
}

#[derive(Debug, Serialize)]
pub struct UnsatisfiableReason {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct GroundTruth {
    pub run_id: String,
    pub injection_id: String,
    pub injected_class: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub source_started_at_ms: u64,
    pub source_finished_at_ms: u64,
    pub battery_model: ModelProvenance,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mutations: Vec<ExecutedMutation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
}

#[derive(Debug, Serialize)]
pub struct ModelProvenance {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct ExecutedMutation {
    pub signal: String,
    pub operator: String,
    pub requested_parameters: MutationParameters,
    pub executed_values: Vec<f32>,
}

#[derive(Debug, Serialize)]
pub struct OracleSidecar {
    pub status: &'static str,
    pub injection_id: String,
    pub generation_goal: GenerationGoal,
    pub evaluation_window_ms: EvaluationWindow,
    pub predicted_observations: Vec<PredictedObservation>,
}

#[derive(Debug, Serialize)]
pub struct EvaluationWindow {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug)]
struct Candidate {
    frames: Vec<CaseFrame>,
    executed: Vec<ExecutedMutation>,
    start_ms: u64,
    end_ms: u64,
    source_start_ms: u64,
    source_end_ms: u64,
    score: f64,
}

pub fn generate(
    request: &GenerationRequest,
    injection: &ResolvedInjection,
    template: &AscDocument,
    config: &GuardianConfig,
    model_hash: &str,
) -> Result<GenerationOutcome> {
    let frames = template.battery_frames();
    if request.lead_in_frames >= frames.len() {
        bail!(
            "lead_in_frames {} leaves no work region in {} battery frames",
            request.lead_in_frames,
            frames.len()
        );
    }

    let candidates = if let Some(action) = &injection.action {
        vec![apply_action(
            &frames,
            request.lead_in_frames,
            action,
            config,
        )?]
    } else {
        signal_candidates(request, injection, &frames, config)?
    };

    if candidates.is_empty() {
        return Ok(GenerationOutcome::Unsatisfiable(Unsatisfiable {
            status: "UNSATISFIABLE",
            injection_id: request.injection_id.clone(),
            reason: UnsatisfiableReason {
                code: "ENCODING_LIMIT".to_owned(),
                detail: "no DBC-representable candidate trajectory could be constructed".to_owned(),
            },
        }));
    }

    let mut accepted = Vec::new();
    let mut last_failure = String::new();
    for candidate in candidates {
        let simulation_end = candidate
            .frames
            .iter()
            .filter(|frame| !frame.removed)
            .map(|frame| frame.arrival_ms)
            .max()
            .unwrap_or(candidate.end_ms)
            .max(candidate.end_ms);
        let observations = evaluate_trajectory(&candidate.frames, config, simulation_end);
        let scoped: Vec<PredictedObservation> = observations
            .into_iter()
            .filter(|observation| {
                observation.predicted_at_ms >= candidate.start_ms
                    && observation.predicted_at_ms <= candidate.end_ms
            })
            .collect();
        match check_goal(&request.generation_goal, &scoped) {
            Ok(()) => accepted.push((candidate, scoped)),
            Err(detail) => last_failure = detail,
        }
    }

    accepted.sort_by(|(left, _), (right, _)| {
        left.score
            .partial_cmp(&right.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.source_end_ms.cmp(&right.source_end_ms))
    });
    let Some((candidate, _candidate_observations)) = accepted.into_iter().next() else {
        return Ok(GenerationOutcome::Unsatisfiable(Unsatisfiable {
            status: "UNSATISFIABLE",
            injection_id: request.injection_id.clone(),
            reason: UnsatisfiableReason {
                code: if last_failure.contains("forbidden") {
                    "FORBIDDEN_CODETECTION"
                } else {
                    "PRIMARY_NOT_REACHED"
                }
                .to_owned(),
                detail: last_failure,
            },
        }));
    };

    let mut output = template.clone();
    output.apply_frames(&candidate.frames)?;
    let rendered_asc = output.render();
    let rendered_document = AscDocument::parse(&rendered_asc)
        .context("reparse rendered ASC for final forward verification")?;
    let rendered_frames = rendered_document.battery_frames();
    let rendered_simulation_end = rendered_frames
        .iter()
        .filter(|frame| !frame.removed)
        .map(|frame| frame.arrival_ms)
        .max()
        .unwrap_or(candidate.end_ms)
        .max(candidate.end_ms);
    let observations: Vec<PredictedObservation> =
        evaluate_trajectory(&rendered_frames, config, rendered_simulation_end)
            .into_iter()
            .filter(|observation| {
                observation.predicted_at_ms >= candidate.start_ms
                    && observation.predicted_at_ms <= candidate.end_ms
            })
            .collect();
    check_goal(&request.generation_goal, &observations).map_err(|detail| {
        anyhow::anyhow!("rendered ASC no longer satisfies its generation goal: {detail}")
    })?;
    let duration_ms = candidate
        .source_end_ms
        .saturating_sub(candidate.source_start_ms);
    let ground_truth = GroundTruth {
        run_id: request.run_id().to_owned(),
        injection_id: request.injection_id.clone(),
        injected_class: request.injected_class.clone(),
        started_at: request.started_at.clone(),
        duration_ms,
        source_started_at_ms: candidate.source_start_ms,
        source_finished_at_ms: candidate.source_end_ms,
        battery_model: ModelProvenance {
            path: request.battery_model.display().to_string(),
            sha256: model_hash.to_owned(),
        },
        mutations: candidate.executed,
        action: injection.action.clone(),
    };
    let oracle = OracleSidecar {
        status: "SATISFIED",
        injection_id: request.injection_id.clone(),
        generation_goal: request.generation_goal.clone(),
        evaluation_window_ms: EvaluationWindow {
            start: candidate.start_ms,
            end: candidate.end_ms,
        },
        predicted_observations: observations,
    };
    Ok(GenerationOutcome::Generated(Box::new(GeneratedCase {
        asc: rendered_asc,
        ground_truth,
        oracle,
    })))
}

fn signal_candidates(
    request: &GenerationRequest,
    injection: &ResolvedInjection,
    original: &[CaseFrame],
    config: &GuardianConfig,
) -> Result<Vec<Candidate>> {
    let start = request.lead_in_frames;
    let baseline = original[start - 1].values;
    let choices: Vec<Vec<Option<f32>>> = injection
        .mutations
        .iter()
        .map(|mutation| parameter_choices(mutation, baseline, request, config))
        .collect::<Result<_>>()?;
    let combinations = cartesian_choices(&choices);
    let mut candidates = Vec::new();
    for combination in combinations {
        if let Ok(candidate) =
            apply_mutations(original, start, &injection.mutations, &combination, config)
        {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn parameter_choices(
    mutation: &Mutation,
    baseline: SignalValues,
    request: &GenerationRequest,
    config: &GuardianConfig,
) -> Result<Vec<Option<f32>>> {
    if mutation.operator == "stuck" {
        return Ok(vec![None]);
    }
    let quantum = if mutation.signal == "soc" {
        SOC_QUANTUM_PP
    } else {
        TEMPERATURE_QUANTUM_C
    };
    let base = baseline.get(&mutation.signal);
    let requested = match mutation.operator.as_str() {
        "spike" => mutation.parameters.delta.context("validated spike delta")?,
        "drift" => mutation
            .parameters
            .rate_per_sample
            .context("validated drift rate")?,
        "out_of_range" => mutation
            .parameters
            .value
            .context("validated target value")?,
        _ => unreachable!("validated operator"),
    };

    let mut values = Vec::new();
    if mutation.operator == "out_of_range" {
        let direction = if requested >= base { 1.0 } else { -1.0 };
        for index in 1..=request.search.max_candidate_quanta {
            let candidate = base + direction * quantum * index as f32;
            if quantize_signal(&mutation.signal, candidate).is_ok() {
                values.push(candidate);
            }
        }
        if quantize_signal(&mutation.signal, requested).is_ok() {
            values.push(requested);
        }
        if mutation.signal == "temp_min" {
            values.push(config.temperature.absolute_min_c - TEMPERATURE_QUANTUM_C);
        }
        if mutation.signal == "temp_max" {
            values.push(config.temperature.absolute_max_c + TEMPERATURE_QUANTUM_C);
        }
    } else {
        let direction = requested.signum();
        if direction == 0.0 {
            bail!("{} parameter must be non-zero", mutation.operator);
        }
        for index in 1..=request.search.max_candidate_quanta {
            values.push(direction * quantum * index as f32);
        }
        values.push(requested);
    }

    values.sort_by(|left, right| {
        let left_magnitude = if mutation.operator == "out_of_range" {
            (left - base).abs()
        } else {
            left.abs()
        };
        let right_magnitude = if mutation.operator == "out_of_range" {
            (right - base).abs()
        } else {
            right.abs()
        };
        left_magnitude
            .partial_cmp(&right_magnitude)
            .unwrap_or(Ordering::Equal)
    });
    values.dedup_by(|left, right| (*left - *right).abs() < 1e-4);
    Ok(values.into_iter().map(Some).collect())
}

fn cartesian_choices(dimensions: &[Vec<Option<f32>>]) -> Vec<Vec<Option<f32>>> {
    let mut result = vec![Vec::new()];
    for dimension in dimensions {
        let mut next = Vec::new();
        for prefix in &result {
            for value in dimension {
                let mut combination = prefix.clone();
                combination.push(*value);
                next.push(combination);
                if next.len() >= MAX_COMBINATIONS {
                    break;
                }
            }
            if next.len() >= MAX_COMBINATIONS {
                break;
            }
        }
        result = next;
    }
    result
}

fn apply_mutations(
    original: &[CaseFrame],
    start: usize,
    mutations: &[Mutation],
    choices: &[Option<f32>],
    config: &GuardianConfig,
) -> Result<Candidate> {
    let mut frames = original.to_vec();
    let baseline = frames[start - 1].values;
    let mut executed = Vec::new();
    let mut max_duration = 0;

    for (mutation, choice) in mutations.iter().zip(choices) {
        let duration = mutation.parameters.duration_samples;
        if mutation.operator == "stuck" && duration < config.stuck.window_samples {
            bail!("stuck duration is shorter than configured window");
        }
        if start + duration > frames.len() {
            bail!("mutation trajectory exceeds ASC template");
        }
        max_duration = max_duration.max(duration);
        let mut values = Vec::with_capacity(duration);
        for offset in 0..duration {
            let index = start + offset;
            let value = match mutation.operator.as_str() {
                "stuck" => baseline.get(&mutation.signal),
                "spike" => {
                    frames[index].values.get(&mutation.signal)
                        + choice.context("spike candidate parameter")?
                }
                "drift" => {
                    baseline.get(&mutation.signal)
                        + choice.context("drift candidate parameter")? * (offset + 1) as f32
                }
                "out_of_range" => choice.context("out-of-range candidate parameter")?,
                _ => unreachable!("validated operator"),
            };
            let represented = quantize_signal(&mutation.signal, value)?;
            frames[index].values.set(&mutation.signal, represented);
            values.push(represented);
        }
        executed.push(ExecutedMutation {
            signal: mutation.signal.clone(),
            operator: mutation.operator.clone(),
            requested_parameters: mutation.parameters.clone(),
            executed_values: values,
        });
    }

    ensure_stuck_excitation(&frames, start, mutations, config)?;
    let return_index = (start + max_duration).min(frames.len() - 1);
    let start_ms = frames[start].arrival_ms;
    let end_ms = frames[return_index].arrival_ms;
    let source_start_ms = frames[start].source_ms;
    let source_end_ms = frames[return_index].source_ms;
    let score = mutation_score(original, &frames, start, return_index);
    Ok(Candidate {
        frames,
        executed,
        start_ms,
        end_ms,
        source_start_ms,
        source_end_ms,
        score,
    })
}

fn ensure_stuck_excitation(
    frames: &[CaseFrame],
    start: usize,
    mutations: &[Mutation],
    config: &GuardianConfig,
) -> Result<()> {
    for mutation in mutations
        .iter()
        .filter(|mutation| mutation.operator == "stuck")
    {
        let end = start + mutation.parameters.duration_samples;
        let temperature_peers = ["temp_min", "temp_avg", "temp_max"]
            .into_iter()
            .filter(|signal| *signal != mutation.signal);
        let excited = temperature_peers.clone().any(|signal| {
            amplitude(&frames[start..end], signal) >= config.stuck.temperature_excitation_c
        }) || amplitude(&frames[start..end], "soc") >= config.stuck.soc_excitation_pp;
        if excited {
            continue;
        }

        bail!(
            "stuck trajectory has no independent excitation in its nominal or explicitly mutated peers"
        );
    }
    Ok(())
}

fn amplitude(frames: &[CaseFrame], signal: &str) -> f32 {
    let minimum = frames
        .iter()
        .map(|frame| frame.values.get(signal))
        .fold(f32::INFINITY, f32::min);
    let maximum = frames
        .iter()
        .map(|frame| frame.values.get(signal))
        .fold(f32::NEG_INFINITY, f32::max);
    maximum - minimum
}

fn mutation_score(
    original: &[CaseFrame],
    candidate: &[CaseFrame],
    start: usize,
    end: usize,
) -> f64 {
    (start..=end)
        .map(|index| {
            let left = original[index].values;
            let right = candidate[index].values;
            ((left.temp_min - right.temp_min).abs()
                + (left.temp_avg - right.temp_avg).abs()
                + (left.temp_max - right.temp_max).abs()
                + (left.soc - right.soc).abs()) as f64
        })
        .sum()
}

fn apply_action(
    original: &[CaseFrame],
    start: usize,
    action: &Action,
    config: &GuardianConfig,
) -> Result<Candidate> {
    let mut frames = original.to_vec();
    let source_start_ms = frames[start].source_ms;
    let source_end_ms = source_start_ms.saturating_add(action.duration_ms);
    let start_ms = frames[start - 1].arrival_ms;
    match action.operator.as_str() {
        "drop" | "suspend_source" => {
            for frame in &mut frames[start..] {
                if frame.source_ms < source_end_ms {
                    frame.removed = true;
                }
            }
        }
        "delay" => {
            let delay_ms = action.delay_ms.context("validated transport delay")?;
            for frame in &mut frames[start..] {
                frame.arrival_ms = frame.arrival_ms.saturating_add(delay_ms);
            }
        }
        _ => bail!("unsupported action operator {:?}", action.operator),
    }

    let first_after = frames
        .iter()
        .skip(start)
        .find(|frame| !frame.removed && frame.source_ms >= source_end_ms);
    let end_ms = first_after.map_or_else(
        || {
            frames[start - 1]
                .arrival_ms
                .saturating_add(config.missing_packet_timeout_ms)
                .saturating_add(config.evaluation_period_ms * 2)
        },
        |frame| frame.arrival_ms,
    );
    Ok(Candidate {
        frames,
        executed: Vec::new(),
        start_ms,
        end_ms,
        source_start_ms,
        source_end_ms,
        score: 0.0,
    })
}

fn check_goal(goal: &GenerationGoal, observations: &[PredictedObservation]) -> Result<(), String> {
    let observed: BTreeSet<ObservationSpec> = observations
        .iter()
        .filter(|observation| observation.state == "active")
        .map(PredictedObservation::specification)
        .collect();
    let primary: BTreeSet<_> = goal.primary.iter().cloned().collect();
    let allowed: BTreeSet<_> = goal.allowed.iter().cloned().collect();
    let forbidden: HashSet<_> = goal.forbidden.iter().cloned().collect();

    let missing: Vec<_> = primary.difference(&observed).collect();
    if !missing.is_empty() {
        return Err(format!("primary observations not reached: {missing:?}"));
    }
    let explicit_forbidden: Vec<_> = observed
        .iter()
        .filter(|entry| forbidden.contains(*entry))
        .collect();
    if !explicit_forbidden.is_empty() {
        return Err(format!(
            "forbidden observations produced: {explicit_forbidden:?}"
        ));
    }
    if !goal.allow_unspecified_codetections {
        let permitted: BTreeSet<_> = primary.union(&allowed).cloned().collect();
        let unspecified: Vec<_> = observed.difference(&permitted).collect();
        if !unspecified.is_empty() {
            return Err(format!(
                "forbidden unspecified co-detections produced: {unspecified:?}"
            ));
        }
    }
    Ok(())
}
