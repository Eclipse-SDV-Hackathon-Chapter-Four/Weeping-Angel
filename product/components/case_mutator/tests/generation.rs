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
        "  primary:\n    - class: STREAM_STALE\n      level: VIOLATION\n  allowed: []\n  forbidden: []\n  allow_unspecified_codetections: false\n",
    );
    let output = temp.path().join("output");
    let RunResult::Generated { asc, oracle, .. } = run(&request, &output).unwrap() else {
        panic!("drop case should be satisfiable");
    };
    let asc = fs::read_to_string(asc).unwrap();
    assert!(!asc.contains("   2.000000 1  100"));
    assert!(asc.contains("   4.000000 1  100"));
    assert!(fs::read_to_string(oracle).unwrap().contains("STREAM_STALE"));
}

#[test]
fn identical_inputs_generate_identical_artifacts() {
    let temp = TempDir::new().unwrap();
    let request = request(
        &temp,
        "transport_drop",
        "transport.drop",
        "  primary:\n    - class: STREAM_STALE\n      level: VIOLATION\n  allowed: []\n  forbidden: []\n  allow_unspecified_codetections: false\n",
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
