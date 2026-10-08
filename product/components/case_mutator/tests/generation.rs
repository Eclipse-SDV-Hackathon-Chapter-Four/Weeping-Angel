// Copyright (c) 2026 Peter Ulbrich
// Copyright (c) 2026 Sebastian Russer
// Copyright (c) 2026 Matthias Knöfel
// Copyright (c) 2026 Alwin Berger
//
// This program and the accompanying materials are made available under
// the terms of the Eclipse Public License 2.0 which accompanies this
// distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
//
// AI Disclosure: This file was mostly AI-generated.
//
// SPDX-License-Identifier: EPL-2.0 and CC0-1.0
// Assisted-by: Claude Opus 5.5, GLM-5.3-flash
use std::fs;
use std::path::{Path, PathBuf};

use case_mutator::{run, RunResult};
use tempfile::TempDir;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("component lives under product/components")
        .to_path_buf()
}

fn request(temp: &TempDir, injection_id: &str, injected_class: &str, goal: &str) -> PathBuf {
    let root = repository_root();
    let path = temp.path().join(format!("{injection_id}.yaml"));
    let contents = format!(
        "template: {}\nbattery_model: {}\nfault_injection_model: {}\nrun_id: test-run\nstarted_at: 2026-10-07T00:00:00Z\ninjection_id: {injection_id}\ninjected_class: {injected_class}\ngeneration_goal:\n{goal}seed: 0\nlead_in_frames: 20\n",
        root.join("product/config/battery_temp_with_ts.asc")
            .display(),
        root.join("product/config/battery_guardian/guardian_model.yaml")
            .display(),
        root.join("product/config/battery_guardian/fault_injection_model.yaml")
            .display()
    );
    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn generates_forward_verified_out_of_range_case() {
    let temp = TempDir::new().unwrap();
    let request = request(
        &temp,
        "signal_out_of_range",
        "signal.out_of_range",
        "  primary:\n    - class: PHYSICAL_TEMP_ABSOLUTE_LIMIT\n      level: VIOLATION\n  allowed:\n    - class: PHYSICAL_TEMP_SPREAD\n      level: VIOLATION\n    - class: PHYSICAL_TEMP_RATE\n      level: VIOLATION\n    - class: SIGNAL_STUCK\n      level: VIOLATION\n  forbidden: []\n  allow_unspecified_codetections: false\n",
    );
    let output = temp.path().join("output");
    let RunResult::Generated {
        asc,
        ground_truth,
        oracle,
    } = run(&request, &output).unwrap()
    else {
        panic!("case should be satisfiable");
    };
    assert!(asc.exists());
    let truth = fs::read_to_string(ground_truth).unwrap();
    assert!(truth.contains("sha256:"));
    assert!(truth.contains("source_started_at_ms: 2000"));
    let oracle = fs::read_to_string(oracle).unwrap();
    assert!(oracle.contains("PHYSICAL_TEMP_ABSOLUTE_LIMIT"));
    assert!(oracle.contains("status: SATISFIED"));
}

#[test]
fn transport_drop_preserves_source_timeline_and_predicts_stale() {
    let temp = TempDir::new().unwrap();
    let request = request(
        &temp,
        "transport_drop",
        "transport.drop",
        "  primary:\n    - class: STREAM_STALE\n      level: VIOLATION\n  allowed:\n    - class: STREAM_GENERATION_GAP\n      level: VIOLATION\n  forbidden: []\n  allow_unspecified_codetections: false\n",
    );
    let output = temp.path().join("output");
    let RunResult::Generated { asc, oracle, .. } = run(&request, &output).unwrap() else {
        panic!("drop case should be satisfiable");
    };
    let asc = fs::read_to_string(asc).unwrap();
    assert!(!asc.contains(" 2.000000 CANFD   1 Rx        100"));
    assert!(asc.contains(" 4.000000 CANFD   1 Rx        100"));
    let oracle = fs::read_to_string(oracle).unwrap();
    assert!(oracle.contains("STREAM_STALE"));
    assert!(oracle.contains("STREAM_GENERATION_GAP"));
}

#[test]
fn identical_inputs_generate_identical_artifacts() {
    let temp = TempDir::new().unwrap();
    let request = request(
        &temp,
        "transport_drop",
        "transport.drop",
        "  primary:\n    - class: STREAM_STALE\n      level: VIOLATION\n  allowed:\n    - class: STREAM_GENERATION_GAP\n      level: VIOLATION\n  forbidden: []\n  allow_unspecified_codetections: false\n",
    );
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let RunResult::Generated {
        asc: first_asc,
        ground_truth: first_truth,
        oracle: first_oracle,
    } = run(&request, &first).unwrap()
    else {
        panic!("first generation failed");
    };
    let RunResult::Generated {
        asc: second_asc,
        ground_truth: second_truth,
        oracle: second_oracle,
    } = run(&request, &second).unwrap()
    else {
        panic!("second generation failed");
    };
    assert_eq!(fs::read(first_asc).unwrap(), fs::read(second_asc).unwrap());
    assert_eq!(
        fs::read(first_truth).unwrap(),
        fs::read(second_truth).unwrap()
    );
    assert_eq!(
        fs::read(first_oracle).unwrap(),
        fs::read(second_oracle).unwrap()
    );
}

#[test]
fn every_canonical_v1_injection_can_generate_a_verified_case() {
    let cases = [
        ("signal_stuck", "signal.stuck", "SIGNAL_STUCK"),
        ("signal_spike", "signal.spike", "PHYSICAL_TEMP_RATE"),
        ("signal_drift", "signal.drift", "PHYSICAL_TEMP_SPREAD"),
        (
            "combined_temperature_fault",
            "signal.combination",
            "PHYSICAL_TEMP_RATE",
        ),
        ("transport_delay", "transport.delay", "STREAM_STALE"),
        ("transport_drop", "transport.drop", "STREAM_STALE"),
        ("source_dropout", "source.dropout", "STREAM_STALE"),
    ];

    for (injection_id, injected_class, primary_class) in cases {
        let temp = TempDir::new().unwrap();
        let goal = format!(
            "  primary:\n    - class: {primary_class}\n      level: VIOLATION\n  allowed: []\n  forbidden: []\n  allow_unspecified_codetections: true\n"
        );
        let request = request(&temp, injection_id, injected_class, &goal);
        let output = temp.path().join("output");
        assert!(
            matches!(run(&request, &output).unwrap(), RunResult::Generated { .. }),
            "{injection_id} should be satisfiable"
        );
    }
}

/// Writes a flat synthetic template (no peer excitation in the mutation
/// window) and returns its path.
fn flat_template(temp: &TempDir) -> PathBuf {
    let template = temp.path().join("flat.asc");
    let mut contents = String::from(
        "date Mon Aug 25 09:00:00 2026\n\
         base hex  timestamps absolute\n\
         internal events logged\n\
         Begin Triggerblock Mon Aug 25 09:00:00.000 2026\n",
    );
    for index in 0..40 {
        let milliseconds = (index * 100) as u32;
        let time = milliseconds as f64 / 1_000.0;
        contents.push_str(&format!(
            " {time:.6} CANFD   1 Rx        100                                   \
             0 0 a 16 {:02X} {:02X} 00 00 8C 00 8C 00 8C 00 A0 00 00 00 00 00      \
             0    0     1000        0        0        0        0        0\n",
            milliseconds & 0xFF,
            (milliseconds >> 8) & 0xFF,
        ));
    }
    contents.push_str("End Triggerblock\n");
    fs::write(&template, contents).unwrap();
    template
}

#[test]
fn unsatisfiable_stuck_writes_concrete_construction_failure_detail() {
    let temp = TempDir::new().unwrap();
    let root = repository_root();
    let template = flat_template(&temp);
    let request_path = temp.path().join("signal_stuck_flat.yaml");
    let contents = format!(
        "template: {}\n\
         battery_model: {}\n\
         run_id: construction-failure-test\n\
         started_at: 2026-10-07T00:00:00Z\n\
         injection_id: signal_stuck_flat\n\
         injected_class: signal.stuck\n\
         mutations:\n\
         \x20 - signal: temp_avg\n\
         \x20   operator: stuck\n\
         \x20   parameters:\n\
         \x20     duration_samples: 5\n\
         generation_goal:\n\
         \x20 primary:\n\
         \x20   - class: SIGNAL_STUCK\n\
         \x20     level: VIOLATION\n\
         \x20 allowed: []\n\
         \x20 forbidden: []\n\
         \x20 allow_unspecified_codetections: true\n\
         seed: 0\n\
         lead_in_frames: 20\n",
        template.display(),
        root.join("product/config/battery_guardian/guardian_model.yaml")
            .display(),
    );
    fs::write(&request_path, contents).unwrap();

    let output = temp.path().join("output");
    let RunResult::Unsatisfiable { result } = run(&request_path, &output).unwrap() else {
        panic!("stuck without peer excitation must be unsatisfiable");
    };
    let record: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(result).unwrap()).unwrap();
    assert_eq!(record["reason"]["code"], "ENCODING_LIMIT");
    let detail = record["reason"]["detail"].as_str().unwrap();
    assert!(
        detail.contains(
            "stuck trajectory has no independent excitation in its nominal or explicitly mutated peers"
        ),
        "unexpected detail: {detail}"
    );
    assert!(
        detail.starts_with("candidate 0: "),
        "unexpected detail: {detail}"
    );
}

