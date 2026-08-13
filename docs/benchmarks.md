# Benchmarks and quality gates

AMATL separates deterministic quality evaluation from operational performance
measurement. A quality gate may enable Ranking v2; a local microbenchmark does
not prove production latency, memory or contention.

## Ranking v2 corpus and gate

`crates/amatl-core/benchmarks/ranking_v2_corpus.json` is embedded into the
binary under ID `ranking-v2-human-labeled-v2`. It contains five human-labeled
queries and four documents per query (20 judgments), each with URL, title,
snippet, content, relevance grade 0–3, and an MVP/provider rank. The baseline is
that stored provider order. The candidate is deterministic BM25 plus separate
evidence weighting according to policy v2.

The report averages nDCG@3 and MRR over queries. Default admission requires both
candidate nDCG@3 ≥ 0.90 and improvement over baseline ≥ 0.05
(`ranking_v2.rs:404-465`). Deep constructs `RankingV2Engine` before use; an
invalid/rejected policy cannot silently promote the candidate.

Reproduced on 2026-08-12 from the current tree:

```text
benchmark_id: ranking-v2-human-labeled-v2
queries: 5
baseline nDCG@3: 0.6557679882437799
candidate nDCG@3: 0.9193779960897104
delta: 0.26361000784593047
baseline MRR: 0.9
candidate MRR: 0.9
passed: true
```

This is evidence for the checked-in fixture only, not a claim about external
search quality. Reproduce with:

```bash
cargo run -p amatl-cli -- benchmark ranking-v2 --json
```

Changing labels, queries, documents, baseline ranks, policy weights or thresholds
requires review of human judgment, a new report, and an ADR when interpretation
changes.

## Criterion microbenchmarks

```bash
cargo bench -p amatl-core --bench core_contracts
```

Current Criterion cases are `query_parse_and_classify` and `canonicalization`
(`benches/core_contracts.rs`). Record CPU, kernel, Rust version, build profile,
sample settings and commit when comparing results. Do not check in a single
machine's timing as a universal SLA.

## Controlled operational harness

`benchmark operational` is a bounded local harness. It exercises Search with a
fixed successful provider plus a fixed failure, Deep with in-process fetch and
extraction fixtures, Tokio concurrency, and concurrent SQLite cold writes/warm
reads. It reports p50/p95/p99/max, throughput, status rates, cache hit/write
rates and Linux peak RSS when `/proc` is available.

```bash
cargo run --locked --release -p amatl-cli -- \
  benchmark operational --json --iterations 64 --concurrency 8
```

Snapshot reproduced on 2026-08-13 from the release profile, Linux
6.12.101+deb13-amd64 x86_64, rustc 1.97.1:

```text
workload: controlled-local-v1
iterations/concurrency: 64/8
Search p50/p95/p99: 3.142/3.242/3.280 ms
Search throughput: 2522.56 requests/s
Search success/partial/failure: 0.000/1.000/0.000
Deep p50/p95/p99: 2.077/2.104/2.109 ms (32 samples)
SQLite cold-write p95: 0.715 ms; write success: 1.000
SQLite warm-read p95: 0.461 ms; hit rate: 1.000
peak RSS: 15077376 bytes
```

These numbers are evidence for one controlled machine, not a production SLA.
The forced provider failure deliberately makes every useful Search response
`partial_success`; that verifies degradation accounting rather than external
availability.

## Remaining environmental measurements

The controlled harness does not claim real-provider latency, Internet failure
behavior, provider cost/query, external top-K quality, Renderer fallback,
dedupe/RRF marginal gain or target-host capacity. Provider network runs require
an approved governance record and credentials and are isolated behind the
manual `provider-canary` workflow. Renderer metrics remain blocked until an
isolated backend exists. Production sizing must also repeat the harness on the
deployment host with representative traffic and retention settings.
