# STEP 4D — real Candle integration + optimized execution

Status: **STEP_4D_REAL_CANDLE_INTEGRATION_COMPLETE**
Base: `566679d` (STEP 4C). Branch: `fix/audit-repository-hygiene`.

All numbers below come from executed runs on this machine (glibc host for the
latency/functional runs; user-space musl toolchain for the hard gate — see
below). No estimated or fixture-calibrated latency.

## What moved into the workspace

The real pure-Rust Candle backend now lives **inside `amatl-core`**, behind the
existing `experimental-local-embeddings` feature (still OFF by default):

| file | role |
|---|---|
| `experimental_embeddings/mod.rs` | the STEP 4C typed seam (`EmbeddingBackend`, `SemanticRescore`, `evaluate_with_embeddings`, `FixtureBackend`) — unchanged contract |
| `experimental_embeddings/candle.rs` | real `CandleBackend` (BERT / `bge-small-en-v1.5`, CLS pooling + L2), adapted from the STEP 4B reference: loads from a validated `ModelPackage` (no download path), fallible with `EmbeddingUnavailable`, batched `embed_documents` |
| `experimental_embeddings/model_config.rs` | `ModelPackage` — `MODEL_PATH` / `MODEL_VERSION` + pinned sha256 of `model.safetensors` / `tokenizer.json` / `config.json`. Purely local, fails closed |
| `experimental_embeddings/pipeline.rs` | `SemanticEvaluator` — the optimized execution architecture |

Feature dependency change: `experimental-local-embeddings` now pulls
`candle-core`/`candle-nn`/`candle-transformers` 0.9.2 + `tokenizers` 0.20
(`unstable_wasm`, no `onig_sys`) + `safetensors` + `memmap2`, **all
`optional = true` / `dep:`-gated**. The default build's dependency graph is
unchanged.

## Optimized execution architecture

The naive STEP 4C path (query embedded per-result, every result embedded,
2 embeds/result) is replaced by:

* **Query embedding — exactly once per search.** `SemanticPassOutcome.query_embed_count == 1`
  whenever the pass runs (asserted by `real_pipeline_query_embedded_once_and_bounded`
  and the latency test).
* **Document embedding — one batch.** The whole bounded candidate set goes
  through a single BERT forward pass (`document_embed_batches == 1`).
* **Bounded candidate set.** `DEFAULT_EXPERIMENTAL_K = 8` (spec: "5–10").
  Only `PossiblyRelevant` results (the ambiguous band) are considered; strong
  results and any result carrying blocking negative/entity evidence are
  excluded before an embed is spent.
* **Advisory only, fixed precedence:**
  `explicit negative/entity evidence > semantic embedding evidence > lexical corroboration`.
  The production `assess_result` classification is passed through untouched.
  The embedding can only produce a *suggested* `PossiblyRelevant → Relevant`,
  and only when there is no contradiction **and** the bounded-semantic layer
  independently corroborates (`bounded_strong_rescue`). It is never a sovereign
  classifier; a contradiction case is never promoted.

Provisional threshold `0.72` (paraphrase agreement) is the frozen STEP 4A/4B
value — **not** re-tuned against any hold-out (spec §10).

## Real E2E latency (glibc host, `--release`, `bge-small-en-v1.5` loaded)

```
MODEL_LOAD_TIME_MS = 151.4
RSS_DELTA_MB        = 136.1   (4.5 → 140.6 MB; the model, not the code)
```

| results/search | SEMANTIC_CANDIDATES | QUERY_EMBED | DOC_BATCHES | BASELINE p50 / p95 (ms) | CANDLE p50 / p95 (ms) | INCREMENTAL p50 / p95 (ms) |
|---:|---:|---:|---:|---|---|---|
| 5  | 4 | 1 | 1 | 0.072 / 0.075 | 56.98 / 60.21 | **+56.91 / +60.14** |
| 10 | 8 | 1 | 1 | 0.155 / 0.268 | 83.61 / 89.71 | **+83.45 / +89.44** |
| 20 | 8 (K-capped) | 1 | 1 | 0.298 / 0.356 | 81.15 / 84.99 | **+80.85 / +84.63** |

`CPU_TIME_DELTA_MS` ≈ 540–640 ms per Candle search (single-threaded gemm).

Compare STEP 4C naive path: **+241 / +468 / +921 ms**. The optimized path is
~4–11× cheaper and, crucially, **flat past K** — a 20-result search costs the
same as a 10-result one because the candidate set is capped at 8.