#[test]
fn forbidden_stuck_negative_control_generates_without_excitation() {
    let temp = TempDir::new().unwrap();
    let root = repository_root();
    // Flat template: no peer excitation in the mutation window. With
    // SIGNAL_STUCK forbidden (ADR-014 negative control), the unexcited hold
    // is the wanted outcome and must NOT be rejected by the stuck gate.
    let template = flat_template(&temp);
    let request_path = temp.path().join("signal_stuck_negative_control.yaml");
    let contents = format!(
        "template: {}\n\
         battery_model: {}\n\
         run_id: negative-control-test\n\
         started_at: 2026-10-07T00:00:00Z\n\
         injection_id: signal_stuck_negative_control\n\
         injected_class: signal.stuck\n\
         mutations:\n\
         \x20 - signal: temp_avg\n\
         \x20   operator: stuck\n\
         \x20   parameters:\n\
         \x20     duration_samples: 5\n\
         generation_goal:\n\
         \x20 primary: []\n\
         \x20 allowed: []\n\
         \x20 forbidden:\n\
         \x20   - class: SIGNAL_STUCK\n\
         \x20     level: VIOLATION\n\
         \x20 allow_unspecified_codetections: true\n\
         seed: 0\n\
         lead_in_frames: 20\n",
        template.display(),
        root.join("product/config/battery_guardian/guardian_model.yaml")
            .display(),
    );
    fs::write(&request_path, contents).unwrap();

    let output = temp.path().join("output");
    let RunResult::Generated {
        ground_truth,
        oracle,
        ..
    } = run(&request_path, &output).unwrap()
    else {
        panic!("forbidden-stuck negative control must be satisfiable");
    };
    let truth = fs::read_to_string(ground_truth).unwrap();
    assert!(truth.contains("operator: stuck"));
    let oracle = fs::read_to_string(oracle).unwrap();
    assert!(oracle.contains("status: SATISFIED"));
}

