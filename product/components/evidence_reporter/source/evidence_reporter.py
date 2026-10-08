#!/usr/bin/env python3
# Copyright (c) 2026 Alwin Berger
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
"""Evidence Reporter — render Evidence Collector JSON into Markdown (ADR-018).

Presentation only: every statement is derived from the collector ``report.json``
and its sibling bundle files. The reporter never re-evaluates Guardian decisions
or recomputes verdicts, and the generated Markdown contains no fact that is not
present in the JSON (test-harness spec section 8.6).

Usage::

    evidence_reporter.py run <run-dir> [--diagnostics FILE]
    evidence_reporter.py campaign <campaign-dir> [--compare DIR] [--diagnostics FILE]

Generated reports are not committed (ADR-018).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

try:  # PyYAML is the only non-stdlib dependency (ADR-018).
    import yaml
except ImportError:  # pragma: no cover - exercised only without PyYAML
    yaml = None

DIAGNOSTICS_REL = Path("product/config/battery_guardian/guardian_diagnostics.json")

# Coverage matrix rows: README "Fault Classes to Cover" plus the configured
# combined class. Discovered injected classes are appended dynamically.
CANONICAL_CLASSES = [
    ("Transport", ["transport.delay", "transport.duplicate", "transport.drop", "transport.reorder"]),
    ("Signal", ["signal.stuck", "signal.spike", "signal.drift", "signal.out_of_range", "signal.combination"]),
    ("Source", ["source.dropout", "source.replay_interruption"]),
    ("Diagnostics", ["diagnostics.dfm_write_delay", "diagnostics.opensovd_partial_visibility"]),
]

DIAGNOSTIC_CHAIN = (
    "replay (ASC) -> KUKSA CAN Provider -> KUKSA Data Broker -> VSS uProtocol "
    "Publisher -> Battery Thermal Guardian -> DFM -> OpenSOVD -> Evidence Collector"
)

GAPS = [
    "**Mitigation:** not instrumented (v1 is event-only, ADR-007) - no mitigation "
    "event is recorded; this chain link is intentionally absent.",
    "**OpenSOVD fault status:** only activation times are captured; the "
    "`testFailed`/`confirmedDtc`/`warningIndicator` triple is not observable.",
    "**Unmapped Guardian warnings:** only DFM-mapped `class/level` pairs reach the "
    "collector stream (ADR-016 section 11); utilization warnings may be invisible.",
]


# --------------------------------------------------------------------------- #
# Loading helpers
# --------------------------------------------------------------------------- #

def load_json(path: Path) -> dict:
    with path.open(encoding="utf-8") as stream:
        return json.load(stream)


def load_yaml(path: Path):
    if yaml is None:
        return None
    if not path.is_file():
        return None
    with path.open(encoding="utf-8") as stream:
        return yaml.safe_load(stream)


def find_repo_root(start: Path) -> Path | None:
    for candidate in [start, *start.parents]:
        if (candidate / DIAGNOSTICS_REL).is_file():
            return candidate
    return None


def load_catalog(path: Path) -> dict[str, dict]:
    """Fault catalog from ``guardian_diagnostics.json`` keyed by fault id."""
    catalog: dict[str, dict] = {}
    if not path or not path.is_file():
        return catalog
    for fault in load_json(path).get("faults", []):
        fault_id = fault.get("id")
        if isinstance(fault_id, dict):
            fault_id = fault_id.get("Text") or next(iter(fault_id.values()), None)
        if not fault_id:
            continue
        catalog[fault_id] = {
            "name": fault.get("name", fault_id),
            "category": fault.get("category", "?"),
            "severity": fault.get("severity", "?"),
            "summary": fault.get("summary", ""),
        }
    return catalog


def default_catalog_path() -> Path | None:
    root = find_repo_root(Path(__file__).resolve())
    return (root / DIAGNOSTICS_REL) if root else None


def display_path(path: Path) -> str:
    """Repo-relative display path; independent of the process working directory."""
    root = find_repo_root(Path(__file__).resolve())
    try:
        return os.path.relpath(path.resolve(), root) if root else str(path)
    except ValueError:
        return str(path)


# --------------------------------------------------------------------------- #
# Identity / path derivation (ADR-018: from paths, not JSON fields)
# --------------------------------------------------------------------------- #

def derive_ids(run_dir: Path) -> tuple[str, str]:
    name = run_dir.name
    if "--" in name:
        return tuple(name.split("--", 1))  # type: ignore[return-value]
    return name, ""


def experiment_dir_for(run_dir: Path, campaign: str, scenario: str) -> Path:
    return run_dir.parent / "experiments" / campaign / scenario


# --------------------------------------------------------------------------- #
# Formatting helpers
# --------------------------------------------------------------------------- #

def cell(value) -> str:
    if value is None:
        return "-"
    if isinstance(value, bool):
        return "yes" if value else "no"
    if isinstance(value, float) and value.is_integer():
        return str(int(value))
    return str(value)


def escaped(value) -> str:
    return cell(value).replace("|", "\\|")


def values_inline(values) -> str:
    if not values:
        return "-"
    return ", ".join(cell(v) for v in values)


def table(header: list[str], rows: list[list]) -> list[str]:
    lines = ["| " + " | ".join(header) + " |",
             "|" + "|".join("---" for _ in header) + "|"]
    for row in rows:
        lines.append("| " + " | ".join(escaped(c) for c in row) + " |")
    lines.append("")
    return lines


def link(target: Path, text: str, base: Path) -> str:
    try:
        rel = os.path.relpath(target, start=base)
    except ValueError:
        rel = str(target)
    return f"[{text}]({rel})"


# --------------------------------------------------------------------------- #
# Per-run report
# --------------------------------------------------------------------------- #

def _verdict_badge(verdict: str) -> str:
    return f"**{verdict or 'UNKNOWN'}**"


def render_run_report(run_dir: Path, report: dict, catalog: dict[str, dict],
                      generated_at: str) -> str:
    campaign, scenario = derive_ids(run_dir)
    exp_dir = experiment_dir_for(run_dir, campaign, scenario)
    verdict = report.get("verdict", "UNKNOWN")
    oracle = report.get("oracle") or {}
    injections = report.get("injections") or []
    transitions = report.get("transitions") or []
    fault_events = report.get("fault_events") or []
    sovd = report.get("sovd")
    notes = report.get("notes") or []

    out: list[str] = []
    out.append(f"# Evidence Report - {campaign}--{scenario}")
    out.append("")
    out.append(f"- **Campaign:** {campaign}")
    out.append(f"- **Scenario:** {scenario}")
    out.append(f"- **Run ID:** {campaign}--{scenario}")
    out.append(f"- **Generated:** {generated_at}")
    out.append(f"- **Verdict:** {_verdict_badge(verdict)}")
    out.append("")

    # 2. Provenance and reproduction.
    out.append("## Provenance and reproduction")
    out.append("")
    for label, target in [
        ("Case (ASC)", exp_dir / "case.asc"),
        ("Ground truth", exp_dir / "case.ground_truth.yaml"),
        ("Oracle", exp_dir / "case.oracle.yaml"),
        ("Experiment", exp_dir / "experiment.yaml"),
    ]:
        if target.exists():
            out.append(f"- {label}: {link(target, target.name, run_dir)}")
        else:
            out.append(f"- {label}: `{target.name}` (not found)")
    logs = sorted(p.name for p in run_dir.glob("*.log")) + \
        sorted(p.name for p in run_dir.glob("collector.out"))
    out.append(f"- Logs: {', '.join('`' + name + '`' for name in logs) or '-'}")
    out.append(f"- Case path (as recorded): `{report.get('case', '-')}`")
    out.append("")

    # 3. Diagnostic chain.
    out.append("## Diagnostic chain")
    out.append("")
    out.append("```text")
    out.append(DIAGNOSTIC_CHAIN)
    out.append("```")
    out.append("")

    # 4. Fault catalog (only faults that occurred).
    occurred = sorted({t.get("fault_id") for t in transitions if t.get("fault_id")} |
                      {e.get("fault_id") for e in fault_events if e.get("fault_id")})
    if occurred:
        out.append("## Fault catalog")
        out.append("")
        rows = []
        for fault_id in occurred:
            entry = catalog.get(fault_id, {})
            rows.append([fault_id, entry.get("category", "?"),
                         entry.get("severity", "?"), entry.get("summary", "")])
        out.extend(table(["Fault code", "Category", "Severity", "Description"], rows))
    else:
        out.append("## Fault catalog")
        out.append("")
        out.append("_No faults occurred; see the injection and transition sections._")
        out.append("")

    # 5. Injection summary.
    out.append("## Injections")
    out.append("")
    if not injections:
        out.append("_No injections in this case (baseline)._")
        out.append("")
    for inj in injections:
        out.append(f"### {inj.get('injection_id', '?')} - {inj.get('injected_class', '?')}")
        out.append("")
        out.append(f"- Verdict: {_verdict_badge(inj.get('verdict', '?'))} "
                   f"({inj.get('matched', 0)}/{inj.get('expected', 0)} matched, "
                   f"{inj.get('missing', 0)} missing, {inj.get('unexpected', 0)} unexpected, "
                   f"{inj.get('sovd_missing', 0)} not in OpenSOVD)")
        out.append(f"- Source window: {cell(inj.get('source_started_at_ms'))}.."
                   f"{cell(inj.get('source_finished_at_ms'))} ms "
                   f"(slot {cell((inj.get('slot_ms') or [None, None])[0])}.."
                   f"{cell((inj.get('slot_ms') or [None, None])[1])})")
        mutations = inj.get("mutations") or []
        if not mutations:
            out.append("- Mutations: _(none - transport/source fault)_")
        for mutation in mutations:
            params = mutation.get("requested_parameters") or {}
            param_text = ", ".join(f"{k}={cell(v)}" for k, v in params.items()) or "-"
            out.append(f"- `{mutation.get('signal', '?')}` / `{mutation.get('operator', '?')}`: "
                       f"requested {param_text}; executed [{values_inline(mutation.get('executed_values'))}]")
        out.append("")

    # 6. Expected vs observed.
    out.append("## Expected vs. observed")
    out.append("")
    if oracle:
        out.append(f"Oracle: {oracle.get('expected', 0)} expected, "
                   f"{oracle.get('matched', 0)} matched, {oracle.get('missing', 0)} missing, "
                   f"{oracle.get('not_reached', 0)} not reached, "
                   f"{oracle.get('unexpected', 0)} unexpected, "
                   f"{oracle.get('not_applicable', 0)} not applicable"
                   + (" (allowed)" if oracle.get("allow_unspecified") else ""))
        out.append("")
    if transitions:
        rows = [[t.get("status"), t.get("fault_id"), t.get("detection_class"),
                 t.get("level"), t.get("stage"), cell(t.get("expected_at_ms")),
                 cell(t.get("observed_at_ms")), t.get("signal"),
                 cell(t.get("sovd_visible_at_ms"))] for t in transitions]
        out.extend(table(["Status", "Fault", "Class", "Level", "Stage",
                          "Expected ms", "Observed ms", "Signal", "OpenSOVD ms"], rows))
    else:
        out.append("_No transitions observed._")
        out.append("")

    # 7. Detection evidence.
    out.append("## Detection evidence")
    out.append("")
    if fault_events:
        rows = []
        for event in fault_events:
            ev = event.get("evidence") or {}
            rows.append([cell(event.get("t_ms")), event.get("fault_id"),
                         event.get("detection_class"), event.get("level"),
                         event.get("stage"), cell(ev.get("signal")),
                         cell(ev.get("observed")), cell(ev.get("limit")),
                         cell(ev.get("residual")), cell(ev.get("utilization")),
                         cell(ev.get("interval_ms"))])
        out.extend(table(["t_ms", "Fault", "Class", "Level", "Stage", "Signal",
                          "Observed", "Limit", "Residual", "Utilization", "Interval ms"], rows))
    else:
        out.append("_No fault events observed._")
        out.append("")

    # 8. DFM / OpenSOVD correlation.
    out.append("## DFM / OpenSOVD correlation")
    out.append("")
    if sovd:
        activations = sovd.get("activations") or []
        out.append(f"- URL: `{sovd.get('url', '-')}`")
        out.append(f"- Polls: {sovd.get('polls', 0)} ({sovd.get('errors', 0)} failed)")
        out.append(f"- Activations: {len(activations)}; "
                   f"not visible: {len(sovd.get('not_visible') or [])}; "
                   f"unexplained: {len(sovd.get('unexplained') or [])}")
        out.append("")
        if activations:
            out.extend(table(["t_ms", "Code"],
                             [[cell(a.get("t_ms")), a.get("code")] for a in activations]))
        if sovd.get("not_visible"):
            out.append("Not visible in time: " + ", ".join(
                f"`{a.get('fault_id', a.get('code'))}`" for a in sovd["not_visible"]))
            out.append("")
        if sovd.get("unexplained"):
            out.append("Unexplained: " + ", ".join(
                f"`{a.get('code')}`@{cell(a.get('t_ms'))}ms" for a in sovd["unexplained"]))
            out.append("")
    else:
        out.append("_OpenSOVD was not queried for this case._")
        out.append("")

    # 9. Timing.
    out.append("## Timing")
    out.append("")
    out.append(f"- Replay end: {cell(report.get('replay_end_ms'))} ms; "
               f"battery stream end: {cell(report.get('battery_end_ms'))} ms")
    out.append("")
    starts = {inj.get("injection_id"): inj.get("source_started_at_ms") for inj in injections}
    timing_rows = []
    for t in transitions:
        if t.get("observed_at_ms") is None:
            continue
        start = starts.get(t.get("injection_id"))
        detection = None if start is None else t["observed_at_ms"] - start
        sovd_ms = t.get("sovd_visible_at_ms")
        visible = None if sovd_ms is None else sovd_ms - t["observed_at_ms"]
        timing_rows.append([t.get("injection_id"), t.get("fault_id"),
                            cell(start), cell(t.get("observed_at_ms")),
                            cell(detection), cell(visible)])
    if timing_rows:
        out.extend(table(["Injection", "Fault", "Injected ms", "Detected ms",
                          "Detection latency ms", "OpenSOVD latency ms"], timing_rows))
    else:
        out.append("_No observed transitions to time._")
        out.append("")

    # 10. Verdict rationale.
    out.append("## Verdict rationale")
    out.append("")
    if verdict == "PASS":
        out.append(f"All {oracle.get('expected', 0)} expected Guardian change(s) matched and "
                   "no unexpected change was observed.")
    elif verdict == "FAIL":
        out.append(f"Oracle reported {oracle.get('missing', 0)} missing and "
                   f"{oracle.get('unexpected', 0)} unexpected Guardian change(s).")
    elif verdict == "INCONCLUSIVE":
        out.append("The verdict is INCONCLUSIVE: expectation set empty, evidence missing, "
                   "or an infrastructure failure (see notes).")
    else:
        out.append("No verdict recorded.")
    out.append("")

    # 11. Observer artifacts.
    out.append("## Observer artifacts")
    out.append("")
    html = run_dir / "observer.html"
    png = run_dir / "observer.png"
    if html.exists():
        out.append(f"- Final live-observer view: {link(html, 'observer.html', run_dir)}")
    if png.exists():
        out.append(f"- Rendering: {link(png, 'observer.png', run_dir)}")
        out.append("")
        out.append("![Final observer state](observer.png)")
    if not html.exists() and not png.exists():
        out.append("_No observer artifacts captured for this run._")
    out.append("")
    out.append("Observer artifacts are illustrative only and never a source of "
               "evaluation facts.")
    out.append("")

    # 12. Gaps and limitations.
    out.append("## Gaps and limitations")
    out.append("")
    for gap in GAPS:
        out.append(f"- {gap}")
    out.append("")

    # 13. Notes.
    out.append("## Notes")
    out.append("")
    if notes:
        for note in notes:
            out.append(f"- {note}")
    else:
        out.append("- _(none)_")
    out.append("")
    return "\n".join(out)


# --------------------------------------------------------------------------- #
# Campaign report
# --------------------------------------------------------------------------- #

def _collect_runs(campaign_dir: Path) -> list[dict]:
    runs: list[dict] = []
    exp_root = campaign_dir / "experiments"
    if exp_root.is_dir():
        for campaign in sorted(exp_root.iterdir()):
            if not campaign.is_dir():
                continue
            for scenario in sorted(campaign.iterdir()):
                if not scenario.is_dir():
                    continue
                runs.append(_run_entry(campaign_dir, campaign.name, scenario.name))
    # Defensive: run dirs without an experiments/ bundle.
    known = {r["id"] for r in runs}
    for child in sorted(campaign_dir.iterdir()):
        if child.is_dir() and "--" in child.name and child.name not in known:
            campaign, scenario = derive_ids(child)
            runs.append(_run_entry(campaign_dir, campaign, scenario))
    return runs


def _run_entry(campaign_dir: Path, campaign: str, scenario: str) -> dict:
    run_id = f"{campaign}--{scenario}"
    run_dir = campaign_dir / run_id
    report_path = run_dir / "report.json"
    report = load_json(report_path) if report_path.is_file() else None
    return {"id": run_id, "campaign": campaign, "scenario": scenario,
            "run_dir": run_dir, "report": report}


def _skip_reason(campaign_dir: Path, run: dict) -> str:
    exp_dir = campaign_dir / "experiments" / run["campaign"] / run["scenario"]
    unsat = exp_dir / "unsatisfiable.yaml"
    if unsat.is_file():
        data = load_yaml(unsat) or _parse_unsatisfiable(unsat)
        detail = data.get("detail") or {}
        reason = detail.get("reason") or {}
        code = reason.get("code", data.get("status", "UNSATISFIABLE"))
        text = reason.get("detail", "") or ""
        text = " ".join(str(text).split())
        return f"{code}: {text}".strip(": ") if text else str(code)
    if (exp_dir / "case.asc").is_file():
        return "generated but not executed"
    return "no replay"


def _parse_unsatisfiable(path: Path) -> dict:
    """Minimal stdlib fallback for unsatisfiable.yaml when PyYAML is absent."""
    text = path.read_text(encoding="utf-8")
    code = re.search(r"\bcode:\s*'?\"?([A-Za-z_][A-Za-z0-9_]*)", text)
    detail = re.search(r"^\s+detail:\s*'?\"?(.*)$", text, re.MULTILINE)
    reason = {"code": code.group(1)} if code else {}
    if detail:
        reason["detail"] = detail.group(1).rstrip("'\"")
    return {"status": "UNSATISFIABLE", "detail": {"reason": reason}}


def _run_verdict(run: dict) -> str:
    if run["report"] is None:
        return "SKIPPED"
    return run["report"].get("verdict", "UNKNOWN")


# Campaign ids that do not equal their injected class (elementary campaigns are
# named by class, e.g. `signal.stuck`); used for skipped experiments only.
CLASS_ALIASES = {"combined_example": "signal.combination"}


def _classes_for(run: dict) -> set[str]:
    """Injected classes of a run, also for skipped experiments without a report."""
    report = run["report"] or {}
    classes = {inj.get("injected_class") for inj in (report.get("injections") or [])
               if inj.get("injected_class")}
    if classes:
        return classes
    campaign = run["campaign"]
    if "." in campaign:
        return {campaign}
    return {CLASS_ALIASES[campaign]} if campaign in CLASS_ALIASES else set()


def _skip_code(campaign_dir: Path, run: dict) -> str:
    return _skip_reason(campaign_dir, run).split(":", 1)[0].strip()


def render_campaign_report(campaign_dir: Path, catalog: dict[str, dict],
                           generated_at: str, compare_dir: Path | None = None) -> str:
    runs = _collect_runs(campaign_dir)
    counts: dict[str, int] = {}
    for run in runs:
        verdict = _run_verdict(run)
        counts[verdict] = counts.get(verdict, 0) + 1

    out: list[str] = []
    out.append(f"# Evidence Report - {campaign_dir.name}")
    out.append("")
    out.append(f"- **Generated:** {generated_at}")
    total = ", ".join(f"{v}: {counts[v]}" for v in
                      ["PASS", "FAIL", "INCONCLUSIVE", "SKIPPED"] if counts.get(v)) or "none"
    out.append(f"- **Runs:** {len(runs)} ({total})")
    out.append("")

    out.append("## Runs")
    out.append("")
    rows = []
    for run in runs:
        report = run["report"] or {}
        oracle = report.get("oracle") or {}
        classes = sorted(_classes_for(run)) or ["(none)"]
        report_link = link(run["run_dir"] / "report.md", "report.md", campaign_dir)
        rows.append([run["id"], ", ".join(classes), _run_verdict(run),
                     oracle.get("expected", "-"), oracle.get("matched", "-"),
                     oracle.get("missing", "-"), oracle.get("unexpected", "-"),
                     report_link if (run["run_dir"] / "report.md").exists() else "-"])
    out.extend(table(["Run", "Injected class(es)", "Verdict", "Expected", "Matched",
                      "Missing", "Unexpected", "Report"], rows))

    # Coverage matrix.
    out.append("## Fault-class coverage")
    out.append("")
    discovered = {cls for run in runs for cls in _classes_for(run)}
    matrix_rows = []
    for layer, classes in CANONICAL_CLASSES:
        for cls in classes:
            matching = [r for r in runs if cls in _classes_for(r)]
            if matching:
                def label(r):
                    verdict = _run_verdict(r)
                    return f"{r['id']}=SKIPPED ({_skip_code(campaign_dir, r)})" \
                        if verdict == "SKIPPED" else f"{r['id']}={verdict}"
                status = ", ".join(label(r) for r in matching)
            else:
                status = "not attempted"
            matrix_rows.append([layer, cls, status])
    others = sorted(discovered - {c for _, cs in CANONICAL_CLASSES for c in cs})
    for cls in others:
        layer = cls.split(".", 1)[0].capitalize()
        matching = [r for r in runs if cls in _classes_for(r)]
        matrix_rows.append([layer, cls, ", ".join(f"{r['id']}={_run_verdict(r)}" for r in matching)])
    out.extend(table(["Layer", "Injected class", "Runs / result"], matrix_rows))

    # Non-PASS section.
    out.append("## Non-PASS runs")
    out.append("")
    non_pass = [r for r in runs if _run_verdict(r) != "PASS"]
    if not non_pass:
        out.append("_Every run passed._")
    for run in non_pass:
        verdict = _run_verdict(run)
        if verdict == "SKIPPED":
            out.append(f"- **{run['id']}** - SKIPPED: {_skip_reason(campaign_dir, run)}")
        else:
            report = run["report"] or {}
            notes = "; ".join(report.get("notes") or []) or "no notes"
            out.append(f"- **{run['id']}** - {verdict}: {notes}")
    out.append("")

    # Consistency across campaigns.
    if compare_dir and compare_dir.is_dir():
        out.append("## Consistency across campaigns")
        out.append("")
        other = {r["id"]: _run_verdict(r) for r in _collect_runs(compare_dir)}
        rows = []
        for run in runs:
            if run["id"] in other:
                rows.append([run["id"], _run_verdict(run), other[run["id"]],
                             "match" if _run_verdict(run) == other[run["id"]] else "DIFFER"])
        out.extend(table(["Run", campaign_dir.name, compare_dir.name, "Consistent"], rows))

    out.append("## Aggregate gaps and limitations")
    out.append("")
    for gap in GAPS:
        out.append(f"- {gap}")
    out.append("")

    out.append("## Reproduction")
    out.append("")
    out.append(f"- Campaign directory: `{display_path(campaign_dir)}`")
    out.append("- Re-run: `tools/run_campaign.sh` (optionally `--campaign ID --scenario ID`).")
    out.append("- Per-run detail: see each run's `report.md`.")
    out.append("")
    return "\n".join(out)


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #

def _now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _resolve_catalog(args) -> dict[str, dict]:
    path = Path(args.diagnostics) if args.diagnostics else default_catalog_path()
    if path and path.is_file():
        return load_catalog(path)
    if args.diagnostics:
        print(f"evidence_reporter: diagnostics not found: {path}", file=sys.stderr)
    return {}


def cmd_run(args) -> int:
    run_dir = Path(args.run_dir)
    report_path = run_dir / "report.json"
    if not report_path.is_file():
        print(f"evidence_reporter: no report.json in {run_dir}", file=sys.stderr)
        return 1
    catalog = _resolve_catalog(args)
    text = render_run_report(run_dir, load_json(report_path), catalog,
                             args.generated_at or _now())
    out = Path(args.out) if args.out else run_dir / "report.md"
    out.write_text(text, encoding="utf-8")
    print(f"evidence_reporter: wrote {out}")
    return 0


def cmd_campaign(args) -> int:
    campaign_dir = Path(args.campaign_dir)
    if not campaign_dir.is_dir():
        print(f"evidence_reporter: no campaign directory {campaign_dir}", file=sys.stderr)
        return 1
    catalog = _resolve_catalog(args)
    compare = Path(args.compare) if args.compare else None

    # Render per-run reports first so the campaign links can resolve.
    for run in _collect_runs(campaign_dir):
        if run["report"] is None:
            continue
        run_text = render_run_report(run["run_dir"], run["report"], catalog,
                                     args.generated_at or _now())
        (run["run_dir"] / "report.md").write_text(run_text, encoding="utf-8")

    text = render_campaign_report(campaign_dir, catalog, args.generated_at or _now(), compare)
    out = Path(args.out) if args.out else campaign_dir / "evidence_report.md"
    out.write_text(text, encoding="utf-8")
    print(f"evidence_reporter: wrote {out}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="evidence_reporter",
                                     description="Render collector JSON into Markdown (ADR-018)")
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--diagnostics", help="path to guardian_diagnostics.json")
    common.add_argument("--generated-at", help="fixed header timestamp (tests)")
    common.add_argument("--out", help="output file (default: next to the input)")
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run", parents=[common], help="render one run directory")
    run.add_argument("run_dir")
    run.set_defaults(func=cmd_run)
    campaign = sub.add_parser("campaign", parents=[common], help="render a campaign directory")
    campaign.add_argument("campaign_dir")
    campaign.add_argument("--compare", help="second campaign directory for consistency")
    campaign.set_defaults(func=cmd_campaign)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
