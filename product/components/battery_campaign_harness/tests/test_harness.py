import importlib.util
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "harness.py"
SPEC = importlib.util.spec_from_file_location("battery_campaign_harness", SCRIPT)
HARNESS = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = HARNESS
SPEC.loader.exec_module(HARNESS)


class HarnessTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.config = HARNESS.read_configuration(HARNESS.DEFAULT_CONFIG)
        cls.guardian = HARNESS.load_yaml(HARNESS.MODEL)["guardian"]

    def test_initial_matrix_has_twenty_two_experiments(self):
        plan = HARNESS.campaign_plan(self.config)
        self.assertEqual(22, len(plan))
        self.assertEqual(21, sum(item.campaign_id != "combined_example" for item in plan))

    def test_golden_scenarios_and_sidecars_are_valid(self):
        HARNESS.validate(self.config)

    def test_standard_profiles_have_five_incidents(self):
        for campaign in (
            "signal.stuck",
            "signal.spike",
            "signal.drift",
            "signal.out_of_range",
            "transport.delay",
            "transport.drop",
            "source.dropout",
        ):
            with self.subTest(campaign=campaign):
                self.assertEqual(
                    5, len(HARNESS.standard_incidents(campaign, self.guardian))
                )

    def test_stale_boundary_is_not_a_violation(self):
        incidents = HARNESS.standard_incidents("transport.drop", self.guardian)
        self.assertEqual([], incidents[1]["goal"]["primary"])
        self.assertEqual(
            "STREAM_STALE", incidents[2]["goal"]["primary"][0]["class"]
        )

    def test_stuck_detected_incidents_carry_exactly_one_companion(self):
        incidents = HARNESS.standard_incidents("signal.stuck", self.guardian)
        self.assertEqual([], incidents[0]["mutations"][1:])
        for incident in incidents[1:]:
            mutations = incident["mutations"]
            self.assertEqual(2, len(mutations))
            self.assertEqual(
                {"temp_avg", "soc"}, {mutation["signal"] for mutation in mutations}
            )
            self.assertEqual(
                "stuck", mutations[0]["operator"], "stuck mutation is first"
            )
            self.assertEqual("drift", mutations[1]["operator"])
            self.assertEqual(
                mutations[0]["parameters"]["duration_samples"],
                mutations[1]["parameters"]["duration_samples"],
                "companion shares the stuck hold window",
            )


if __name__ == "__main__":
    unittest.main()
