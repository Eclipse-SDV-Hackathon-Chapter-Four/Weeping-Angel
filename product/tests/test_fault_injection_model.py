# Copyright (c) 2026 Peter Ulbrich
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
from copy import deepcopy
from pathlib import Path
import unittest

import yaml

from scripts.validate_fault_injection_model import ModelValidationError, validate_model


MODEL_PATH = (
    Path(__file__).resolve().parents[1]
    / "config"
    / "battery_guardian"
    / "fault_injection_model.yaml"
)


class FaultInjectionModelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.model = yaml.safe_load(MODEL_PATH.read_text(encoding="utf-8"))

    def assert_invalid(self, model: dict, message: str) -> None:
        with self.assertRaisesRegex(ModelValidationError, message):
            validate_model(model)

    def injection(self, model: dict, injected_class: str) -> dict:
        return next(
            injection
            for injection in model["injections"]
            if injection["injected_class"] == injected_class
        )

    def test_supplied_model_is_valid(self) -> None:
        validate_model(deepcopy(self.model))

    def test_single_signal_class_rejects_multiple_mutations(self) -> None:
        model = deepcopy(self.model)
        injection = self.injection(model, "signal.spike")
        injection["mutations"].append(deepcopy(injection["mutations"][0]))
        injection["mutations"][1]["signal"] = "temp_avg"
        self.assert_invalid(model, "exactly one")

    def test_combination_rejects_one_mutation(self) -> None:
        model = deepcopy(self.model)
        injection = self.injection(model, "signal.combination")
        injection["mutations"] = injection["mutations"][:1]
        self.assert_invalid(model, "at least two")

    def test_combination_rejects_duplicate_signal_targets(self) -> None:
        model = deepcopy(self.model)
        injection = self.injection(model, "signal.combination")
        injection["mutations"][1]["signal"] = injection["mutations"][0]["signal"]
        self.assert_invalid(model, "distinct signals")

    def test_combination_rejects_nested_operator(self) -> None:
        model = deepcopy(self.model)
        injection = self.injection(model, "signal.combination")
        injection["mutations"][0]["operator"] = "combination"
        self.assert_invalid(model, "supported component operator")

    def test_single_signal_class_rejects_mismatched_operator(self) -> None:
        model = deepcopy(self.model)
        injection = self.injection(model, "signal.spike")
        injection["mutations"][0] = {
            "signal": "temp_max",
            "operator": "drift",
            "parameters": {"rate_per_sample": 0.5, "duration_samples": 2},
        }
        self.assert_invalid(model, "must be 'spike'")

    def test_transport_fault_rejects_signal_mutations(self) -> None:
        model = deepcopy(self.model)
        injection = self.injection(model, "transport.drop")
        injection["mutations"] = [
            {
                "signal": "temp_avg",
                "operator": "stuck",
                "parameters": {"duration_samples": 2},
            }
        ]
        self.assert_invalid(model, "must not define mutations")

    def test_signal_fault_rejects_action(self) -> None:
        model = deepcopy(self.model)
        injection = self.injection(model, "signal.stuck")
        injection["action"] = {"operator": "drop", "duration_ms": 1}
        self.assert_invalid(model, "must not define action")

    def test_transport_delay_requires_delay_parameter(self) -> None:
        model = deepcopy(self.model)
        injection = self.injection(model, "transport.delay")
        del injection["action"]["delay_ms"]
        self.assert_invalid(model, "delay_ms")

    def test_noncanonical_signal_is_rejected(self) -> None:
        model = deepcopy(self.model)
        injection = self.injection(model, "signal.stuck")
        injection["mutations"][0]["signal"] = "CellTempAvg"
        self.assert_invalid(model, "canonical signal")


if __name__ == "__main__":
    unittest.main()
