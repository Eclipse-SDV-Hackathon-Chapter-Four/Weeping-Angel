use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use battery_guardian::GuardianConfig;
use case_mutator::asc::AscDocument;
use case_mutator::oracle::evaluate_trajectory;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct OracleFile {
    schema_version: u8,
    scenario_id: String,
    source_window_ms: Window,
    guardian: GuardianOracle,
    dfm: DfmOracle,
}

#[derive(Debug, Serialize)]
struct Window {
    start: u64,
    end: u64,
}

#[derive(Debug, Serialize)]
struct GuardianOracle {
    allow_unspecified: bool,
    transitions: Vec<Transition>,
}

#[derive(Debug, Serialize)]
struct Transition {
    at_ms: u64,
    class: String,
    level: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    signal: Option<String>,
}

#[derive(Debug, Serialize)]
struct DfmOracle {
    derive_from: &'static str,
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        println!("Usage: case-oracle --asc <case.asc> --battery-model <guardian_model.yaml> --scenario-id <id> --output <case.oracle.yaml>");
        return ExitCode::SUCCESS;
    }
    match execute(arguments.into_iter()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("case-oracle failed: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn execute(mut arguments: impl Iterator<Item = String>) -> Result<()> {
    let mut asc = None;
    let mut battery_model = None;
    let mut scenario_id = None;
    let mut output = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--asc" => asc = arguments.next().map(PathBuf::from),
            "--battery-model" => battery_model = arguments.next().map(PathBuf::from),
            "--scenario-id" => scenario_id = arguments.next(),
            "--output" => output = arguments.next().map(PathBuf::from),
            _ => bail!("unknown argument {argument:?}"),
        }
    }
    write_oracle(
        &asc.context("--asc is required")?,
        &battery_model.context("--battery-model is required")?,
        &scenario_id.context("--scenario-id is required")?,
        &output.context("--output is required")?,
    )
}

fn write_oracle(
    asc_path: &Path,
    model_path: &Path,
    scenario_id: &str,
    output: &Path,
) -> Result<()> {
    let config = GuardianConfig::load(model_path)?;
    let document = AscDocument::load(asc_path)?;
    let frames = document.battery_frames();
    let visible: Vec<_> = frames.iter().filter(|frame| !frame.removed).collect();
    let first = visible
        .first()
        .context("ASC contains no visible battery frame")?;
    let last = visible
        .last()
        .context("ASC contains no visible battery frame")?;
    let simulation_end_ms = visible
        .iter()
        .map(|frame| frame.arrival_ms)
        .max()
        .unwrap_or(last.arrival_ms);
    let transitions = evaluate_trajectory(&frames, &config, simulation_end_ms)
        .into_iter()
        .map(|observation| Transition {
            at_ms: observation.predicted_at_ms,
            class: observation.class,
            level: observation.level,
            state: observation.state,
            signal: observation.signal,
        })
        .collect();
    let oracle = OracleFile {
        schema_version: 1,
        scenario_id: scenario_id.to_owned(),
        source_window_ms: Window {
            start: first.source_ms,
            end: last.source_ms,
        },
        guardian: GuardianOracle {
            allow_unspecified: false,
            transitions,
        },
        dfm: DfmOracle {
            derive_from: "product/config/battery_guardian/guardian_diagnostics.json",
        },
    };
    let rendered = serde_yaml::to_string(&oracle).context("serialize Oracle YAML")?;
    fs::write(output, rendered).with_context(|| format!("write Oracle YAML {}", output.display()))
}
