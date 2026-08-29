# Local embedding relevance — feasibility (STEP 4A)

**Date:** 2026-08-29
**Branch:** `fix/audit-repository-hygiene`
**Baseline HEAD:** `22d3844` (production relevance unchanged from `66598c9`)
**Status:** feasibility experiment complete. No production change proposed yet.
**Experiment crate:** `experiments/embedding-relevance/` (outside the workspace).

All numbers below come from executed runs on this machine. Where a value could
not be measured it is marked `NOT_MEASURED`.

## 1. Objective

STEP 3B's independent 93-row hold-out showed the bounded-semantic relevance
model failing 5 of 6 acceptance gates, with ~68% of errors being
zero/low-overlap paraphrase, synonymy or conceptual equivalence
(RELEVANT recall 0.356). This experiment asks, empirically: can a small,
fully-local sentence-embedding model materially close that gap, at acceptable
runtime/packaging cost, with safe fallback?

This is an experiment. It does **not** replace the current relevance model and
changes **nothing** in routing, ranking, RRF, dedupe, canonicalization,
providers, diversity or telemetry.

## 2. Environment

| | |
|---|---|
| Kernel | `6.12.105+deb13-amd64` (Linux, x86_64) |
| Cores | 24 |
| RAM | 128 GB |
| rustc | 1.97.1 |
| Build profile for all numbers | `--release` |

## 3. Backends investigated

### A. `fastembed` 6.0.2 → `ort` 2.0.0-rc.13 (ONNX Runtime)  — PROTOTYPED

- **Prototyped and benchmarked** (`FastembedBackend`).
- `ort` build script downloads a **prebuilt static `libonnxruntime`** from
  `cdn.pyke.io` (`ms@1.28.0`) and links `static=onnxruntime` + dynamic
  `stdc++`. Only `x86_64-unknown-linux-gnu` (and other glibc triples) are
  published — **no musl build**.
- `fastembed` crate license: **Apache-2.0**. Pulls `tokenizers` 0.22,
  `hf-hub` 0.5, `ndarray` 0.17.
- CPU-only: yes (default execution provider).
- Model download: `hf-hub` fetches once into a cache dir
  (`./.fastembed_cache` here). No network at inference.

### B. `candle` 0.11 (+ `candle-transformers`)  — NOT PROTOTYPED, analysed

- Pure Rust, no C++ runtime, **cross-compiles to `x86_64-unknown-linux-musl`
  cleanly** — the amatl release target.
- `candle-transformers` ships BERT support usable for sentence-transformers
  models; would need ~150 lines of pooling/normalization glue that `fastembed`
  gives for free.
- Not prototyped in this phase because `fastembed` was the fastest path to
  real embedding *quality* numbers, and the quality result (§6) is what gates
  whether a `candle` port is worth doing.

### C. `tract`  — REJECTED for this phase

- Pure Rust, musl-friendly, but no batteries-included tokenizer/pooling and
  weaker transformer-op coverage than `candle`. If a pure-Rust path is
  pursued, `candle` is the better first choice.

Python runtime: **not used**. Hosted APIs / remote inference / LLM judges:
**not used**.

## 4. Selected experimental model

| field | value |
|---|---|
| `SELECTED_EXPERIMENTAL_MODEL` | `BAAI/bge-small-en-v1.5` (ONNX, via `Xenova/bge-small-en-v1.5`) |
| `MODEL_LICENSE` | MIT |
| `EMBEDDING_DIMENSION` | 384 |
| ONNX weights file | `model.onnx`, 133,093,490 bytes (**126.9 MiB**, fp32) |
| `MODEL_HASH` (sha256, HF content-address of the onnx blob) | `828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35` |
| tokenizer `tokenizer.json` sha256 | `d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66` |
| on-disk cache total | 128 MiB |

Second candidate, dev-split comparison only:
`sentence-transformers/all-MiniLM-L6-v2` — Apache-2.0, 384-dim, ~87 MiB onnx.

## 5. Performance (measured, `--release`, CPU)

### bge-small-en-v1.5

| metric | value |
|---|---|
| `MODEL_LOAD_TIME_MS` (cold, incl. one-time download) | 12293 |
| `MODEL_LOAD_TIME_MS` (warm, model cached) | ~191 |
| `RSS_BEFORE_MB` | ~8 |
| `RSS_AFTER_LOAD_MB` | ~191 |
| `RSS_DELTA_MB` | **~186** |
| `QUERY_EMBED_P50_MS` / `P95` | 3.30 / 4.46 |
| `DOCUMENT_EMBED_P50_MS` / `P95` | 6.21 / 7.66 |
| `BATCH_10_DOCUMENTS_MS` | ~37 |
| `BATCH_20_DOCUMENTS_MS` | ~56 |
| `SIMILARITY_COMPUTE` | ~220 ns / pair (384-dim cosine, scalar) |
| end-to-end relevance (embed query + 10 docs + 10 cosines), P50 / P95 | 37.9 / 43.1 ms |