`LATENCY_DECISION = LATENCY_ACCEPTABLE_WITH_OPTIMIZATION`. AMATL defines no
per-request SLO; the only bound is the 30 s request deadline, against which an
~60–90 ms incremental cost on the *experimental, opt-in* path is comfortably
within budget. Further optimization (candidate cap lower than 8, snippet
truncation, a rayon gemm pool) is possible but not required to pass the phase.

## MUSL HARD GATE

```
cargo build --locked --release --target x86_64-unknown-linux-musl \
  -p amatl-cli --features amatl-core/experimental-local-embeddings
```

Environment note: `musl-tools` is not installed and there is no sudo. Resolved
in user space — `apt-get download` the `musl` / `musl-dev` / `musl-tools`
`.deb`s, extract to a scratchpad prefix, generate a patched `musl-gcc.specs`
and an `x86_64-linux-musl-gcc` wrapper on `PATH`. The STEP 4A–4C
`BLOCKED_BY_ENVIRONMENT` is lifted.

```
FULL_AMATL_MUSL_BUILD_STATUS = PASS
CANDLE_LINKED_IN_FULL_BUILD  = YES   (17 candle/bert/gemm symbols in the binary;
                                      "Represent this sentence…", "safetensors"
                                      strings present)
STATIC_LINK_STATUS           = static-pie, statically linked
DYNAMIC_RUNTIME_DEPS         = none  (`ldd` → "statically linked")
FEATURE_OFF_BINARY_SIZE_MB   = 28.45  (0 candle symbols)
FEATURE_ON_BINARY_SIZE_MB    = 29.44  (+~1.0 MB — Candle is small; the 128 MB
                                       model is NOT vendored)
```

Baseline (feature-off) full musl build also re-verified: **PASS**.

## Packaging validation (`MODEL_DISTRIBUTION_OPTIONAL_PACKAGE` preserved)

| case | behaviour | evidence |
|---|---|---|
| A — binary, model package absent | search works, bounded-semantic fallback; `ModelPackage::resolve` → `ModelMissing`; `SemanticEvaluator::load_backend` → `None` | `model_missing_yields_model_missing_error` |
| B — binary + valid model package | Candle activates; loads offline, embeds, pipeline runs with `query_embed_count == 1` | `real_backend_loads_and_embeds_offline`, `real_pipeline_query_embedded_once_and_bounded` |
| C — binary + invalid model package (corrupt / hash mismatch / partial) | clean fallback, fail-closed; `HashMismatch` / `TokenizerMissing` / `ConfigMissing` | `model_corrupt_and_hash_mismatch_fall_back`, `tokenizer_and_config_missing_detected` |

`OFFLINE_OPERATION = YES` — no network path anywhere in `model_config` or
`candle` (no `curl`, no `hf-hub`); a missing file is an error, never a fetch.

## Invariants held

* `PRODUCTION_DEFAULT_CHANGED = NO` — relevance/ranking/router untouched; only
  `lib.rs` module gate + comment changed on the default path.
* `ROUTING_CHANGED = NO`, `TELEMETRY_CHANGED = NO`, `TELEMETRY_UNLOCKED = NO` —
  the pipeline module imports none of `router` / `telemetry` / provider code.
* `FEATURE_OFF_BEHAVIOR_UNCHANGED = YES` — no dependency-graph change, no
  code-path change; feature-off musl binary has zero candle symbols and
  identical `--version`.
* `FALLBACK_BEHAVIOR = PASS` — every `ModelPackageError` / `EmbeddingUnavailable`
  variant degrades to the bounded-semantic outcome, no panic, no result loss,
  no provider suppression.
* No new blind hold-out. No semantic tuning against consumed labels. No
  generalization claimed — that is STEP 4E.

## Reproduce

```
# unit + fallback matrix (no model needed)
cargo test -p amatl-core --features experimental-local-embeddings

# real Candle integration + latency (model package required)
AMATL_TEST_MODEL_DIR=/path/to/pkg cargo test -p amatl-core \
  --features experimental-local-embeddings --release \
  --test experimental_candle_integration --test experimental_candle_latency \
  -- --ignored --nocapture --test-threads=1

# musl hard gate
cargo build --locked --release --target x86_64-unknown-linux-musl \
  -p amatl-cli --features amatl-core/experimental-local-embeddings
```

A valid model package is the three files
`model.safetensors` / `tokenizer.json` / `config.json` from
`huggingface.co/BAAI/bge-small-en-v1.5/resolve/main`, matching the sha256
digests pinned in `model_config::PINNED_HASHES`.
