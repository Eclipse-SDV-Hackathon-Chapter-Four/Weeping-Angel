#!/usr/bin/env python3
"""Generate and validate deterministic Battery Guardian campaign bundles."""

from __future__ import annotations

import argparse
import math
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml


ROOT = Path(__file__).resolve().parents[3]
DEFAULT_CONFIG = ROOT / "product/config/battery_campaign/harness.yaml"
MODEL = ROOT / "product/config/battery_guardian/guardian_model.yaml"
MANIFEST = ROOT / "product/components/case_mutator/Cargo.toml"
SLOT_STARTS_MS = (1000, 4500, 8000, 11500, 15000)
EXPECTED_SOURCE_TIMESTAMPS = tuple(range(0, 20_000, 100))


class HarnessError(RuntimeError):
    pass


@dataclass(frozen=True)
class Experiment:
    campaign_id: str
    scenario_id: str
    definition: dict[str, Any]

    @property
    def experiment_id(self) -> str:
        return f"{self.campaign_id}--{self.scenario_id}"


def load_yaml(path: Path) -> Any:
    try:
        with path.open(encoding="utf-8") as stream:
            return yaml.safe_load(stream)
    except (OSError, yaml.YAMLError) as error:
        raise HarnessError(f"cannot load {path}: {error}") from error


def dump_yaml(path: Path, value: Any) -> None:
    path.write_text(
        yaml.safe_dump(value, sort_keys=False, allow_unicode=True), encoding="utf-8"
    )


