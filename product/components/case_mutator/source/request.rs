use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const CANONICAL_SIGNALS: &[&str] = &["temp_min", "temp_avg", "temp_max", "soc"];

const DETECTION_CLASSES: &[&str] = &[
    "STREAM_STALE",
    "STREAM_GENERATION_GAP",
    "THERMAL_LIMIT",
    "PHYSICAL_TEMP_ABSOLUTE_LIMIT",
    "PHYSICAL_TEMP_ORDERING",
    "PHYSICAL_TEMP_SPREAD",
    "PHYSICAL_TEMP_HOTSPOT",
    "PHYSICAL_TEMP_RATE",
    "PHYSICAL_SOC_RANGE",
    "PHYSICAL_SOC_RATE",
    "SIGNAL_STUCK",
];

const DETECTION_LEVELS: &[&str] = &["WARNING", "VIOLATION", "CRITICAL"];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationRequest {
    pub template: PathBuf,
    pub battery_model: PathBuf,
    pub fault_injection_model: Option<PathBuf>,
    pub run_id: Option<String>,
    pub started_at: String,
    pub injection_id: String,
    pub injected_class: String,
    pub mutations: Option<Vec<Mutation>>,
    pub action: Option<Action>,
    pub generation_goal: GenerationGoal,
    #[serde(default)]
    pub seed: u64,
    #[serde(default = "default_lead_in_frames")]
    pub lead_in_frames: usize,
    #[serde(default)]
    pub search: SearchConfig,
}

