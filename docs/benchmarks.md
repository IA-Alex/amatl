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

## Required operational suite

The golden plan requires measurement of total/provider/Deep latency,
throughput, peak memory, Tokio concurrency, SQLite contention, dedupe/RRF,
routing and marginal gain, extraction, Renderer fallback, provider contribution,
top-K quality, error/partial rates and cost/query. **These operational benchmarks
are not implemented in the current repository.**

A complete harness should provide fixed local HTTP providers, controlled delay
and failure distributions, warm/cold cache runs, concurrency levels, SQLite
read/write contention, byte/fetch/deadline exhaustion, and memory capture. Report
p50/p95/p99, throughput, peak RSS, error/partial rate, unique useful results,
unique domains, duplicate ratio, marginal gain and cost. Renderer metrics remain
blocked until an isolated backend exists.

No operational performance or production capacity claim should be made from the
current two Criterion cases or the 20-judgment ranking corpus.
