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
/// Upper bound for the construction-failure summary text inside an UNSAT
/// record detail (`reason.detail` itself has no schema length limit; this
/// keeps the construction share short and single-quoted-YAML readable).
const CONSTRUCTION_FAILURE_DETAIL_LIMIT: usize = 400;

/// One rejected candidate-construction attempt, kept for UNSAT diagnostics.
#[derive(Debug)]
struct ConstructionFailure {
    /// Combinatorial candidate index; `None` marks a failure while deriving
    /// the candidate parameter choices themselves.
    candidate: Option<usize>,
    message: String,
}

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

    let mut construction_failures: Vec<ConstructionFailure> = Vec::new();
    let candidates = if let Some(action) = &injection.action {
        vec![apply_action(
            &frames,
            request.lead_in_frames,
            action,
            config,
        )?]
    } else {
        let (candidates, failures) = signal_candidates(request, injection, &frames, config);
        construction_failures = failures;
        candidates
    };

    if candidates.is_empty() {
        // Every candidate was rejected while being constructed; surface the
        // concrete rejection instead of the generic no-encoding note.
        let detail = if construction_failures.is_empty() {
            "no DBC-representable candidate trajectory could be constructed".to_owned()
        } else {
            summarize_construction_failures(&construction_failures)
        };
        return Ok(GenerationOutcome::Unsatisfiable(Unsatisfiable {
            status: "UNSATISFIABLE",
            injection_id: request.injection_id.clone(),
            reason: UnsatisfiableReason {
                code: "ENCODING_LIMIT".to_owned(),
                detail,
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
        match check_goal(request, &scoped) {
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
        // Goal-level rejections keep the existing code/detail semantics; any
        // additionally observed construction failures are appended afterwards.
        let code = if last_failure.contains("forbidden") {
            "FORBIDDEN_CODETECTION"
        } else {
            "PRIMARY_NOT_REACHED"
        };
        let mut detail = last_failure;
        if !construction_failures.is_empty() {
            detail.push_str("; additionally observed construction failures: ");
            detail.push_str(&summarize_construction_failures(&construction_failures));
        }
        return Ok(GenerationOutcome::Unsatisfiable(Unsatisfiable {
            status: "UNSATISFIABLE",
            injection_id: request.injection_id.clone(),
            reason: UnsatisfiableReason {
                code: code.to_owned(),
                detail,
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
    check_goal(request, &observations).map_err(|detail| {
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
) -> (Vec<Candidate>, Vec<ConstructionFailure>) {
    let start = request.lead_in_frames;
    let baseline = original[start - 1].values;
    let mut candidates = Vec::new();
    let mut construction_failures = Vec::new();
    let mut choices = Vec::with_capacity(injection.mutations.len());
    for (index, mutation) in injection.mutations.iter().enumerate() {
        match parameter_choices(mutation, baseline, request, config) {
            Ok(values) => choices.push(values),
            Err(error) => construction_failures.push(ConstructionFailure {
                candidate: None,
                message: format!("mutation {index}: {error:#}"),
            }),
        }
    }
    if !construction_failures.is_empty() {
        return (candidates, construction_failures);
    }
    let combinations = cartesian_choices(&choices);
    for (index, combination) in combinations.iter().enumerate() {
        match apply_mutations(
            original,
            start,
            &injection.mutations,
            combination,
            &request.generation_goal,
            config,
        ) {
            Ok(candidate) => candidates.push(candidate),
            Err(error) => construction_failures.push(ConstructionFailure {
                candidate: Some(index),
                message: format!("{error:#}"),
            }),
        }
    }
    (candidates, construction_failures)
}

/// Reduces per-candidate construction failures to the UNSAT detail text. A
/// single failed attempt keeps its concrete message verbatim; candidates that
/// share one failure message collapse into one group labeled with their
/// candidate indices; distinct causes are joined with "; ". The joined text
/// is hard-capped at CONSTRUCTION_FAILURE_DETAIL_LIMIT characters.
fn summarize_construction_failures(failures: &[ConstructionFailure]) -> String {
    let render_single = |failure: &ConstructionFailure| match failure.candidate {
        Some(index) => format!("candidate {index}: {}", failure.message),
        None => format!("candidate parameters: {}", failure.message),
    };
    if failures.len() == 1 {
        return render_single(&failures[0]);
    }
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    for failure in failures {
        match groups
            .iter_mut()
            .find(|(message, _)| *message == failure.message)
        {
            Some((_, labels)) => {
                let label = failure
                    .candidate
                    .map_or_else(|| "parameters".to_owned(), |index| index.to_string());
                if labels.last() != Some(&label) {
                    labels.push(label);
                }
            }
            None => {
                let label = failure
                    .candidate
                    .map_or_else(|| "parameters".to_owned(), |index| index.to_string());
                groups.push((failure.message.as_str(), vec![label]));
            }
        }
    }
    let mut summary = groups
        .into_iter()
        .map(|(message, labels)| {
            let prefix = match labels.len() {
                1 if labels[0] == "parameters" => "candidate parameters".to_owned(),
                1 => format!("candidate {}", labels[0]),
                count if count > 4 => {
                    // Keep the summary readable: the first four candidate
                    // labels plus a count instead of every index.
                    format!(
                        "candidates {}, … (+{} more attempts)",
                        labels[..4].join(", "),
                        count - 4
                    )
                }
                _ => format!("candidates {}", labels.join(", ")),
            };
            format!("{prefix}: {message}")
        })
        .collect::<Vec<_>>()
        .join("; ");
    if summary.chars().count() > CONSTRUCTION_FAILURE_DETAIL_LIMIT {
        let budget = CONSTRUCTION_FAILURE_DETAIL_LIMIT - '…'.len_utf8();
        summary = summary.chars().take(budget).collect();
        summary.push('…');
    }
    summary
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

    if request.search.exact_parameters {
        let exact = if mutation.operator == "out_of_range" {
            quantize_signal(&mutation.signal, requested)?
        } else {
            let direction = requested.signum();
            if direction == 0.0 {
                bail!("{} parameter must be non-zero", mutation.operator);
            }
            requested
        };
        return Ok(vec![Some(exact)]);
    }

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
    goal: &GenerationGoal,
    config: &GuardianConfig,
) -> Result<Candidate> {
    let mut frames = original.to_vec();
    let baseline = frames[start - 1].values;
    let mut executed = Vec::new();
    let mut max_duration = 0;

    for (mutation, choice) in mutations.iter().zip(choices) {
        let duration = mutation.parameters.duration_samples;
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

    ensure_stuck_excitation(&frames, start, mutations, goal, config)?;
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
    goal: &GenerationGoal,
    config: &GuardianConfig,
) -> Result<()> {
    // ADR-014 negative-control semantics (forward-verified per guardian model):
    // the Guardian arms stuck detection only on peer excitation (>=
    // temperature_excitation_c in the window, guardian_model.yaml). A
    // negative control that forbids SIGNAL_STUCK therefore gets its wanted
    // outcome exactly from an UNexcited stuck hold, and must not be rejected
    // by this gate. Skip only when SIGNAL_STUCK appears in `forbidden`;
    // neutral/empty goals keep enforcement (fail-closed, no verdict risk).
    if goal
        .forbidden
        .iter()
        .any(|observation| observation.class == "SIGNAL_STUCK")
    {
        return Ok(());
    }
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

fn check_goal(
    request: &GenerationRequest,
    observations: &[PredictedObservation],
) -> Result<(), String> {
    let goal = &request.generation_goal;
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
    for target in &goal.primary {
        let matching: Vec<_> = observations
            .iter()
            .filter(|observation| {
                observation.state == "active"
                    && observation.class == target.class
                    && observation.level == target.level
            })
            .collect();
        let required = match target.level.as_str() {
            "WARNING" => Some(request.search.warning_target_utilization),
            "VIOLATION" => Some(request.search.violation_target_utilization),
            _ => None,
        };
        if let Some(required) = required {
            if matching
                .iter()
                .any(|observation| observation.utilization.is_some())
                && !matching
                    .iter()
                    .filter_map(|observation| observation.utilization)
                    .any(|utilization| utilization + 1e-6 >= required)
            {
                return Err(format!(
                    "primary observation {}/{} did not reach target utilization {}",
                    target.class, target.level, required
                ));
            }
        }
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

#[cfg(test)]
mod construction_failure_summary_tests {
    use super::*;

    fn failure(candidate: Option<usize>, message: &str) -> ConstructionFailure {
        ConstructionFailure {
            candidate,
            message: message.to_owned(),
        }
    }

    #[test]
    fn single_failure_is_reported_verbatim_with_candidate_index() {
        let failures = [failure(
            Some(0),
            "stuck trajectory has no independent excitation",
        )];
        assert_eq!(
            summarize_construction_failures(&failures),
            "candidate 0: stuck trajectory has no independent excitation"
        );
    }

    #[test]
    fn single_parameter_failure_is_reported_verbatim() {
        let failures = [failure(
            None,
            "mutation 0: spike parameter must be non-zero",
        )];
        assert_eq!(
            summarize_construction_failures(&failures),
            "candidate parameters: mutation 0: spike parameter must be non-zero"
        );
    }

    #[test]
    fn identical_failures_collapse_into_one_labeled_group() {
        let message = "stuck trajectory has no independent excitation";
        let failures = [
            failure(Some(0), message),
            failure(Some(1), message),
            failure(Some(2), message),
        ];
        assert_eq!(
            summarize_construction_failures(&failures),
            "candidates 0, 1, 2: stuck trajectory has no independent excitation"
        );
    }

    #[test]
    fn large_identical_group_lists_first_labels_and_counts() {
        let failures: Vec<_> = (0..40)
            .map(|index| failure(Some(index), "quantization conflict"))
            .collect();
        assert_eq!(
            summarize_construction_failures(&failures),
            "candidates 0, 1, 2, 3, … (+36 more attempts): quantization conflict"
        );
    }

    #[test]
    fn distinct_failures_join_with_separator() {
        let failures = [failure(Some(0), "alpha"), failure(Some(2), "beta")];
        assert_eq!(
            summarize_construction_failures(&failures),
            "candidate 0: alpha; candidate 2: beta"
        );
    }

    #[test]
    fn joined_summary_is_capped_at_limit_with_ellipsis() {
        let failures: Vec<_> = (0..50)
            .map(|index| failure(Some(index), format!("failure {index} u").as_str()))
            .collect();
        let summary = summarize_construction_failures(&failures);
        assert!(summary.chars().count() <= CONSTRUCTION_FAILURE_DETAIL_LIMIT);
        assert!(summary.ends_with('…'));
        assert!(summary.starts_with("candidate 0: failure 0"));
    }
}
