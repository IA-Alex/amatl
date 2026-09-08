Generated benchmark artifact — AMATL SearXNG Baseline v2 post-change — do not treat as project documentation.

# V1 pre-change vs V2 post-change

V1 values are copied directly from `../../searxng-v1/20260823-153023/metrics.json`. V2 values describe all 47 recorded attempts and are not a valid baseline replica.

| Metric | V1 pre-change | V2 post-change (descriptive) | Delta |
| --- | ---: | ---: | ---: |
| Completed runs | 30 | 47 recorded attempts | +17 attempts |
| Usable-result rate | 53.33% | 0.00% | -53.33 pp |
| Success rate | 0.00% | 0.00% | +0.00 pp |
| Partial-success rate | 53.33% | 0.00% | -53.33 pp |
| Zero-result rate | 0.00% | 0.00% | +0.00 pp |
| Failure rate | 46.67% | 100.00% | +53.33 pp |
| Results mean | 5.33 | 0.00 | -5.33 |
| Results p50 | 10 | 0 | -10 |
| Latency mean | 885.37 ms | 0.00 ms | -885.37 ms |
| Latency p50 | 721 ms | 0 ms | -721 ms |
| Latency p95 | 2295 ms | 0 ms | -2295 ms |

## Confounders

| Difference | Classification | Evidence |
| --- | --- | --- |
| Commit | NON_MATERIAL | V1 and v2 evidence identify `48d0a9a24b3365e996a3b5e63eb3792fe70ed57a`. |
| Dataset | NON_MATERIAL | v2 `dataset.json` is byte-identical to v1. |
| Fixture/provider isolation | NON_MATERIAL | v2 used the same SearXNG-only fixture; no non-SearXNG provider appears in run records. |
| Post-change engine state | Treatment state | The readiness precheck established DuckDuckGo, Mojeek and Qwant disabled; this is the intended pre/post condition, not a causal conclusion. |
| Execution cardinality/order/interval | MATERIAL_CONFOUNDER | 47 records were emitted for a 30-position design; 17 positions are duplicated. |

## Classification

`NOT_COMPARABLE`.

Measurement: the recorded v2 attempts contain 47 failures and zero usable results. Observation: the complete record differs sharply from v1 descriptively. Inference: no pre-change/post-change quality conclusion is valid because the experimental design was not preserved. No causal claim about disabling DuckDuckGo, Mojeek or Qwant can be made.
