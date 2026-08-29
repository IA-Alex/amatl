# Findings

Generated benchmark artifact — AMATL SearXNG Baseline v2 post-change.

MEASUREMENT: 30/30 planned, recorded, unique, sequential executions completed; retries=0 and Marginalia executions=0.
MEASUREMENT: V2 counts are SUCCESS=0, PARTIAL_SUCCESS=0, ZERO_RESULTS=0, FAILURE=30; usable-result rate=0.0000.
OBSERVATION: the V2 post-change state presented 0.0000 usable-result rate versus 0.5333 in V1.
INFERENCE: DEGRADED; this comparison does not establish that disabling DuckDuckGo, Mojeek, and Qwant caused the difference.
Confounders: different execution time, variable upstream-engine availability and rate limiting, and the single 30-run sample; normal AMATL JSON does not expose upstream HTTP or engine-level result details.
