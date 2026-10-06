"""Robot Framework library for the Battery Thermal Guardian safety-evidence suite.

Drives the end-to-end chain:

    fault_injector --(uProtocol/Zenoh)--> Guardian --(iceoryx2 IPC)--> DFM
        --> OpenSOVD gateway --(SOVD HTTP)--> this library

Responsibilities:
  * shape the battery-temperature signal per scenario (runs the ``fault_injector``
    binary),
  * inject transport-level faults on the Zenoh link via the Toxiproxy API,
  * read the resulting faults back through the OpenSOVD *apps* fault interface,
  * accumulate per-scenario evidence and render a Markdown report.
"""

import json
import os
import subprocess
import time
from datetime import datetime, timezone

import requests

from robot.api import logger


class SovdFaultLibrary:
    ROBOT_LIBRARY_SCOPE = "SUITE"

    # ------------------------------------------------------------- forensics
    # The "witness" that catches each fault: the Guardian detector and the exact
    # parameter it trips on (kept in sync with services/src/bin/guardian.rs).
    _DETECTORS = {
        "BatteryOverTempWarning": "Guardian threshold monitor — temp_max >= 45 C (WARN band)",
        "BatteryOverTempCritical": "Guardian threshold monitor — temp_max >= 55 C (CRIT band)",
        "BatteryTempImplausible": (
            "Guardian plausibility check — value outside [-40, 125] C "
            "or a jump > 20 C between consecutive samples"
        ),
        "BatteryTempSignalStuck": "Guardian stuck monitor — >= 5 consecutive identical samples",
        "BatteryTempSignalStale": (
            "Guardian freshness watchdog — no fresh sample for > 2000 ms "
            "(polled every 500 ms)"
        ),
    }

    # Ground-truth attribution per scenario: who did it and how it entered the
    # diagnostic chain. Keyed by the scenario name passed to Record Scenario.
    _CASE_FILES = {
        "Baseline": {
            "layer": "— (healthy system)",
            "culprit": "No culprit — nominal operation",
            "means": "Nominal 30..44 C ramp with fresh samples throughout",
        },
        "Overtemperature": {
            "layer": "Physical — battery pack (Hardware)",
            "culprit": "Genuine cell over-temperature — the pack really crossed 45 C then 55 C",
            "means": "fault_injector shapes a true rising temperature ramp",
        },
        "Stuck signal": {
            "layer": "Sensor / signal source (Communication)",
            "culprit": "Frozen sensor reading — the source is wedged on one value",
            "means": "fault_injector emits an unchanging 40 C value",
        },
        "Implausible spike": {
            "layer": "Sensor / plausibility (Configuration)",
            "culprit": "Corrupt reading — 300 C is physically impossible",
            "means": "fault_injector emits a single out-of-range spike (300 C)",
        },
        "Source dropout": {
            "layer": "Publisher — application source (Communication)",
            "culprit": "Silent publisher — the temperature source stopped emitting",
            "means": "fault_injector sends a few samples then exits; the wire goes quiet",
        },
        "Transport delay": {
            "layer": "Transport — Zenoh link (Communication)",
            "culprit": "Broken link — a healthy publisher's samples are dropped in transit",
            "means": "Toxiproxy blackholes the Zenoh downstream while the injector keeps publishing",
        },
    }

    def __init__(
        self,
        gateway="http://127.0.0.1:7690",
        app_id="battery_guardian",
        injector="../target/debug/fault_injector",
        catalog="../diagnostics/catalog/battery_guardian.json",
        zenoh_direct="tcp/127.0.0.1:7447",
        zenoh_proxy="tcp/127.0.0.1:7448",
        toxiproxy_api="http://127.0.0.1:8474",
        proxy_name="zenoh",
        report="../reports/evidence_report.md",
    ):
        self.gateway = gateway.rstrip("/")
        self.app_id = app_id
        self.injector = os.path.abspath(injector)
        self.catalog = os.path.abspath(catalog)
        self.zenoh_direct = zenoh_direct
        self.zenoh_proxy = zenoh_proxy
        self.toxiproxy_api = toxiproxy_api.rstrip("/")
        self.proxy_name = proxy_name
        self.report = os.path.abspath(report)
        self._evidence = []

    # ------------------------------------------------------------------ SOVD
    @property
    def _faults_url(self):
        return f"{self.gateway}/sovd/v1/apps/{self.app_id}/faults"

    def get_fault_snapshot(self):
        """Return {code: status_dict} for every catalog fault (raised or not)."""
        resp = requests.get(self._faults_url, timeout=5)
        resp.raise_for_status()
        items = resp.json().get("items", [])
        return {f.get("code"): f.get("status", {}) for f in items}

    def get_active_faults(self):
        """Return the sorted list of fault codes whose ``testFailed`` bit is set."""
        snap = self.get_fault_snapshot()
        return sorted(c for c, s in snap.items() if s.get("testFailed"))

    def opensovd_lists_all_catalog_faults(self, expected_count=5):
        """Assert the OpenSOVD app interface exposes the full fault catalog."""
        codes = sorted(self.get_fault_snapshot().keys())
        expected = int(expected_count)
        if len(codes) != expected:
            raise AssertionError(
                f"expected {expected} catalog faults on OpenSOVD, got {len(codes)}: {codes}"
            )
        logger.info(f"OpenSOVD exposes {len(codes)} faults: {codes}")
        return codes

    def reset_faults(self):
        """Clear sticky DTC state via SOVD DELETE (idempotent: 204 then 503)."""
        resp = requests.delete(self._faults_url, timeout=5)
        # First clear returns 204; a subsequent clear of an already-clean store
        # returns 503 KeyNotFound. Both mean "nothing left raised".
        if resp.status_code not in (200, 204, 503):
            resp.raise_for_status()
        logger.info(f"reset faults -> HTTP {resp.status_code}")

    # -------------------------------------------------------------- injector
    def _run_injector(self, scenario, connect, *, background=False, extra_env=None):
        env = dict(os.environ)
        env["SCENARIO"] = scenario
        env["ZENOH_CONNECT"] = connect
        env["RUST_LOG"] = env.get("RUST_LOG", "warn")
        if extra_env:
            env.update(extra_env)
        logger.info(f"inject scenario='{scenario}' connect={connect}")
        if background:
            return subprocess.Popen(
                [self.injector],
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        subprocess.run([self.injector], env=env, check=True, timeout=120)
        return None

    def inject_scenario(self, scenario):
        """Run a signal-shaping scenario to completion over the direct link."""
        self._run_injector(scenario, self.zenoh_direct)

    def establish_baseline(self):
        """Feed a nominal series so the watchdog sees fresh data (no faults)."""
        self._run_injector("nominal", self.zenoh_direct)

    # ----------------------------------------------------------- transport fx
    def _toxic_url(self):
        return f"{self.toxiproxy_api}/proxies/{self.proxy_name}/toxics"

    def add_zenoh_blackhole(self):
        """Add a Toxiproxy timeout toxic that halts the Zenoh stream (dropout)."""
        self.remove_zenoh_blackhole()
        body = {
            "name": "zenoh_blackhole",
            "type": "timeout",
            "stream": "downstream",
            "toxicity": 1.0,
            "attributes": {"timeout": 0},
        }
        resp = requests.post(self._toxic_url(), json=body, timeout=5)
        if resp.status_code not in (200, 201):
            raise AssertionError(f"add toxic failed: HTTP {resp.status_code} {resp.text}")
        logger.info("added Zenoh blackhole toxic (timeout=0)")

    def remove_zenoh_blackhole(self):
        """Remove the transport toxic if present."""
        url = f"{self._toxic_url()}/zenoh_blackhole"
        try:
            requests.delete(url, timeout=5)
        except requests.RequestException:
            pass

    def inject_transport_delay(self):
        """Simulate a delayed/dropped CAN-derived signal at the transport layer.

        Starts a long, otherwise-safe stream over the Toxiproxy'd Zenoh link,
        confirms it flows, then blackholes the link so no fresh samples reach
        the Guardian -- the freshness watchdog must then raise the stale fault.
        """
        proc = self._run_injector("stream", self.zenoh_proxy, background=True)
        try:
            time.sleep(2.0)  # let the stream flow and clear any prior staleness
            self.add_zenoh_blackhole()
            time.sleep(3.5)  # exceed the 2 s freshness deadline
        finally:
            self.remove_zenoh_blackhole()
            if proc is not None:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()

    # -------------------------------------------------------------- polling
    def wait_for_active_faults(self, *expected, timeout=15, poll=0.5):
        """Wait until every fault in ``expected`` is active; return active list."""
        if len(expected) == 1 and not isinstance(expected[0], str):
            expected = expected[0]
        expected = set(expected)
        deadline = time.time() + float(timeout)
        active = []
        while time.time() < deadline:
            active = self.get_active_faults()
            if expected.issubset(set(active)):
                logger.info(f"observed expected faults active: {sorted(expected)}")
                return active
            time.sleep(float(poll))
        raise AssertionError(
            f"timeout waiting for {sorted(expected)}; active now: {active}"
        )

    def wait_for_clear(self, timeout=10, poll=0.5):
        """Wait until no fault is active."""
        deadline = time.time() + float(timeout)
        active = self.get_active_faults()
        while time.time() < deadline:
            active = self.get_active_faults()
            if not active:
                return []
            time.sleep(float(poll))
        raise AssertionError(f"faults still active after {timeout}s: {active}")

    # -------------------------------------------------------------- evidence
    def record_scenario(self, name, hazard, expected, verdict="PASS"):
        """Capture a SOVD snapshot as evidence for the report."""
        if isinstance(expected, str):
            expected = [e for e in expected.replace(",", " ").split() if e]
        snapshot = self.get_fault_snapshot()
        active = sorted(c for c, s in snapshot.items() if s.get("testFailed"))
        self._evidence.append(
            {
                "name": name,
                "hazard": hazard,
                "expected": list(expected),
                "observed": active,
                "verdict": verdict,
                "snapshot": snapshot,
                "time": datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ"),
            }
        )
        logger.info(f"recorded scenario '{name}': observed={active}")

    def _load_catalog(self):
        with open(self.catalog) as fh:
            return json.load(fh)

    def _detective_story_lines(self):
        """Render the root-cause 'whodunit' narrative from recorded evidence."""
        lines = []
        lines.append("## The detective story — from symptom to culprit")
        lines.append("")
        lines.append(
            "A DTC is only a *symptom*. This section walks each clue back along the "
            "diagnostic chain to the **culprit**: the layer where the fault entered, "
            "the means by which it was introduced, and the **witness** (the Guardian "
            "detector) that caught it. The OpenSOVD read-back is the confession on record."
        )
        lines.append("")
        lines.append("| Case | Symptom (DTC on OpenSOVD) | Origin layer | Culprit (root cause) | How it entered | Caught by (witness) |")
        lines.append("|---|---|---|---|---|---|")
        for e in self._evidence:
            case = self._CASE_FILES.get(e["name"], {})
            symptom = ", ".join(f"`{c}`" for c in e["observed"]) or "_none (cleared)_"
            witnesses = [self._DETECTORS.get(c) for c in e["observed"] if self._DETECTORS.get(c)]
            witness = "; ".join(dict.fromkeys(witnesses)) or "_no detector tripped_"
            lines.append(
                f"| {e['name']} | {symptom} | {case.get('layer','—')} | "
                f"{case.get('culprit','—')} | {case.get('means','—')} | {witness} |"
            )
        lines.append("")

        # Highlight the ambiguous DTC shared by two different culprits.
        stale = "BatteryTempSignalStale"
        stale_cases = [e["name"] for e in self._evidence if stale in e["observed"]]
        if len(stale_cases) > 1:
            lines.append("### The twist — one DTC, two culprits")
            lines.append("")
            lines.append(
                f"`{stale}` is raised in **{len(stale_cases)}** different cases "
                f"({', '.join('*' + c + '*' for c in stale_cases)}) with an *identical* "
                "fault code. The DTC alone cannot tell you **who did it** — the Guardian's "
                "freshness watchdog only knows that no fresh sample arrived in time."
            )
            lines.append("")
            lines.append("The evidence that distinguishes the culprits:")
            lines.append("")
            lines.append(
                "- **Source dropout** — the publisher process has *exited*; nothing is on the "
                "wire at all. Root cause lives at the **application/source** layer."
            )
            lines.append(
                "- **Transport delay** — the publisher is *still alive and emitting*, but the "
                "Zenoh link is blackholed by Toxiproxy, so samples never arrive. Root cause "
                "lives at the **transport** layer."
            )
            lines.append("")
            lines.append(
                "> Detective's note: to close the case you must correlate the DTC with "
                "liveness of the publisher and traffic on the Zenoh link — the read-back "
                "confirms *that* the signal went stale, not *why*."
            )
            lines.append("")
        return lines

    def write_evidence_report(self):
        """Render the accumulated evidence as a Markdown safety report."""
        cat = self._load_catalog()
        os.makedirs(os.path.dirname(self.report), exist_ok=True)
        lines = []
        now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ")
        lines.append("# Battery Thermal Guardian — Safety Evidence Report")
        lines.append("")
        lines.append(f"_Generated: {now}_")
        lines.append("")
        lines.append(
            "This report is produced automatically by the Robot Framework suite. "
            "Each scenario injects a battery-temperature or transport fault, the "
            "Guardian evaluates it and reports to the Diagnostic Fault Manager (DFM), "
            "and the faults are read back through the **OpenSOVD** fault interface."
        )
        lines.append("")
        lines.append("## Diagnostic chain")
        lines.append("")
        lines.append("```")
        lines.append("fault_injector --(uProtocol/Zenoh)--> Battery Thermal Guardian")
        lines.append("      --(iceoryx2 IPC)--> DFM --> OpenSOVD gateway")
        lines.append(f"      --> SOVD HTTP: GET {self.gateway}/sovd/v1/apps/{self.app_id}/faults")
        lines.append("```")
        lines.append("")

        # Fault catalog ----------------------------------------------------
        lines.append("## Fault catalog")
        lines.append("")
        lines.append("| Fault code | Category | Severity | Description |")
        lines.append("|---|---|---|---|")
        for f in cat.get("faults", []):
            code = f.get("id", {}).get("Text", "")
            lines.append(
                f"| `{code}` | {f.get('category','')} | {f.get('severity','')} | {f.get('summary','')} |"
            )
        lines.append("")

        # Scenario results -------------------------------------------------
        total = len(self._evidence)
        passed = sum(1 for e in self._evidence if e["verdict"] == "PASS")
        lines.append("## Scenario results")
        lines.append("")
        lines.append(f"**{passed}/{total} scenarios passed.**")
        lines.append("")
        lines.append("| Scenario | Injected condition | Expected fault(s) | OpenSOVD observed | Verdict |")
        lines.append("|---|---|---|---|---|")
        for e in self._evidence:
            exp = ", ".join(f"`{c}`" for c in e["expected"]) or "_none_"
            obs = ", ".join(f"`{c}`" for c in e["observed"]) or "_none (clear)_"
            mark = "PASS" if e["verdict"] == "PASS" else "FAIL"
            lines.append(
                f"| {e['name']} | {e['hazard']} | {exp} | {obs} | {mark} |"
            )
        lines.append("")

        # Detective story: root-cause attribution ---------------------------
        lines.extend(self._detective_story_lines())

        # Appendix: raw SOVD status ---------------------------------------
        lines.append("## Appendix — OpenSOVD fault status per scenario")
        lines.append("")
        for e in self._evidence:
            lines.append(f"### {e['name']}")
            lines.append(f"_captured {e['time']}_")
            lines.append("")
            lines.append("| Fault code | testFailed | confirmedDtc | warningIndicator |")
            lines.append("|---|---|---|---|")
            for code, st in sorted(e["snapshot"].items()):
                lines.append(
                    f"| `{code}` | {st.get('testFailed')} | "
                    f"{st.get('confirmedDtc')} | {st.get('warningIndicatorRequested')} |"
                )
            lines.append("")

        with open(self.report, "w") as fh:
            fh.write("\n".join(lines) + "\n")
        logger.info(f"wrote evidence report -> {self.report}")
        return self.report
