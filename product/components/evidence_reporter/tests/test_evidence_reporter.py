# Copyright (c) 2026 Alwin Berger
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
"""Tests for the Evidence Reporter (ADR-018)."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "source"))

import evidence_reporter as er  # noqa: E402

GOLDEN = HERE / "fixtures" / "golden" / "campaign-20261007-000000"
FIXED = "1970-01-01T00:00:00Z"


def golden_catalog() -> dict:
    return er.load_catalog(er.default_catalog_path())


def write_minimal_run(root: Path, verdict: str = "PASS") -> Path:
    """Create a minimal run directory with one injection and one transition."""
    run = root / "reports" / "campaign-x" / "signal.spike--warm_nominal"
    run.mkdir(parents=True)
    report = {
        "case": "/tmp/case",
        "verdict": verdict,
        "replay_end_ms": 5000,
        "battery_end_ms": 4900,
        "oracle": {"expected": 1, "not_applicable": 0, "matched": 1 if verdict == "PASS" else 0,
                   "missing": 0 if verdict == "PASS" else 1, "not_reached": 0,
                   "unexpected": 0, "allow_unspecified": False},
        "injections": [{
            "run_id": "signal.spike--warm_nominal", "injection_id": "signal-spike-1",
            "injected_class": "signal.spike", "source_started_at_ms": 1000,
            "source_finished_at_ms": 1100, "duration_ms": 100,
            "mutations": [{"signal": "temp_max", "operator": "spike",
                           "requested_parameters": {"delta": 2.0}, "executed_values": [42.0]}],
            "window_ms": [1000, 2000], "slot_ms": [1000, 4500], "verdict": verdict,
            "expected": 1, "matched": 1 if verdict == "PASS" else 0,
            "missing": 0 if verdict == "PASS" else 1, "unexpected": 0, "sovd_missing": 0,
        }],
        "transitions": [{
            "status": "MATCHED", "fault_id": "BatteryTempRate",
            "detection_class": "PHYSICAL_TEMP_RATE", "level": "VIOLATION",
            "stage": "Failed", "expected_at_ms": 1000, "observed_at_ms": 1000,
            "signal": "temp_max", "sovd_visible_at_ms": 1076, "sovd_missing": False,
            "injection_id": "signal-spike-1",
        }],
        "fault_events": [], "sovd": None, "notes": ["synthetic note"],
    }
    (run / "report.json").write_text(json.dumps(report), encoding="utf-8")
    return run


class GoldenTests(unittest.TestCase):
    def test_run_report_matches_golden(self):
        run_dir = GOLDEN / "signal.spike--warm_nominal"
        report = er.load_json(run_dir / "report.json")
        text = er.render_run_report(run_dir, report, golden_catalog(), FIXED)
        expected = (run_dir / "report.md").read_text(encoding="utf-8")
        self.assertEqual(text, expected)

    def test_campaign_report_matches_golden(self):
        text = er.render_campaign_report(GOLDEN, golden_catalog(), FIXED)
        expected = (GOLDEN / "evidence_report.md").read_text(encoding="utf-8")
        self.assertEqual(text, expected)

    def test_rendering_is_deterministic(self):
        run_dir = GOLDEN / "signal.spike--warm_nominal"
        report = er.load_json(run_dir / "report.json")
        first = er.render_run_report(run_dir, report, golden_catalog(), FIXED)
        second = er.render_run_report(run_dir, report, golden_catalog(), FIXED)
        self.assertEqual(first, second)


class RunReportTests(unittest.TestCase):
    def test_pass_verdict_and_rationale(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = write_minimal_run(Path(tmp), "PASS")
            report = er.load_json(run / "report.json")
            text = er.render_run_report(run, report, golden_catalog(), FIXED)
            self.assertIn("**Verdict:** **PASS**", text)
            self.assertIn("All 1 expected Guardian change(s) matched", text)
            self.assertIn("signal.spike", text)
            self.assertIn("synthetic note", text)

    def test_fail_verdict_and_notes(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = write_minimal_run(Path(tmp), "FAIL")
            report = er.load_json(run / "report.json")
            text = er.render_run_report(run, report, golden_catalog(), FIXED)
            self.assertIn("**Verdict:** **FAIL**", text)
            self.assertIn("1 missing", text)

    def test_github_safe_subset(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = write_minimal_run(Path(tmp), "PASS")
            report = er.load_json(run / "report.json")
            text = er.render_run_report(run, report, golden_catalog(), FIXED)
            for forbidden in ("<script", "<iframe", "<style", "javascript:"):
                self.assertNotIn(forbidden, text)

    def test_data_gaps_are_explicit_placeholders(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = write_minimal_run(Path(tmp), "PASS")
            report = er.load_json(run / "report.json")
            text = er.render_run_report(run, report, golden_catalog(), FIXED)
            self.assertIn("Mitigation:", text)
            self.assertIn("testFailed", text)
            self.assertIn("Unmapped Guardian warnings", text)

    def test_observer_artifacts_linked_and_embedded(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = write_minimal_run(Path(tmp), "PASS")
            (run / "observer.html").write_text("<html></html>", encoding="utf-8")
            (run / "observer.png").write_bytes(b"\x89PNG")
            report = er.load_json(run / "report.json")
            text = er.render_run_report(run, report, golden_catalog(), FIXED)
            self.assertIn("[observer.html](observer.html)", text)
            self.assertIn("![Final observer state](observer.png)", text)

    def test_missing_vs_present_observer_section(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = write_minimal_run(Path(tmp), "PASS")
            report = er.load_json(run / "report.json")
            text = er.render_run_report(run, report, golden_catalog(), FIXED)
            self.assertIn("No observer artifacts captured", text)


class CampaignReportTests(unittest.TestCase):
    def test_totals_and_skip_reason(self):
        text = er.render_campaign_report(GOLDEN, golden_catalog(), FIXED)
        self.assertIn("**Runs:** 2 (PASS: 1, SKIPPED: 1)", text)
        self.assertIn("ENCODING_LIMIT", text)

    def test_consistency_columns(self):
        text = er.render_campaign_report(GOLDEN, golden_catalog(), FIXED, compare_dir=GOLDEN)
        self.assertIn("Consistency across campaigns", text)
        self.assertIn("match", text)

    def test_coverage_includes_skipped_classes(self):
        text = er.render_campaign_report(GOLDEN, golden_catalog(), FIXED)
        self.assertIn("signal.stuck--cold_nominal=SKIPPED (ENCODING_LIMIT)", text)
        self.assertNotIn("| Signal | signal.stuck | not attempted |", text)


class CliTests(unittest.TestCase):
    def test_run_cli_writes_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = write_minimal_run(Path(tmp), "PASS")
            out = run / "generated.md"
            rc = er.main(["run", str(run), "--generated-at", FIXED, "--out", str(out)])
            self.assertEqual(rc, 0)
            self.assertTrue(out.is_file())

    def test_run_cli_missing_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            rc = er.main(["run", tmp])
            self.assertEqual(rc, 1)

    def test_campaign_cli_writes_reports(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = write_minimal_run(Path(tmp), "PASS")
            campaign = run.parent
            rc = er.main(["campaign", str(campaign), "--generated-at", FIXED])
            self.assertEqual(rc, 0)
            self.assertTrue((campaign / "evidence_report.md").is_file())
            self.assertTrue((run / "report.md").is_file())


class UnsatFallbackTests(unittest.TestCase):
    def test_stdlib_fallback_parses_code_and_detail(self):
        path = GOLDEN / "experiments" / "signal.stuck" / "cold_nominal" / "unsatisfiable.yaml"
        parsed = er._parse_unsatisfiable(path)
        reason = parsed["detail"]["reason"]
        self.assertEqual(reason["code"], "ENCODING_LIMIT")
        self.assertIn("no DBC-representable candidate trajectory", reason["detail"])


if __name__ == "__main__":
    unittest.main()
