# Offline benchmark-runner validation — failed

Generated offline benchmark-runner validation artifact — no provider execution performed.

The single permitted validation invocation aborted with `ABORT:VALIDATION_FAILED`.
No AMATL binary, provider, HTTP client, DNS lookup or socket was invoked.

No corrective implementation or second validation round was performed. The static
cause is preserved in `negative-tests.json`: the constructed duplicate test has
31 positions, and `validate_plan()` checks count before uniqueness, so it returns
`ABORT:PLAN_COUNT_MISMATCH` rather than the required `ABORT:DUPLICATE_POSITION`.