#[test]
fn unsat_goal_detail_appends_observed_construction_failures() {
    let temp = TempDir::new().unwrap();
    let root = repository_root();
    // Stuck over the (nominal, exciting) template never fails construction,
    // but the drift sub-mutation probes a sub-quantum rate that is not DBC
    // representable; a utilization target of 100 leaves every constructed
    // candidate goal-failing. The record must carry BOTH reason families:
    // codes stay goal-driven, the construction failures are appended.
    let request = temp.path().join("probe_mixed.yaml");
    let contents = format!(
        "template: {}\n\
         battery_model: {}\n\
         run_id: probe-mixed\n\
         started_at: 2026-10-07T00:00:00Z\n\
         injection_id: probe_mixed\n\
         injected_class: signal.combination\n\
         mutations:\n\
         \x20 - signal: temp_avg\n\
         \x20   operator: stuck\n\
         \x20   parameters:\n\
         \x20     duration_samples: 40\n\
         \x20 - signal: temp_max\n\
         \x20   operator: drift\n\
         \x20   parameters:\n\
         \x20     rate_per_sample: 0.05\n\
         \x20     duration_samples: 1\n\
         generation_goal:\n\
         \x20 primary:\n\
         \x20   - class: PHYSICAL_TEMP_RATE\n\
         \x20     level: VIOLATION\n\
         \x20 allowed: []\n\
         \x20 forbidden: []\n\
         \x20 allow_unspecified_codetections: true\n\
         seed: 0\n\
         lead_in_frames: 20\n\
         search:\n\
         \x20 warning_target_utilization: 0.9\n\
         \x20 violation_target_utilization: 100.0\n",
        root.join("product/config/battery_temp_with_ts.asc")
            .display(),
        root.join("product/config/battery_guardian/guardian_model.yaml")
            .display(),
    );
    fs::write(&request, contents).unwrap();

    let output = temp.path().join("output");
    let RunResult::Unsatisfiable { result } = run(&request, &output).unwrap() else {
        panic!("a utilization target of 100 cannot be reached");
    };
    let record: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(result).unwrap()).unwrap();
    assert_eq!(record["reason"]["code"], "PRIMARY_NOT_REACHED");
    let detail = record["reason"]["detail"].as_str().unwrap();
    assert!(
        detail.contains("did not reach target utilization 100"),
        "unexpected detail: {detail}"
    );
    assert!(
        detail.contains(
            "; additionally observed construction failures: candidate 0: temperature value "
        ),
        "unexpected detail: {detail}"
    );
    assert!(
        detail.ends_with("is not representable with quantum 0.5"),
        "unexpected detail: {detail}"
    );
}

