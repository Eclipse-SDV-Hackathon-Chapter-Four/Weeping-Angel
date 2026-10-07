# Battery Campaign Harness

The harness is a small Python CLI for the deterministic generation phase in
`product/doc/testing/battery_campaign_test_harness.md`. It runs directly in the
DevContainer and delegates mutation and forward evaluation to the Rust Case
Mutator binaries.

```sh
make -C product/components/battery_campaign_harness validate
make -C product/components/battery_campaign_harness plan
make -C product/components/battery_campaign_harness generate
```

Use `CAMPAIGN` and `SCENARIO` for a small selection while developing:

```sh
make -C product/components/battery_campaign_harness generate \
  CAMPAIGN=signal.spike SCENARIO=warm_nominal
```

Generation never overwrites an existing experiment directory. A profile that
cannot meet its model goal is retained as `unsatisfiable.yaml` instead of being
silently weakened. Live replay is intentionally separate: the readiness and
three-plane Evidence Collector contracts listed as open in the specification
do not yet exist in machine-readable form.