### all-MiniLM-L6-v2 (comparison)

| metric | value |
|---|---|
| warm `MODEL_LOAD_TIME_MS` | NOT_MEASURED (cold run only: 9331 incl. download) |
| `RSS_DELTA_MB` | ~139 |
| `QUERY_EMBED_P50_MS` | 1.47 |
| `DOCUMENT_EMBED_P50_MS` | 2.87 |
| `BATCH_10_DOCUMENTS_MS` | ~20 |

Batching helps but is not dramatic on CPU (10 docs ≈ 6× a single doc, not
10×). Document embeddings can be computed per result set within a few tens of
ms — no persistent vector store is needed for this use.

## 6. Semantic diagnostic (CONSUMED corpora — diagnostic only, NOT a gate)

`corpus.json` (172) + `holdout.json` (93) = **265 labeled rows**. Thresholds
for modes C/D were swept on this consumed data; **no generalization is claimed
from these numbers.**

Modes:
- **A** lexical baseline (token-overlap classifier)
- **B** bounded-semantic *proxy* — a small local re-implementation
  (stemming-lite + alias groups + crude negative evidence). **Not** a call
  into amatl-core's production bounded-semantic assessment (the experiment
  crate is isolated); it is a weak reference column and under-reports what the
  real production model does on paraphrase.
- **C** embedding similarity only (bge-small cosine on query vs title+snippet)
- **D** hybrid: bounded-semantic proxy corroborated / moderated by embedding
  bands, with auditable fields and **negative evidence never blindly
  overridden**.

### Similarity separation — the central result

| class | n | mean cosine | spread |
|---|---|---|---|
| RELEVANT paraphrase (low lexical overlap) | 31 | **0.753** | min 0.630 |
| NOT_RELEVANT lexical collision / wrong-sense | 10 | **0.708** | max 0.804 |

**bge-small cosine does not cleanly separate the two target classes.** The
distributions overlap: the hardest lexical collisions score *higher* than the
softest true paraphrases. bge-small measures topical similarity, and a wrong
-sense / wrong-entity collision is usually topically adjacent. MiniLM is worse
here (paraphrase mean 0.612 vs collision 0.510, but paraphrase min 0.179).

### Mode comparison (265 rows)

| mode | acc | macro-F1 | RELEVANT P / R / F1 | NR→R |
|---|---|---|---|---|
| A lexical | 0.494 | 0.375 | 0.908 / 0.460 / 0.611 | 0.052 |
| B bounded-semantic proxy | 0.524 | 0.386 | 0.857 / 0.520 / 0.647 | 0.052 |
| C embedding-only | 0.679 | 0.467 | 0.864 / 0.887 / 0.875 | 0.086 |
| D hybrid | 0.660 | 0.463 | 0.870 / 0.800 / 0.833 | 0.069 |

### Per-failure-mode (correct / total), and D−B delta

| bucket | A | B | C | D | Δ(D−B) |
|---|---|---|---|---|---|
| PARAPHRASE_RELEVANT | 0/31 | 2/31 | **24/31** | 9/31 | **+0.226** |
| LEXICAL_COLLISION_NEGATIVE | 0/10 | 1/10 | 1/10 | 1/10 | +0.000 |
| ENTITY_MISMATCH | 48/48 | 48/48 | 19/48 | 39/48 | **−0.188** |
| PARTIAL | 13/37 | 11/37 | 26/37 | 14/37 | +0.081 |
| INSUFFICIENT_EVIDENCE | 1/20 | 1/20 | 1/20 | 1/20 | +0.000 |
| OTHER | 69/119 | 76/119 | 109/119 | 111/119 | +0.294 |

### Reading of the diagnostic

- **Embedding evidence clearly helps paraphrase recall** — the exact class the
  STEP 3B campaign failed. Embedding-only lifts paraphrase from ~2/31 to
  24/31.
- **It does so at a real cost to entity-mismatch discrimination**
  (48/48 → 19/48 for C). The hybrid contains that damage (→ 39/48) because
  the proxy's negative evidence still blocks — but does not eliminate it.
- **Lexical-collision negatives are not fixed by embeddings** (1/10 either
  way) — as the similarity-separation table predicts.
- Aggregate macro-F1 barely moves (0.386 → 0.463) because paraphrase gains and
  entity-mismatch losses partly cancel. The gain is real but **class-specific
  and not free**.
- The B column understates the production bounded-semantic model; the true
  D−B paraphrase delta against amatl-core would be smaller than +0.226.

## 7. Safety / fallback (spec §7–8) — all pass

`experiments/embedding-relevance/tests/`:

