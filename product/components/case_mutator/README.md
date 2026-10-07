# Battery Case Mutator

The component implements the inverse-model case generation specified in
`product/doc/mutator/case_mutator_model.md`. It mutates canonical battery ASC
frames, forward-verifies candidates with the real `battery-guardian` model,
and emits three artifacts:

- `<injection-id>.asc`
- `<injection-id>.ground_truth.yaml`
- `<injection-id>.oracle.yaml`

Run inside the repository dev container:

```sh
cd product/components/case_mutator
make check
make run REQUEST=examples/out_of_range.yaml
```

Paths in a request are resolved relative to the current working directory.
The request may define mutations/actions inline or select them by
`injection_id` from `fault_injection_model.yaml`.

On an unsatisfiable request the CLI writes
`<injection-id>.unsatisfiable.yaml` and exits with status 2. Invalid input or
configuration exits with status 1.
