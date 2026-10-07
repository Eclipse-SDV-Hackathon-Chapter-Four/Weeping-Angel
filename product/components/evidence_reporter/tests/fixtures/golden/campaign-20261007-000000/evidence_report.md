# Evidence Report - campaign-20261007-000000

- **Generated:** 1970-01-01T00:00:00Z
- **Runs:** 2 (PASS: 1, SKIPPED: 1)

## Runs

| Run | Injected class(es) | Verdict | Expected | Matched | Missing | Unexpected | Report |
|---|---|---|---|---|---|---|---|
| signal.spike--warm_nominal | signal.spike | PASS | 2 | 2 | 0 | 0 | [report.md](signal.spike--warm_nominal/report.md) |
| signal.stuck--cold_nominal | signal.stuck | SKIPPED | - | - | - | - | - |

## Fault-class coverage

| Layer | Injected class | Runs / result |
|---|---|---|
| Transport | transport.delay | not attempted |
| Transport | transport.duplicate | not attempted |
| Transport | transport.drop | not attempted |
| Transport | transport.reorder | not attempted |
| Signal | signal.stuck | signal.stuck--cold_nominal=SKIPPED (ENCODING_LIMIT) |
| Signal | signal.spike | signal.spike--warm_nominal=PASS |
| Signal | signal.drift | not attempted |
| Signal | signal.out_of_range | not attempted |
| Signal | signal.combination | not attempted |
| Source | source.dropout | not attempted |
| Source | source.replay_interruption | not attempted |
| Diagnostics | diagnostics.dfm_write_delay | not attempted |
| Diagnostics | diagnostics.opensovd_partial_visibility | not attempted |

## Non-PASS runs

- **signal.stuck--cold_nominal** - SKIPPED: ENCODING_LIMIT: no DBC-representable candidate trajectory

## Aggregate gaps and limitations

- **Mitigation:** not instrumented (v1 is event-only, ADR-007) - no mitigation event is recorded; this chain link is intentionally absent.
- **OpenSOVD fault status:** only activation times are captured; the `testFailed`/`confirmedDtc`/`warningIndicator` triple is not observable.
- **Unmapped Guardian warnings:** only DFM-mapped `class/level` pairs reach the collector stream (ADR-016 section 11); utilization warnings may be invisible.

## Reproduction

- Campaign directory: `product/components/evidence_reporter/tests/fixtures/golden/campaign-20261007-000000`
- Re-run: `tools/run_campaign.sh` (optionally `--campaign ID --scenario ID`).
- Per-run detail: see each run's `report.md`.
