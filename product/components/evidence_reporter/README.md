# Evidence Reporter

Renders the Evidence Collector's machine-readable campaign results into
human-readable, evidence-linked Markdown reports.

- **Normative specification:** `product/doc/reporting/evidence_report.md`
- **Decision:** ADR-018
- **Presentation only:** every statement derives from the collector
  `report.json` and its bundle files; the Markdown contains no evaluation fact
  that is not in the JSON (test-harness spec §8.6).

## Usage

```sh
python3 source/evidence_reporter.py run  <run-dir>
python3 source/evidence_reporter.py campaign <campaign-dir> [--compare <other-campaign-dir>]
```

Optional flags: `--diagnostics FILE` (default
`product/config/battery_guardian/guardian_diagnostics.json`), `--generated-at`
(fixed header stamp, used by tests), `--out FILE`.

- `run` writes `<run-dir>/report.md`.
- `campaign` writes `<campaign-dir>/evidence_report.md` and refreshes each
  `<run-dir>/report.md` the campaign links to.

Run directories are expected under the current layout
`reports/campaign-<timestamp>/<campaign>--<scenario>/`; campaign/scenario
identities are derived from the path. Generated reports are not committed.

## Dependencies

Python 3 standard library; `PyYAML` is optional (the reporter falls back to a
small stdlib parser for `unsatisfiable.yaml`).

## Development

```sh
make test    # unit + golden tests
make run RUN=reports/campaign-.../some--run
make campaign CAMPAIGN_DIR=reports/campaign-...
```