def repo_path(value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else ROOT / path


def read_configuration(path: Path) -> dict[str, Any]:
    config = load_yaml(path)
    if not isinstance(config, dict) or config.get("schema_version") != 1:
        raise HarnessError("harness configuration must use schema_version: 1")
    for key in ("scenarios", "scenario_groups", "campaign_files", "output_dir"):
        if key not in config:
            raise HarnessError(f"harness configuration is missing {key!r}")
    config["_directory"] = path.parent
    return config


def campaign_plan(config: dict[str, Any]) -> list[Experiment]:
    experiments: list[Experiment] = []
    for name in config["campaign_files"]:
        campaign_file = load_yaml(config["_directory"] / name)
        if not isinstance(campaign_file, dict) or campaign_file.get("schema_version") != 1:
            raise HarnessError(f"campaign file {name} must use schema_version: 1")
        if "campaigns" in campaign_file:
            defaults = campaign_file.get("defaults", {})
            group = defaults.get("scenarios")
            scenarios = config["scenario_groups"].get(group)
            if not scenarios:
                raise HarnessError(f"campaign file {name} references unknown group {group!r}")
            if defaults.get("incidents") != 5 or defaults.get("schedule") != "even":
                raise HarnessError(f"campaign file {name} must use five even incidents")
            for campaign in campaign_file["campaigns"]:
                campaign_id = campaign if isinstance(campaign, str) else campaign.get("id")
                if not campaign_id:
                    raise HarnessError(f"campaign file {name} contains an invalid campaign")
                for scenario_id in scenarios:
                    experiments.append(
                        Experiment(campaign_id, scenario_id, {"variation": "standard"})
                    )
        elif "campaign" in campaign_file:
            campaign = campaign_file["campaign"]
            incidents = campaign_file.get("incidents", [])
            if len(incidents) != 5:
                raise HarnessError(f"combined campaign {name} must contain five incidents")
            experiments.append(
                Experiment(campaign["id"], campaign["scenario"], campaign_file)
            )
        else:
            raise HarnessError(f"campaign file {name} has no campaign definition")
    return experiments


def parse_asc(path: Path) -> list[tuple[int, int]]:
    frames: list[tuple[int, int]] = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        tokens = line.split()
        if len(tokens) < 25 or tokens[1] != "CANFD" or tokens[4].lower() != "100":
            continue
        try:
            if tokens[8] != "16":
                raise HarnessError(f"{path}:{number}: battery frame is not 16-byte CAN FD")
            arrival_ms = round(float(tokens[0]) * 1000)
            payload = [int(value, 16) for value in tokens[9:25]]
        except ValueError as error:
            raise HarnessError(f"{path}:{number}: malformed battery frame") from error
        source_ms = int.from_bytes(bytes(payload[:4]), "little")
        frames.append((arrival_ms, source_ms))
    if not frames:
        raise HarnessError(f"{path} contains no canonical battery CAN FD frames")
    return frames


def validate(config: dict[str, Any]) -> None:
    for scenario_id, prefix_value in config["scenarios"].items():
        prefix = repo_path(prefix_value)
        paths = [
            Path(f"{prefix}.asc"),
            Path(f"{prefix}.ground_truth.yaml"),
            Path(f"{prefix}.oracle.yaml"),
        ]
        missing = [str(path) for path in paths if not path.is_file()]
        if missing:
            raise HarnessError(f"scenario {scenario_id} is incomplete: {', '.join(missing)}")
        frames = parse_asc(paths[0])
        source = tuple(item[1] for item in frames)
        if source != EXPECTED_SOURCE_TIMESTAMPS:
            raise HarnessError(
                f"scenario {scenario_id} must contain source timestamps 0..19900 in 100-ms steps"
            )
        ground_truth = load_yaml(paths[1])
        oracle = load_yaml(paths[2])
        if ground_truth != []:
            raise HarnessError(f"Golden scenario {scenario_id} must have empty ground truth")
        if not isinstance(oracle, dict) or oracle.get("scenario_id") != scenario_id:
            raise HarnessError(f"scenario {scenario_id} has a mismatching Oracle")
    experiments = campaign_plan(config)
    if len(experiments) != 22:
        raise HarnessError(f"initial campaign matrix must contain 22 experiments, got {len(experiments)}")


def observation(class_name: str, level: str) -> dict[str, str]:
    return {"class": class_name, "level": level}


def goal(
    primary: list[dict[str, str]] | None = None,
    forbidden: list[dict[str, str]] | None = None,
) -> dict[str, Any]:
    return {
        "primary": primary or [],
        "allowed": [],
        "forbidden": forbidden or [],
        "allow_unspecified_codetections": True,
    }


def standard_incidents(campaign: str, guardian: dict[str, Any]) -> list[dict[str, Any]]:
    stuck_n = int(guardian["stuck"]["window_samples"])
    absolute_min = float(guardian["temperature"]["absolute_min_c"])
    absolute_max = float(guardian["temperature"]["absolute_max_c"])
    incidents: list[dict[str, Any]] = []
    if campaign == "signal.stuck":
        durations = [stuck_n - 1, stuck_n, stuck_n + 1, math.ceil(1.5 * stuck_n), 2 * stuck_n]
        for index, duration in enumerate(durations):
            detected = index > 0
            mutations = [
                {"signal": "temp_avg", "operator": "stuck", "parameters": {"duration_samples": duration}}
            ]
            if detected:
                # Companion excitation arms the Guardian stuck detector
                # (ADR-014). The measured per-signal static amplitudes inside
                # the hold windows of the golden templates stay at or below
                # 0.5 °C / 1.0 pp, so a stuck-only hold never reaches
                # `stuck.temperature_excitation_c` = 1.0. The companion drifts
                # SoC downward at 0.5 pp per sample = 5.0 pp/second, exactly
                # the SoC-rate limit: peak-to-peak excitation is
                # 0.5 * duration pp (>= 1.0 pp for every detected incident)
                # and the cumulative SoC stays inside its DBC range
                # (>= 45 pp nominal baseline). The 0.25 pp/sample candidate
                # is NOT DBC-representable (quantum 0.5 pp) and was rejected
                # by forward-verified generation; a temperature companion of
                # the same rate would sit exactly on the thermal rate limits
                # and couple into ordering/hotspot, so SoC was chosen.
                # Steps are negative to keep headroom against 100 pp.
                mutations.append(
                    {
                        "signal": "soc",
                        "operator": "drift",
                        "parameters": {"rate_per_sample": -0.5, "duration_samples": duration},
                    }
                )
            incidents.append(
                {
                    "mutations": mutations,
                    "goal": goal(
                        [observation("SIGNAL_STUCK", "VIOLATION")] if detected else [],
                        [] if detected else [observation("SIGNAL_STUCK", "VIOLATION")],
                    ),
                    "search": {"exact_parameters": True},
                }
            )
    elif campaign in ("signal.spike", "signal.drift"):
        operator = "spike" if campaign.endswith("spike") else "drift"
        class_name = "PHYSICAL_TEMP_RATE" if operator == "spike" else "PHYSICAL_TEMP_HOTSPOT"
        levels = [None, ("WARNING", 0.9), ("WARNING", 1.0), ("VIOLATION", 1.000001), ("VIOLATION", 1.25)]
        for index, level in enumerate(levels):
            parameters: dict[str, Any] = {
                "duration_samples": 1 if operator == "spike" or index == 0 else 20
            }
            parameters["delta" if operator == "spike" else "rate_per_sample"] = 0.5
            if level is None:
                incident_goal = goal([], [observation(class_name, "WARNING"), observation(class_name, "VIOLATION")])
                search = {"exact_parameters": True}
            else:
                level_name, utilization = level
                incident_goal = goal([observation(class_name, level_name)])
                search = {
                    "warning_target_utilization": min(utilization, 1.0),
                    "violation_target_utilization": utilization if utilization > 1.0 else 1.1,
                }
            incidents.append(
                {
                    "mutations": [{"signal": "temp_max", "operator": operator, "parameters": parameters}],
                    "goal": incident_goal,
                    "search": search,
                }
            )
    elif campaign == "signal.out_of_range":
        values = [
            ("temp_min", absolute_min, False),
            ("temp_min", absolute_min - 0.5, True),
            ("temp_max", absolute_max, False),
            ("temp_max", absolute_max + 0.5, True),
            ("temp_max", absolute_max + 5.0, True),
        ]
        target = observation("PHYSICAL_TEMP_ABSOLUTE_LIMIT", "VIOLATION")
        for signal, value, detected in values:
            incidents.append(
                {
                    "mutations": [{"signal": signal, "operator": "out_of_range", "parameters": {"duration_samples": 5, "value": value}}],
                    "goal": goal([target] if detected else [], [] if detected else [target]),
                    "search": {"exact_parameters": True},
                }
            )
    elif campaign in ("transport.delay", "transport.drop", "source.dropout"):
        operator = {"transport.delay": "delay", "transport.drop": "drop", "source.dropout": "suspend_source"}[campaign]
        stale = observation("STREAM_STALE", "VIOLATION")
        for gap in (400, 500, 700, 1000, 2000):
            action: dict[str, Any] = {"operator": operator, "duration_ms": gap - 100}
            if operator == "delay":
                action["duration_ms"] = 100
                action["delay_ms"] = gap - 100
            detected = gap > int(guardian["missing_packet_timeout_ms"])
            incidents.append(
                {
                    "action": action,
                    "goal": goal([stale] if detected else [], [] if detected else [stale]),
                    "search": {"exact_parameters": True},
                }
            )
    else:
        raise HarnessError(f"unsupported default campaign {campaign!r}")
    return incidents


def combined_incidents(definition: dict[str, Any], guardian: dict[str, Any]) -> list[dict[str, Any]]:
    stuck_n = int(guardian["stuck"]["window_samples"])
    absolute_min = float(guardian["temperature"]["absolute_min_c"])
    absolute_max = float(guardian["temperature"]["absolute_max_c"])
    result = []
    for item in definition["incidents"]:
        mutations = []
        for fault in item["combine"]:
            strength = fault["strength"]
            operator = fault["fault"].split(".", 1)[1]
            signal = fault["signal"]
            if operator == "stuck":
                duration = {"low": stuck_n, "medium": math.ceil(1.5 * stuck_n), "high": 2 * stuck_n}[strength]
                parameters = {"duration_samples": duration}
            elif operator == "spike":
                parameters = {"duration_samples": 1, "delta": {"low": 1.0, "medium": 3.0, "high": 8.0}[strength]}
            elif operator == "drift":
                parameters = {"duration_samples": 20, "rate_per_sample": {"low": 0.5, "medium": 1.0, "high": 2.0}[strength]}
            elif operator == "out_of_range":
                quanta = {"low": 1, "medium": 5, "high": 10}[strength]
                bound = absolute_min if signal == "temp_min" else absolute_max
                direction = -1 if signal == "temp_min" else 1
                parameters = {"duration_samples": 5, "value": bound + direction * 0.5 * quanta}
            else:
                raise HarnessError(f"unsupported combined fault {fault['fault']!r}")
            mutations.append({"signal": signal, "operator": operator, "parameters": parameters})
        result.append(
            {
                "at_ms": int(item["at_ms"]),
                "mutations": mutations,
                "goal": goal(),
                "search": {"exact_parameters": True},
            }
        )
    return result


def _runnable(path: Path) -> bool:
    try:
        subprocess.run(
            [str(path), "--help"], cwd=ROOT,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30,
        )
    except (OSError, subprocess.SubprocessError):
        return False
    return True


def binary_paths(arguments: argparse.Namespace) -> tuple[Path, Path]:
    mutator = Path(arguments.mutator) if arguments.mutator else MANIFEST.parent / "target/debug/case-mutator"
    oracle = Path(arguments.oracle) if arguments.oracle else MANIFEST.parent / "target/debug/case-oracle"
    # A stale cross-toolchain binary (e.g. a nix-built artifact shared with the
    # host inside a container) exists but cannot be executed (ELF interpreter
    # missing) — probe with --help instead of trusting the file.
    if mutator.is_file() and oracle.is_file() and _runnable(mutator) and _runnable(oracle):
        return mutator, oracle
    command = ["cargo", "build", "--manifest-path", str(MANIFEST), "--locked", "--bins"]
    try:
        subprocess.run(command, cwd=ROOT, check=True)
    except (OSError, subprocess.CalledProcessError) as error:
        raise HarnessError(f"cannot build Case Mutator binaries: {error}") from error
    return mutator, oracle


def visible_lead_in(path: Path, at_ms: int) -> int:
    frames = parse_asc(path)
    for index, (_, source_ms) in enumerate(frames):
        if source_ms >= at_ms:
            if index == 0:
                raise HarnessError("incident has no lead-in frame")
            return index
    raise HarnessError(f"incident start {at_ms} ms is outside {path}")


def safe_stem(value: str) -> str:
    return "".join(character if character.isalnum() or character in "-_" else "-" for character in value)


def write_unsatisfiable(destination: Path, experiment: Experiment, incident: int, detail: Any) -> None:
    destination.mkdir(parents=True, exist_ok=False)
    dump_yaml(
        destination / "unsatisfiable.yaml",
        {
            "schema_version": 1,
            "status": "UNSATISFIABLE",
            "experiment_id": experiment.experiment_id,
            "campaign_id": experiment.campaign_id,
            "scenario_id": experiment.scenario_id,
            "incident": incident,
            "detail": detail,
        },
    )


def generate_experiment(
    experiment: Experiment,
    config: dict[str, Any],
    guardian: dict[str, Any],
    mutator: Path,
    oracle: Path,
    output_root: Path,
) -> str:
    destination = output_root / experiment.campaign_id / experiment.scenario_id
    if destination.exists():
        raise HarnessError(f"refusing to overwrite existing experiment {destination}")
    scenario_prefix = repo_path(config["scenarios"][experiment.scenario_id])
    source_asc = Path(f"{scenario_prefix}.asc")
    incidents = (
        combined_incidents(experiment.definition, guardian)
        if "incidents" in experiment.definition
        else standard_incidents(experiment.campaign_id, guardian)
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="battery-campaign-", dir=destination.parent) as temporary:
        work = Path(temporary)
        current = work / "input.asc"
        shutil.copyfile(source_asc, current)
        ground_truth = []
        for index, incident in enumerate(incidents, 1):
            at_ms = int(incident.get("at_ms", SLOT_STARTS_MS[index - 1]))
            injection_id = f"{safe_stem(experiment.campaign_id)}-{index}"
            request: dict[str, Any] = {
                "template": str(current),
                "battery_model": str(MODEL),
                "run_id": experiment.experiment_id,
                "started_at": "1970-01-01T00:00:00Z",
                "injection_id": injection_id,
                "injected_class": "signal.combination" if "incidents" in experiment.definition else experiment.campaign_id,
                "generation_goal": incident["goal"],
                "lead_in_frames": visible_lead_in(current, at_ms),
                "search": {
                    "warning_target_utilization": 0.9,
                    "violation_target_utilization": 1.1,
                    "max_candidate_quanta": 64,
                    **incident.get("search", {}),
                },
            }
            if "mutations" in incident:
                request["mutations"] = incident["mutations"]
            else:
                request["action"] = incident["action"]
            request_path = work / f"request-{index}.yaml"
            incident_dir = work / f"incident-{index}"
            dump_yaml(request_path, request)
            completed = subprocess.run(
                [str(mutator), "--request", str(request_path), "--output-dir", str(incident_dir)],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            if completed.returncode == 2:
                result_files = list(incident_dir.glob("*.unsatisfiable.yaml"))
                detail = load_yaml(result_files[0]) if result_files else completed.stderr.strip()
                write_unsatisfiable(destination, experiment, index, detail)
                return "UNSATISFIABLE"
            if completed.returncode != 0:
                raise HarnessError(
                    f"Mutator failed for {experiment.experiment_id} incident {index}: {completed.stderr.strip()}"
                )
            stem = safe_stem(injection_id)
            current = incident_dir / f"{stem}.asc"
            record = load_yaml(incident_dir / f"{stem}.ground_truth.yaml")
            record.pop("battery_model", None)
            record["started_at"] = request["started_at"]
            ground_truth.append(record)

        staging = work / "bundle"
        staging.mkdir()
        shutil.copyfile(current, staging / "case.asc")
        dump_yaml(staging / "case.ground_truth.yaml", ground_truth)
        subprocess.run(
            [
                str(oracle),
                "--asc", str(staging / "case.asc"),
                "--battery-model", str(MODEL),
                "--scenario-id", experiment.experiment_id,
                "--output", str(staging / "case.oracle.yaml"),
            ],
            cwd=ROOT,
            check=True,
        )
        dump_yaml(
            staging / "experiment.yaml",
            {
                "schema_version": 1,
                "experiment_id": experiment.experiment_id,
                "campaign_id": experiment.campaign_id,
                "scenario_id": experiment.scenario_id,
                "source_window_ms": {"start": 0, "end": 19_900},
                "files": {
                    "asc": "case.asc",
                    "ground_truth": "case.ground_truth.yaml",
                    "oracle": "case.oracle.yaml",
                },
            },
        )
        os.replace(staging, destination)
    return "GENERATED"


def selected_plan(config: dict[str, Any], arguments: argparse.Namespace) -> list[Experiment]:
    experiments = campaign_plan(config)
    if getattr(arguments, "campaign", None):
        experiments = [item for item in experiments if item.campaign_id == arguments.campaign]
    if getattr(arguments, "scenario", None):
        experiments = [item for item in experiments if item.scenario_id == arguments.scenario]
    if not experiments:
        raise HarnessError("selection contains no experiments")
    return experiments


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    subcommands = result.add_subparsers(dest="command", required=True)
    subcommands.add_parser("validate", help="validate scenarios and campaign configuration")
    plan = subcommands.add_parser("plan", help="print the selected experiment matrix")
    generate = subcommands.add_parser("generate", help="pre-generate selected experiment bundles")
    for command in (plan, generate):
        command.add_argument("--campaign")
        command.add_argument("--scenario")
    generate.add_argument("--output-dir", type=Path)
    generate.add_argument("--mutator", help="path to a prebuilt case-mutator")
    generate.add_argument("--oracle", help="path to a prebuilt case-oracle")
    return result


def main() -> int:
    arguments = parser().parse_args()
    try:
        config = read_configuration(arguments.config.resolve())
        if arguments.command == "validate":
            validate(config)
            print(f"valid: 5 scenarios, {len(campaign_plan(config))} experiments")
            return 0
        experiments = selected_plan(config, arguments)
        if arguments.command == "plan":
            for item in experiments:
                print(f"{item.campaign_id}\t{item.scenario_id}")
            print(f"total: {len(experiments)}")
            return 0
        validate(config)
        mutator, oracle = binary_paths(arguments)
        model = load_yaml(MODEL)["guardian"]
        output_root = arguments.output_dir or repo_path(config["output_dir"])
        counts = {"GENERATED": 0, "UNSATISFIABLE": 0}
        for item in experiments:
            status = generate_experiment(item, config, model, mutator, oracle, output_root)
            counts[status] += 1
            print(f"{status.lower()}: {item.experiment_id}")
        print(f"generated: {counts['GENERATED']}; unsatisfiable: {counts['UNSATISFIABLE']}")
        return 0
    except (HarnessError, OSError, subprocess.CalledProcessError) as error:
        print(f"battery-campaign-harness: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
