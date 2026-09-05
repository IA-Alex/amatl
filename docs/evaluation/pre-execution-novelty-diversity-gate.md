# PRE_EXECUTION_NOVELTY_DIVERSITY_GATE

The ADR-012 gate is an offline precondition of the candidate-universe freeze
boundary. A candidate remains `CANDIDATE` until the gate returns `PASS`; the
normal freeze helper raises `FREEZE_BLOCKED...` for every other decision.
It does not predict relevance and does not use labels, providers, network
requests, productive URL canonicalization, ranking, or routing.

`tools/pre_execution_novelty_diversity_gate.py` preserves original query text
and derives a minimal normalized form (Unicode NFC, casefold, trim and
whitespace collapse). Internal diversity uses token sets, deterministic
Jaccard similarity, and explicit family/topic fields where supplied. It emits
query novelty/overlap, duplicate and near-duplicate, family, pair, arm-balance,
capacity, and historical-result-risk metrics. Historical result novelty is
`INSUFFICIENT_EVIDENCE` when no local result manifests are supplied; no future
result claim is made.

With result manifests, the descriptive estimate is reproducible: historical
URL overlap risk is `historical_unique_urls / (historical_unique_urls +
candidate_query_count)`, domain concentration is the largest observed domain
count divided by all observed domain counts, and the risk score is the mean of
one minus query novelty and the URL-risk estimate. These are risk indicators,
not predictions of future retrieval.

Thresholds are configurable through the gate constructor and are recorded in
the evidence manifest. The checked-in
`tools/pre_execution_novelty_diversity_gate.toml` contains ADR-012 policy
defaults; these defaults are policy values, not statistically validated
estimates. The decision values are `PASS`, `FAIL_NOVELTY`, `FAIL_DIVERSITY`,
`FAIL_BALANCE`, `FAIL_CAPACITY`, and `FAIL_INTEGRITY`.

Each evaluation can write a canonical JSON manifest containing the candidate
hash, historical source hashes, thresholds, metrics, decision, reasons,
provenance, and `artifact_sha256`. The V5 replay and positive-control tests
are network-free and do not rewrite V5 artifacts.