#[test]
fn impossible_goal_writes_structured_unsatisfiable_result() {
    let temp = TempDir::new().unwrap();
    let request = request(
        &temp,
        "signal_spike",
        "signal.spike",
        "  primary:\n    - class: SIGNAL_STUCK\n      level: VIOLATION\n  allowed: []\n  forbidden: []\n  allow_unspecified_codetections: true\n",
    );
    let output = temp.path().join("output");
    let RunResult::Unsatisfiable { result } = run(&request, &output).unwrap() else {
        panic!("one-frame spike must not satisfy a stuck goal");
    };
    let result = fs::read_to_string(result).unwrap();
    assert!(result.contains("status: UNSATISFIABLE"));
    assert!(result.contains("PRIMARY_NOT_REACHED"));
}

/// Companion excitation on a flat template: the SoC-peer drift arms the
/// Guardian stuck detector (ADR-014) inside the hold window, so a detected
/// stuck goal is satisfiable even without nominal template excitation.
#[test]
fn stuck_with_soc_companion_excitation_generates_detected_case() {
    let temp = TempDir::new().unwrap();
    let root = repository_root();
    // Flat template: no nominal excitation in the mutation window; SoC
    // baseline is 80.0 pp.
    let template = flat_template(&temp);
    let request_path = temp.path().join("signal_stuck_companion.yaml");
    let contents = format!(
        "template: {}\n\
         battery_model: {}\n\
         run_id: stuck-companion-test\n\
         started_at: 2026-10-07T00:00:00Z\n\
         injection_id: signal_stuck_companion\n\
         injected_class: signal.stuck\n\
         mutations:\n\
         \x20 - signal: temp_avg\n\
         \x20   operator: stuck\n\
         \x20   parameters:\n\
         \x20     duration_samples: 10\n\
         \x20 - signal: soc\n\
         \x20   operator: drift\n\
         \x20   parameters:\n\
         \x20     rate_per_sample: -0.5\n\
         \x20     duration_samples: 10\n\
         generation_goal:\n\
         \x20 primary:\n\
         \x20   - class: SIGNAL_STUCK\n\
         \x20     level: VIOLATION\n\
         \x20 allowed: []\n\
         \x20 forbidden: []\n\
         \x20 allow_unspecified_codetections: true\n\
         seed: 0\n\
         lead_in_frames: 20\n",
        template.display(),
        root.join("product/config/battery_guardian/guardian_model.yaml")
            .display(),
    );
    fs::write(&request_path, contents).unwrap();

    let output = temp.path().join("output");
    let RunResult::Generated {
        ground_truth,
        oracle,
        ..
    } = run(&request_path, &output).unwrap()
    else {
        panic!("stuck + SoC companion must be satisfiable on a flat template");
    };
    let truth = fs::read_to_string(ground_truth).unwrap();
    assert!(truth.contains("signal: soc"));
    assert!(truth.contains("operator: drift"));
    let oracle = fs::read_to_string(oracle).unwrap();
    assert!(oracle.contains("status: SATISFIED"));
    assert!(oracle.contains("SIGNAL_STUCK"));
}

/// The companion must not mask goal semantics: with SIGNAL_STUCK forbidden,
/// the stuck construction gate is skipped (negative-control semantics), but
/// the companion excitation itself arms the detector, so the case fails the
/// goal evaluation — UNSAT is gate-independent and honest.
#[test]
fn forbidden_goal_with_companion_fails_on_goal_not_on_the_stuck_gate() {
    let temp = TempDir::new().unwrap();
    let root = repository_root();
    let template = flat_template(&temp);
    let request_path = temp.path().join("signal_stuck_companion_negative.yaml");
    let contents = format!(
        "template: {}\n\
         battery_model: {}\n\
         run_id: stuck-companion-negative-test\n\
         started_at: 2026-10-07T00:00:00Z\n\
         injection_id: signal_stuck_companion_negative\n\
         injected_class: signal.stuck\n\
         mutations:\n\
         \x20 - signal: temp_avg\n\
         \x20   operator: stuck\n\
         \x20   parameters:\n\
         \x20     duration_samples: 10\n\
         \x20 - signal: soc\n\
         \x20   operator: drift\n\
         \x20   parameters:\n\
         \x20     rate_per_sample: -0.5\n\
         \x20     duration_samples: 10\n\
         generation_goal:\n\
         \x20 primary: []\n\
         \x20 allowed: []\n\
         \x20 forbidden:\n\
         \x20   - class: SIGNAL_STUCK\n\
         \x20     level: VIOLATION\n\
         \x20 allow_unspecified_codetections: true\n\
         seed: 0\n\
         lead_in_frames: 20\n",
        template.display(),
        root.join("product/config/battery_guardian/guardian_model.yaml")
            .display(),
    );
    fs::write(&request_path, contents).unwrap();

    let output = temp.path().join("output");
    let RunResult::Unsatisfiable { result } = run(&request_path, &output).unwrap() else {
        panic!("companion excitation must trigger the forbidden stuck observation");
    };
    let record: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(result).unwrap()).unwrap();
    assert_eq!(record["reason"]["code"], "FORBIDDEN_CODETECTION");
}
