# Wiring proof

`main()` registers `--campaign` and dispatches it to `run_campaign()` only after
requiring `--fixture` and `--amatl-binary`.

`run_campaign()` performs, in order:

1. `ensure_output_absent(output_dir)`
2. `load_dataset()`
3. `build_plan()`
4. `validated_plan()` (which calls `validate_plan()`)
5. `AmatlProcessExecutor(...)`
6. `DurableJSONLWriter(output_dir / "runs.jsonl")`
7. `execute_plan(validated, amatl, writer)`

There is no loop over positions in this function and its `execute_plan` call has
no `max_positions` argument. The existing global 30-position guard remains in
`execute_plan()`.