| case | result |
|---|---|
| EMPTY_QUERY | zero vector, no error |
| EMPTY_TITLE / EMPTY_SNIPPET / EMPTY doc | zero vector; cosine 0.0, never NaN |
| NON_ASCII (accents, CJK, Greek, emoji) | embeds, non-zero |
| LONG_INPUT (~120k chars) | truncates, no panic |
| REPEATED_INPUT_DETERMINISM | max abs diff < 1e-6 across runs |
| MODEL_MISSING | clean `Err`, no panic — caller can fall back |
| MODEL_CORRUPT | clean `Err`, no panic |

`cosine_similarity` is total (returns 0.0 on degenerate input). The
`EmbeddingBackend` trait returns `anyhow::Result`, so a caller wires
"embedding unavailable → keep bounded-semantic" with a single `match`.

## 8. Deployment reality

| field | value |
|---|---|
| `RUST_DEPENDENCIES_ADDED` (experiment only) | `fastembed` 6, `ort` 2.0-rc.13, `tokenizers` 0.22, `hf-hub` 0.5, `ndarray` 0.17, `serde_json`, `anyhow` |
| `NATIVE_LIBRARIES_REQUIRED` | static `libonnxruntime` 1.28 (downloaded by `ort` at build time from `cdn.pyke.io`); dynamic `libstdc++` |
| `CPU_ONLY_SUPPORTED` | yes |
| `OFFLINE_AFTER_INSTALL` | yes — static ORT, model in local cache; no inference-time network |
| `MODEL_DISTRIBUTION_STRATEGY` | **open question.** Options: (a) vendor the 127 MiB onnx + tokenizer in the release artifact; (b) first-run download with an integrity-pinned hash and an explicit opt-in. Runtime auto-download is **not acceptable** per spec. |
| `MODEL_CACHE_PATH` | configurable (`FASTEMBED_CACHE_PATH`); experiment uses `./.fastembed_cache` |
| `BINARY_SIZE_DELTA` | experiment `bench` binary ≈ 37 MB (statically-linked ORT). A production integration would add a similar static-ORT delta to the amatl binary. |
| `PACKAGING_RISK` | **High for the stated release target.** amatl releases target `x86_64-unknown-linux-musl` (root `Cargo.toml` `[workspace.metadata.release]`). pyke publishes **no musl ORT build**; a musl release would need a self-built ONNX Runtime or a switch to the glibc target. `candle` (pure Rust) avoids this entirely. |

## 9. Decision

**`LOCAL_EMBEDDING_FEASIBLE_WITH_COST`** — with an explicit caveat that the
semantic gain is narrower than hoped.

Reasoning against the spec §9 criteria:

| criterion | verdict |
|---|---|
| semantic paraphrase discrimination materially improves | **partly** — strong on paraphrase recall; does **not** fix lexical-collision or wrong-sense negatives, and costs entity-mismatch precision unless carefully gated |
| lexical-collision false positives remain controllable | **only via the hybrid's negative-evidence gate**; embedding-only is not safe here |
| latency / memory reasonable for amatl | **yes on glibc CPU** — ~4 ms/query, ~40 ms per 10-doc result set, +186 MB RSS |
| offline Linux packaging practical | **not on the musl release target with `ort`.** Practical with `candle`, or by moving amatl to a glibc release target |
| fallback safe | **yes** |

`SEMANTIC_GENERALIZATION_GAIN`: **UNPROVEN.** The paraphrase gain is measured
only on consumed data. Whether it generalizes — and whether it survives the
unchanged §9 acceptance gates on a fresh blind hold-out — is exactly what
STEP 4B must test.

## 10. Recommended next steps (not done in this phase)

1. **Do not** change the production default. Bounded-semantic stays.
2. If STEP 4B proceeds: port the backend to **`candle`** (pure Rust, musl-safe)
   before any production wiring, OR get an explicit decision to move amatl's
   release target to glibc. Keep `fastembed` only as the dev-time reference.
3. Design the STEP 4B hybrid around the measured reality: embedding evidence as
   a **paraphrase-recall booster that is subordinate to negative/entity
   evidence**, never an independent RELEVANT vote.
4. Build the new ≥100-row blind hold-out (§5 of the STEP 4 plan) with inputs
   frozen in a commit *before* any threshold selection, and — given the
   single-annotator gap flagged in the methodology audit — a second
   independent labelling pass before it gates anything.
5. Resolve model distribution (vendored vs pinned first-run download) before
   any release consideration.

## Scope confirmation

`PRODUCTION_DEFAULT_CHANGED=NO` · `ROUTING_CHANGED=NO` · `TELEMETRY_CHANGED=NO`
· `TELEMETRY_UNLOCKED=NO` · no new blind hold-out created · no labels inspected.
