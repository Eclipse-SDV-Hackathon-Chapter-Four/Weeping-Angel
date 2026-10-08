// Copyright (c) 2026 Peter Ulbrich
//
// This program and the accompanying materials are made available under
// the terms of the Eclipse Public License 2.0 which accompanies this
// distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
//
// AI Disclosure: This file was mostly AI-generated.
//
// SPDX-License-Identifier: EPL-2.0 and CC0-1.0
pub mod asc;
pub mod generator;
pub mod oracle;
pub mod request;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use battery_guardian::GuardianConfig;
use generator::{generate, GenerationOutcome};
use request::GenerationRequest;
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub enum RunResult {
    Generated {
        asc: PathBuf,
        ground_truth: PathBuf,
        oracle: PathBuf,
    },
    Unsatisfiable {
        result: PathBuf,
    },
}

pub fn run(request_path: &Path, output_dir: &Path) -> Result<RunResult> {
    let request = GenerationRequest::load(request_path)?;
    let injection = request.resolve_injection()?;
    let model_bytes = fs::read(&request.battery_model).with_context(|| {
        format!(
            "read Guardian model configuration {}",
            request.battery_model.display()
        )
    })?;
    let model_hash = hex::encode(Sha256::digest(&model_bytes));
    let config = GuardianConfig::load(&request.battery_model)?;
    let template = asc::AscDocument::load(&request.template)?;

    fs::create_dir_all(output_dir)
        .with_context(|| format!("create output directory {}", output_dir.display()))?;
    let stem = safe_stem(&request.injection_id);
    match generate(&request, &injection, &template, &config, &model_hash)? {
        GenerationOutcome::Generated(case) => {
            let case = *case;
            let asc_path = output_dir.join(format!("{stem}.asc"));
            let ground_truth_path = output_dir.join(format!("{stem}.ground_truth.yaml"));
            let oracle_path = output_dir.join(format!("{stem}.oracle.yaml"));
            fs::write(&asc_path, case.asc)
                .with_context(|| format!("write generated ASC {}", asc_path.display()))?;
            write_yaml(&ground_truth_path, &case.ground_truth)?;
            write_yaml(&oracle_path, &case.oracle)?;
            Ok(RunResult::Generated {
                asc: asc_path,
                ground_truth: ground_truth_path,
                oracle: oracle_path,
            })
        }
        GenerationOutcome::Unsatisfiable(result) => {
            let result_path = output_dir.join(format!("{stem}.unsatisfiable.yaml"));
            write_yaml(&result_path, &result)?;
            Ok(RunResult::Unsatisfiable {
                result: result_path,
            })
        }
    }
}

fn write_yaml(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let contents = serde_yaml::to_string(value).context("serialize YAML artifact")?;
    fs::write(path, contents).with_context(|| format!("write YAML artifact {}", path.display()))
}

fn safe_stem(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}
