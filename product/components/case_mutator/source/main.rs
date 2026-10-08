// Copyright (c) 2026 Peter Ulbrich
//
// This program and the accompanying materials are made available under
// the terms of the Eclipse Public License 2.0 which accompanies this
// distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
//
// AI Disclosure: This file was mostly AI-generated.
//
// SPDX-License-Identifier: EPL-2.0 and CC0-1.0
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};
use case_mutator::{run, RunResult};

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        println!("Usage: case-mutator --request <request.yaml> --output-dir <directory>");
        return ExitCode::SUCCESS;
    }

    match execute(arguments.into_iter()) {
        Ok(RunResult::Generated {
            asc,
            ground_truth,
            oracle,
        }) => {
            println!("generated ASC:         {}", asc.display());
            println!("injection ground truth: {}", ground_truth.display());
            println!("test oracle:           {}", oracle.display());
            ExitCode::SUCCESS
        }
        Ok(RunResult::Unsatisfiable { result }) => {
            eprintln!("generation request is unsatisfiable: {}", result.display());
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("case-mutator failed: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn execute(mut arguments: impl Iterator<Item = String>) -> Result<RunResult> {
    let mut request = None;
    let mut output_dir = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--request" => request = arguments.next().map(PathBuf::from),
            "--output-dir" => output_dir = arguments.next().map(PathBuf::from),
            _ => bail!("unknown argument {argument:?}"),
        }
    }
    let request = request.ok_or_else(|| anyhow::anyhow!("--request is required"))?;
    let output_dir = output_dir.ok_or_else(|| anyhow::anyhow!("--output-dir is required"))?;
    run(&request, &output_dir)
}
