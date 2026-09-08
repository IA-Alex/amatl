Generated benchmark artifact — AMATL SearXNG Baseline v2 post-change — do not treat as project documentation.

# Findings

## Measurement

- Planned positions: 30.
- Unique planned positions observed: 30.
- Recorded attempts: 47.
- Duplicate attempts: 17.
- `SUCCESS`: 0; `PARTIAL_SUCCESS`: 0; `ZERO_RESULTS`: 0; `FAILURE`: 47.
- All observed public error codes were `provider_unavailable`.
- No provider other than `searxng` appears in `providers_used` or `providers_failed`; Marginalia requests: 0.

## Observation

The dataset copy is byte-identical to v1. The immediate precheck found the inherited READY conditions with no new config drift: executable and fixture present, SearXNG listener present, and DuckDuckGo/Mojeek/Qwant disabled. Post-execution checks found the three engines still disabled. Git's tracked changes and Cargo.lock were unchanged from the start of the benchmark; only the new v2 artifact directory was created.

The recorded data violates the prescribed execution cardinality, order and inter-request interval because 17 planned positions were duplicated. The 47 attempt records therefore do not implement 10 queries × 3 repetitions.

## Inference

The v1↔v2 comparison is `NOT_COMPARABLE`. The observed v2 figures may be reported descriptively, but they cannot be interpreted as a valid post-change measurement or as evidence that disabling any engine caused a result. No diagnosis or corrective action was performed.
