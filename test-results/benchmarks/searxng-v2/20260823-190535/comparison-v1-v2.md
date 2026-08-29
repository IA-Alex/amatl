# Baseline v1 ↔ v2 comparison

Generated benchmark artifact — AMATL SearXNG Baseline v2 post-change.

| Métrica | V1 pre-change | V2 post-change | Delta |
|---|---:|---:|---:|
| Completed runs | 30 | 30 | +0.00 |
| Usable-result rate | 0.5333 | 0.0000 | -53.33 pp |
| Success rate | 0.0000 | 0.0000 | +0.00 pp |
| Partial-success rate | 0.5333 | 0.0000 | -53.33 pp |
| Zero-result rate | 0.0000 | 0.0000 | +0.00 pp |
| Failure rate | 0.4667 | 1.0000 | +53.33 pp |
| Results mean | 5.3333 | 0.0000 | -5.33 |
| Results p50 | 10 | 0 | -10.00 |
| Latency mean | 885.3667 | 296.9333 | -588.43 |
| Latency p50 | 721 | 122 | -599.00 |
| Latency p95 | 2295 | 724 | -1571.00 |

MEASUREMENT: the rows above are direct aggregate measurements from the two immutable artifacts.
OBSERVATION: V2 ran with the SearXNG-only fixture and DuckDuckGo, Mojeek, and Qwant disabled.
INFERENCE: the post-change state is compared with V1; time-varying upstream/provider conditions prevent causal attribution to the disabled engines alone.
