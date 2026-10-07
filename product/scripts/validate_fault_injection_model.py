#!/usr/bin/env python3
"""Validate the Battery Guardian fault-injection configuration contract."""

from __future__ import annotations

import argparse
import math
from pathlib import Path
from typing import Any

import yaml


CANONICAL_SIGNALS = {"temp_min", "temp_avg", "temp_max", "soc"}
SIGNAL_CLASSES = {
    "signal.stuck": "stuck",
    "signal.spike": "spike",
    "signal.drift": "drift",
    "signal.out_of_range": "out_of_range",
}
COMBINATION_CLASS = "signal.combination"
ACTION_CLASSES = {
    "transport.delay": "delay",
    "transport.drop": "drop",
    "source.dropout": "suspend_source",
}
SUPPORTED_CLASSES = set(SIGNAL_CLASSES) | {COMBINATION_CLASS} | set(ACTION_CLASSES)
COMPONENT_OPERATORS = set(SIGNAL_CLASSES.values())


class ModelValidationError(ValueError):
    """Raised when a fault-injection model violates the canonical contract."""


def _is_number(value: object) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
    )


def _positive_int(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def _validate_parameters(operator: str, parameters: Any, path: str, errors: list[str]) -> None:
    if not isinstance(parameters, dict):
        errors.append(f"{path}.parameters must be a mapping")
        return

    expected_parameters = {
        "stuck": {"duration_samples"},
        "spike": {"delta", "duration_samples"},
        "drift": {"rate_per_sample", "duration_samples"},
        "out_of_range": {"value", "duration_samples"},
    }[operator]
    if set(parameters) != expected_parameters:
        errors.append(
            f"{path}.parameters must contain exactly {sorted(expected_parameters)}"
        )

    duration_samples = parameters.get("duration_samples")
    if not _positive_int(duration_samples):
        errors.append(f"{path}.parameters.duration_samples must be a positive integer")

    numeric_parameter = {
        "spike": "delta",
        "drift": "rate_per_sample",
        "out_of_range": "value",
    }.get(operator)
    if numeric_parameter is not None:
        value = parameters.get(numeric_parameter)
        if not _is_number(value):
            errors.append(f"{path}.parameters.{numeric_parameter} must be a finite number")
        elif operator in {"spike", "drift"} and value == 0:
            errors.append(f"{path}.parameters.{numeric_parameter} must be non-zero")


def _validate_mutations(
    mutations: Any,
    injected_class: str,
    path: str,
    errors: list[str],
) -> None:
    if not isinstance(mutations, list):
        errors.append(f"{path}.mutations must be a list")
        return

    is_combination = injected_class == COMBINATION_CLASS
    expected_length = "at least two" if is_combination else "exactly one"
    if (is_combination and len(mutations) < 2) or (not is_combination and len(mutations) != 1):
        errors.append(f"{path}.mutations must contain {expected_length} mutation(s)")

    seen_signals: set[str] = set()
    expected_operator = SIGNAL_CLASSES.get(injected_class)
    for index, mutation in enumerate(mutations):
        mutation_path = f"{path}.mutations[{index}]"
        if not isinstance(mutation, dict):
            errors.append(f"{mutation_path} must be a mapping")
            continue

        signal = mutation.get("signal")
        if signal not in CANONICAL_SIGNALS:
            errors.append(f"{mutation_path}.signal must be a canonical signal")
        elif signal in seen_signals:
            errors.append(f"{path}.mutations must target distinct signals")
        else:
            seen_signals.add(signal)

        operator = mutation.get("operator")
        if operator not in COMPONENT_OPERATORS:
            errors.append(f"{mutation_path}.operator is not a supported component operator")
            continue
        if expected_operator is not None and operator != expected_operator:
            errors.append(
                f"{mutation_path}.operator must be {expected_operator!r} for {injected_class}"
            )
        _validate_parameters(operator, mutation.get("parameters"), mutation_path, errors)


def _validate_action(action: Any, injected_class: str, path: str, errors: list[str]) -> None:
    if not isinstance(action, dict):
        errors.append(f"{path}.action must be a mapping")
        return

    expected_operator = ACTION_CLASSES[injected_class]
    expected_fields = {"operator", "duration_ms"}
    if injected_class == "transport.delay":
        expected_fields.add("delay_ms")
    if set(action) != expected_fields:
        errors.append(f"{path}.action must contain exactly {sorted(expected_fields)}")
    if action.get("operator") != expected_operator:
        errors.append(f"{path}.action.operator must be {expected_operator!r}")
    if not _positive_int(action.get("duration_ms")):
        errors.append(f"{path}.action.duration_ms must be a positive integer")
    if injected_class == "transport.delay" and not _positive_int(action.get("delay_ms")):
        errors.append(f"{path}.action.delay_ms must be a positive integer")


def validate_model(model: Any) -> None:
    """Validate a parsed model and raise ModelValidationError on any issue."""

    errors: list[str] = []
    if not isinstance(model, dict):
        raise ModelValidationError("model root must be a mapping")

    if model.get("schema_version") != 1:
        errors.append("schema_version must be 1")

    sample_stream = model.get("sample_stream")
    if not isinstance(sample_stream, dict):
        errors.append("sample_stream must be a mapping")
    else:
        if not isinstance(sample_stream.get("id"), str) or not sample_stream["id"]:
            errors.append("sample_stream.id must be a non-empty string")
        if not _positive_int(sample_stream.get("can_frame_id")):
            errors.append("sample_stream.can_frame_id must be a positive integer")

    signals = model.get("signals")
    if not isinstance(signals, dict):
        errors.append("signals must be a mapping")
    else:
        signal_names = set(signals)
        if signal_names != CANONICAL_SIGNALS:
            missing = sorted(CANONICAL_SIGNALS - signal_names)
            extra = sorted(signal_names - CANONICAL_SIGNALS)
            errors.append(f"signals must define exactly the canonical names; missing={missing}, extra={extra}")
        dbc_signals: set[str] = set()
        for name, metadata in signals.items():
            if not isinstance(metadata, dict):
                errors.append(f"signals.{name} must be a mapping")
                continue
            if not isinstance(metadata.get("unit"), str) or not metadata["unit"]:
                errors.append(f"signals.{name}.unit must be a non-empty string")
            if not isinstance(metadata.get("dbc_signal"), str) or not metadata["dbc_signal"]:
                errors.append(f"signals.{name}.dbc_signal must be a non-empty string")
            elif metadata["dbc_signal"] in dbc_signals:
                errors.append("signals must use distinct dbc_signal values")
            else:
                dbc_signals.add(metadata["dbc_signal"])

    ground_truth = model.get("ground_truth_record")
    if not isinstance(ground_truth, dict):
        errors.append("ground_truth_record must be a mapping")
    else:
        required_fields = set(ground_truth.get("required_fields", []))
        if required_fields != {"injection_id", "injected_class", "started_at"}:
            errors.append("ground_truth_record.required_fields must define the canonical fields")
        timing_fields = set(ground_truth.get("timing_fields_any_of", []))
        if timing_fields != {"finished_at", "duration_ms"}:
            errors.append("ground_truth_record.timing_fields_any_of must define finished_at and duration_ms")
        if ground_truth.get("signal_fault_required_fields") != ["mutations"]:
            errors.append("ground_truth_record.signal_fault_required_fields must contain mutations")

    injections = model.get("injections")
    if not isinstance(injections, list):
        errors.append("injections must be a list")
        injections = []

    ids: set[str] = set()
    classes_seen: set[str] = set()
    for index, injection in enumerate(injections):
        path = f"injections[{index}]"
        if not isinstance(injection, dict):
            errors.append(f"{path} must be a mapping")
            continue

        injection_id = injection.get("id")
        if not isinstance(injection_id, str) or not injection_id:
            errors.append(f"{path}.id must be a non-empty string")
        elif injection_id in ids:
            errors.append(f"{path}.id must be unique")
        else:
            ids.add(injection_id)

        injected_class = injection.get("injected_class")
        if injected_class not in SUPPORTED_CLASSES:
            errors.append(f"{path}.injected_class is not supported")
            continue
        classes_seen.add(injected_class)

        if injected_class in SIGNAL_CLASSES or injected_class == COMBINATION_CLASS:
            if "action" in injection:
                errors.append(f"{path} signal faults must not define action")
            _validate_mutations(injection.get("mutations"), injected_class, path, errors)
        else:
            if "mutations" in injection:
                errors.append(f"{path} transport/source faults must not define mutations")
            _validate_action(injection.get("action"), injected_class, path, errors)

    if classes_seen != SUPPORTED_CLASSES:
        missing = sorted(SUPPORTED_CLASSES - classes_seen)
        extra = sorted(classes_seen - SUPPORTED_CLASSES)
        errors.append(f"injections must expose every supported class; missing={missing}, extra={extra}")

    if errors:
        raise ModelValidationError("\n".join(errors))


def load_and_validate(path: Path) -> None:
    with path.open("r", encoding="utf-8") as stream:
        validate_model(yaml.safe_load(stream))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    args = parser.parse_args()
    try:
        load_and_validate(args.model)
    except (OSError, yaml.YAMLError, ModelValidationError) as error:
        parser.exit(1, f"invalid fault-injection model: {error}\n")
    print(f"valid fault-injection model: {args.model}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