fn default_lead_in_frames() -> usize {
    20
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub signal: String,
    pub operator: String,
    pub parameters: MutationParameters,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MutationParameters {
    pub duration_samples: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_per_sample: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub operator: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationGoal {
    #[serde(default)]
    pub primary: Vec<ObservationSpec>,
    #[serde(default)]
    pub allowed: Vec<ObservationSpec>,
    #[serde(default)]
    pub forbidden: Vec<ObservationSpec>,
    #[serde(default)]
    pub allow_unspecified_codetections: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ObservationSpec {
    pub class: String,
    pub level: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchConfig {
    pub warning_target_utilization: f32,
    pub violation_target_utilization: f32,
    pub max_candidate_quanta: usize,
    pub exact_parameters: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            warning_target_utilization: 0.9,
            violation_target_utilization: 1.1,
            max_candidate_quanta: 64,
            exact_parameters: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedInjection {
    pub mutations: Vec<Mutation>,
    pub action: Option<Action>,
}

#[derive(Debug, Deserialize)]
struct FaultInjectionModel {
    injections: Vec<CatalogInjection>,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogInjection {
    id: String,
    injected_class: String,
    mutations: Option<Vec<Mutation>>,
    action: Option<Action>,
}

impl GenerationRequest {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("read generation request {}", path.display()))?;
        let request: Self = serde_yaml::from_str(&contents)
            .with_context(|| format!("parse generation request {}", path.display()))?;
        request.validate()?;
        Ok(request)
    }

    pub fn resolve_injection(&self) -> Result<ResolvedInjection> {
        let inline = self.mutations.is_some() || self.action.is_some();
        let resolved = if inline {
            ResolvedInjection {
                mutations: self.mutations.clone().unwrap_or_default(),
                action: self.action.clone(),
            }
        } else {
            let path = self.fault_injection_model.as_ref().context(
                "request without inline mutations/action requires fault_injection_model",
            )?;
            let contents = fs::read_to_string(path)
                .with_context(|| format!("read fault-injection model {}", path.display()))?;
            let catalog: FaultInjectionModel = serde_yaml::from_str(&contents)
                .with_context(|| format!("parse fault-injection model {}", path.display()))?;
            let injection = catalog
                .injections
                .into_iter()
                .find(|entry| entry.id == self.injection_id)
                .with_context(|| {
                    format!(
                        "injection_id {:?} not found in {}",
                        self.injection_id,
                        path.display()
                    )
                })?;
            if injection.injected_class != self.injected_class {
                bail!(
                    "request injected_class {:?} does not match catalog class {:?}",
                    self.injected_class,
                    injection.injected_class
                );
            }
            ResolvedInjection {
                mutations: injection.mutations.unwrap_or_default(),
                action: injection.action,
            }
        };
        validate_injection(&self.injected_class, &resolved)?;
        Ok(resolved)
    }

    pub fn run_id(&self) -> &str {
        self.run_id.as_deref().unwrap_or(&self.injection_id)
    }

    fn validate(&self) -> Result<()> {
        if self.injection_id.trim().is_empty() {
            bail!("injection_id must not be empty");
        }
        if self.started_at.trim().is_empty() {
            bail!("started_at must not be empty");
        }
        if self.lead_in_frames == 0 {
            bail!("lead_in_frames must be greater than zero");
        }
        if self.search.max_candidate_quanta == 0 {
            bail!("search.max_candidate_quanta must be greater than zero");
        }
        if !(0.0 < self.search.warning_target_utilization
            && self.search.warning_target_utilization <= 1.0)
        {
            bail!("search.warning_target_utilization must be in (0, 1]");
        }
        if self.search.violation_target_utilization <= 1.0 {
            bail!("search.violation_target_utilization must be greater than 1");
        }
        let mut categories = BTreeMap::new();
        for (name, observations) in [
            ("primary", &self.generation_goal.primary),
            ("allowed", &self.generation_goal.allowed),
            ("forbidden", &self.generation_goal.forbidden),
        ] {
            for observation in observations {
                validate_observation(observation)?;
                let previous = categories.insert(observation.clone(), name);
                if let Some(previous) = previous {
                    bail!(
                        "generation goal observation {}/{} appears in both {} and {}",
                        observation.class,
                        observation.level,
                        previous,
                        name
                    );
                }
            }
        }
        Ok(())
    }
}

fn validate_observation(observation: &ObservationSpec) -> Result<()> {
    if !DETECTION_CLASSES.contains(&observation.class.as_str()) {
        bail!("unknown detection class {:?}", observation.class);
    }
    if !DETECTION_LEVELS.contains(&observation.level.as_str()) {
        bail!("unknown detection level {:?}", observation.level);
    }
    Ok(())
}

fn validate_injection(injected_class: &str, injection: &ResolvedInjection) -> Result<()> {
    let action_class = matches!(
        injected_class,
        "transport.delay" | "transport.drop" | "source.dropout"
    );
    if action_class {
        if !injection.mutations.is_empty() {
            bail!("{injected_class} must not contain signal mutations");
        }
        let action = injection
            .action
            .as_ref()
            .context("transport/source injection requires action")?;
        let expected = match injected_class {
            "transport.delay" => "delay",
            "transport.drop" => "drop",
            "source.dropout" => "suspend_source",
            _ => unreachable!(),
        };
        if action.operator != expected {
            bail!("{injected_class} requires action operator {expected:?}");
        }
        if action.duration_ms == 0 {
            bail!("action.duration_ms must be greater than zero");
        }
        if injected_class == "transport.delay" && action.delay_ms.unwrap_or_default() == 0 {
            bail!("transport.delay requires delay_ms greater than zero");
        }
        return Ok(());
    }

    if injection.action.is_some() {
        bail!("{injected_class} must not contain action");
    }
    let expected_operator = match injected_class {
        "signal.stuck" => Some("stuck"),
        "signal.spike" => Some("spike"),
        "signal.drift" => Some("drift"),
        "signal.out_of_range" => Some("out_of_range"),
        "signal.combination" => None,
        _ => bail!("unsupported injected_class {injected_class:?}"),
    };
    let expected_count = if injected_class == "signal.combination" {
        2
    } else {
        1
    };
    if injection.mutations.len() < expected_count
        || (expected_count == 1 && injection.mutations.len() != 1)
    {
        bail!("{injected_class} has invalid mutation count");
    }
    let mut signals = HashSet::new();
    for mutation in &injection.mutations {
        if !CANONICAL_SIGNALS.contains(&mutation.signal.as_str()) {
            bail!("unknown canonical signal {:?}", mutation.signal);
        }
        if !signals.insert(&mutation.signal) {
            bail!("combination mutations must target distinct signals");
        }
        if let Some(expected) = expected_operator {
            if mutation.operator != expected {
                bail!("{injected_class} requires operator {expected:?}");
            }
        }
        if mutation.operator == "stuck" && mutation.signal == "soc" {
            bail!("SoC is not a valid stuck target");
        }
        if !matches!(
            mutation.operator.as_str(),
            "stuck" | "spike" | "drift" | "out_of_range"
        ) {
            bail!("unsupported mutation operator {:?}", mutation.operator);
        }
        validate_parameters(mutation)?;
    }
    Ok(())
}

fn validate_parameters(mutation: &Mutation) -> Result<()> {
    let parameters = &mutation.parameters;
    if parameters.duration_samples == 0 {
        bail!("mutation duration_samples must be greater than zero");
    }
    match mutation.operator.as_str() {
        "stuck" => {
            if parameters.delta.is_some()
                || parameters.rate_per_sample.is_some()
                || parameters.value.is_some()
            {
                bail!("stuck accepts only duration_samples");
            }
        }
        "spike" => {
            if parameters.delta.is_none()
                || parameters.rate_per_sample.is_some()
                || parameters.value.is_some()
            {
                bail!("spike requires exactly delta and duration_samples");
            }
        }
        "drift" => {
            if parameters.rate_per_sample.is_none()
                || parameters.delta.is_some()
                || parameters.value.is_some()
            {
                bail!("drift requires exactly rate_per_sample and duration_samples");
            }
        }
        "out_of_range" => {
            if parameters.value.is_none()
                || parameters.delta.is_some()
                || parameters.rate_per_sample.is_some()
            {
                bail!("out_of_range requires exactly value and duration_samples");
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soc_is_rejected_as_stuck_target() {
        let injection = ResolvedInjection {
            mutations: vec![Mutation {
                signal: "soc".into(),
                operator: "stuck".into(),
                parameters: MutationParameters {
                    duration_samples: 10,
                    ..MutationParameters::default()
                },
            }],
            action: None,
        };
        assert!(validate_injection("signal.stuck", &injection)
            .unwrap_err()
            .to_string()
            .contains("not a valid stuck target"));
    }
}
