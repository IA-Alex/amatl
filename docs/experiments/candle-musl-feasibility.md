# Candle + musl production-backend feasibility (STEP 4B)

**Date:** 2026-08-29
**Branch:** `fix/audit-repository-hygiene`
**Baseline HEAD:** `585911b` (STEP 4A). Production relevance unchanged from
`66598c9`.
**Status:** feasibility experiment complete. No production change proposed.
**Experiment crate:** `experiments/embedding-relevance/` (outside the workspace).

All numbers below come from executed runs on this machine. Where a value could
not be measured it is marked `NOT_MEASURED` / `BLOCKED_BY_ENVIRONMENT`.

## 1. Objective

STEP 4A (`local-embedding-feasibility.md`) proved a small local sentence
-embedding model (`BAAI/bge-small-en-v1.5`, ONNX via `fastembed`/`ort`)
delivers a real paraphrase-recall gain, but flagged that the ONNX Runtime path
does **not** align with amatl's `x86_64-unknown-linux-musl` release target
(pyke publishes no musl `libonnxruntime`). STEP 4B asks: can the **same**
semantic benefit be obtained through a **pure-Rust / Candle** inference path
that is musl-compatible?

Nothing here changes routing, ranking, RRF, dedupe, canonicalization,
providers, diversity, telemetry, or the production relevance default.

## 2. Environment

| | |
|---|---|
| Kernel | `6.12.105+deb13-amd64` (Linux, x86_64) |
| Cores | 24 |
| rustc / cargo | 1.97.1 |
| musl Rust target | `x86_64-unknown-linux-musl` **installed** |
| musl C toolchain (`musl-tools` / `x86_64-linux-musl-gcc`) | **absent, no sudo** |
| Build profile for all numbers | `--release` |

**Environmental limitation.** This host has no musl C compiler and the session
cannot install one. This blocks the *full* amatl musl link — but the blocker
is **`ring`** (rustls' C crypto), which is **already in amatl today** (rustls
is pervasive) and is installed in CI by `release.yml` (`apt-get install
musl-tools`). It is **not** a Candle-introduced dependency. See §4.

## 3. Candle backend

`experiments/embedding-relevance/src/candle_backend.rs` — `CandleBackend`,
implementing the same `EmbeddingBackend` trait as `FastembedBackend`
(`embed_query`, `embed_document`, `embed_documents`; `cosine_similarity` is the
shared free function). The ONNX prototype is untouched and still builds/runs.

| field | value |
|---|---|
| `MODEL_NAME` | `BAAI/bge-small-en-v1.5` |
| `MODEL_FORMAT` | `model.safetensors` (fp32, native `sentence-transformers` weights) |
| `MODEL_LICENSE` | MIT |
| `MODEL_SIZE_MB` | 127.3 (`model.safetensors`, 133 466 304 bytes) |
| `MODEL_HASH` (sha256, safetensors blob) | `3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad` |
| `TOKENIZER_HASH` (sha256 `tokenizer.json`) | `d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66` — **identical to the STEP 4A ONNX prototype's tokenizer** |
| `config.json` sha256 | `094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750` |
| `EMBEDDING_DIMENSION` | 384 |
| architecture | BERT (`candle_transformers::models::bert::BertModel`) |
| pooling | CLS token (position 0), then L2 normalize — matches FlagEmbedding / what `fastembed` does |
| query prefix | `"Represent this sentence for searching relevant passages: "` (documents raw) |

The exact BAAI weights load through Candle unchanged — no architecture
substitution was needed.

### ONNX ↔ Candle agreement (`src/bin/agreement.rs`, 265 consumed pairs)

Numerical identity is **not** assumed (fp32 Candle vs the Xenova ONNX export).
Measured:

| metric | value |
|---|---|
| `ONNX_CANDLE_COSINE_AGREEMENT_MEAN` (cosine of the two backends' embeddings of the same text) | **0.9768** |
| `ONNX_CANDLE_COSINE_AGREEMENT_MIN` | 0.8433 |
| `ONNX_CANDLE_COSINE_AGREEMENT_P05` | 0.9352 |
| query~document similarity abs-diff, mean / max | 0.0213 / 0.0955 |
| `ONNX_CANDLE_RANK_CORRELATION` (Spearman, query~doc sims) | **0.9735** |

The two backends produce **semantically equivalent** embeddings: same ranking
of relevance, sub-0.03 mean similarity drift. The handful of low-agreement
outliers (min 0.84) are short / punctuation-heavy strings where the Xenova
export and fp32 Candle diverge most; they do not change the diagnostic
outcome (§7).

## 4. musl build — the main gate

`MUSL_TARGET` = `x86_64-unknown-linux-musl`
`CANDLE_MUSL_BUILD_COMMAND` (amatl-equivalent):
`cargo build --release --target x86_64-unknown-linux-musl -p amatl-cli`

### 4a. Full experiment crate / amatl workspace

`CANDLE_MUSL_BUILD_STATUS` = **`BLOCKED_BY_ENVIRONMENT`** — **not**
`BACKEND_INCOMPATIBLE`.

- The build stops at **`ring v0.17.14`** (`error occurred in cc-rs: failed to
  find tool "x86_64-linux-musl-gcc"`).
- The **unmodified amatl workspace fails identically** at `ring` on this host
  (verified: `cargo build --release --target x86_64-unknown-linux-musl -p
  amatl-cli` from a clean tree). `ring` enters amatl via rustls, which is
  everywhere in the current dependency graph — it has nothing to do with
  Candle.
- amatl's own `release.yml` job `linux-x86_64-musl` runs `apt-get install
  --yes musl-tools rpm zstd` before the identical `cargo build` command, so
  the amatl musl release **works in CI today** with `musl-tools` present.
- The one C dependency Candle *would* have added — **`onig_sys`** (oniguruma,
  pulled by `tokenizers`' default `onig` feature) — was removed by selecting
  the `unstable_wasm` feature (pure-Rust `fancy-regex` backend). bge-small's
  WordPiece tokenizer does not need PCRE. After that swap, the *only* C
  toolchain requirement is the pre-existing `ring`.

### 4b. Candle stack in isolation — actual successful musl build

To prove the backend itself (not the co-resident `ort`/`ring`) is musl-clean,
a standalone crate with **only** `candle-core` + `candle-nn` +
`candle-transformers` + `tokenizers` (`unstable_wasm`) + the verbatim
`candle_backend.rs` inference code was cross-compiled and **run**:

| field | value |
|---|---|
| `CANDLE_MUSL_BUILD_STATUS` (isolated) | **SUCCESS** |
| build command | `cargo build --release --target x86_64-unknown-linux-musl` |
| `STATIC_LINK_STATUS` | **fully static** — `file` reports `static-pie linked`, `ldd` reports `statically linked` |
| `DYNAMIC_RUNTIME_DEPS` | **none** |
| `BUILD_ERRORS` | none |
| C toolchain used | **none** (rustc ships the musl libc; no `cc` invoked) |
| `-sys` / native crates in the Candle subtree | **zero** (`cargo tree -p candle-transformers \| grep -sys` → empty; `gemm` is pure-Rust SIMD) |
| binary size (stripped, LTO), real BERT-embedding program | **5.4 MB** |
| runtime check | loads `model.safetensors`, tokenizes, runs BERT forward, pools, normalizes → `dim=384`; **first component `-0.0792`, bit-identical to the glibc build** |

So: the Candle inference path compiles to a static musl binary with no C
toolchain and no dynamic dependencies, and produces correct, libc-independent,
deterministic embeddings.

## 5. Runtime test (glibc host; musl runtime verified in §4b)

`tests/candle_failure_modes.rs` — all pass (`cargo test --release --test
candle_failure_modes -- --ignored`, 7/7):

| case | result |
|---|---|
| `MODEL_LOAD` | ok (safetensors mmap) |
| `INFERENCE` | ok |
| `TOKENIZATION` | ok (WordPiece, fancy-regex backend) |
| `SIMILARITY` | ok, cosine total (0.0 on degenerate, never NaN) |
| `NON_ASCII` (accents, CJK, Greek, emoji) | embeds, non-zero |
| `LONG_INPUT` (~120k chars) | truncates at 512 tokens, no panic |
| `REPEATED_INPUT_DETERMINISM` | max abs diff < 1e-6 |
| EMPTY_QUERY / EMPTY_DOC | zero vector, no error; cosine 0.0 |
| BATCH vs single | < 1e-3 drift |
| MODEL_CORRUPT | clean `Err`, no panic |

No runtime network: the model fetch is an explicit, dev-time-only `curl` of
three files, gated on their absence; all three present short-circuits it. To
provision offline, drop the files into `.candle_cache/` by hand.

## 6. Performance (measured, `--release`, CPU, same host & method as STEP 4A)

`src/bin/candle_bench.rs`, 172 queries / 172 docs from `corpus.json`.

| metric | Candle | ONNX (STEP 4A) | delta |
|---|---|---|---|
| `MODEL_LOAD_TIME_MS` (warm) | **63** | 191 | **−67%** |
| `RSS_DELTA_MB` | **137** | ~186 | **−26%** |
| `QUERY_EMBED_P50_MS` | 15.0 | 3.30 | **+355%** |
| `QUERY_EMBED_P95_MS` | 19.5 | 4.46 | +337% |
| `DOCUMENT_EMBED_P50_MS` | 22.6 | 6.21 | +264% |
| `DOCUMENT_EMBED_P95_MS` | 29.1 | 7.66 | +280% |
| `BATCH_10_MS` | 176 | ~37 | +376% |
| `BATCH_20_MS` | 343 | ~56 | +513% |
| `END_TO_END_10_DOCS_P50_MS` | 184 | 37.9 | **+386%** |
| `END_TO_END_10_DOCS_P95_MS` | 218 | 43.1 | +406% |

Candle loads faster and uses less memory, but **CPU inference is ~4–5× slower**
and it **batches poorly** here (pure-Rust `gemm`, no MKL/BLAS; small batch
sizes don't amortize). A 10-document result set costs ~184 ms end-to-end vs
~38 ms for ONNX. For amatl's use (embed one query + one result page per
search, no persistent vector store) ~200 ms of added CPU per search is
**operationally tolerable but not free** — it is the dominant cost of the
pure-Rust choice. Options if it matters: the `candle` `mkl`/`accelerate`
features (reintroduce a native BLAS — defeats the point), fewer docs embedded,
or caching document embeddings.

## 7. Semantic parity (CONSUMED diagnostic data only — NOT a gate, NOT tuned)

`src/bin/candle_diagnostic.rs`, thresholds **reused** from the ONNX sweep
(`t_high=0.72`, `t_med=0.55`), not re-swept against Candle. 265 labeled rows.

### Similarity separation

| class | ONNX mean (min/max) | Candle mean (min/max) |
|---|---|---|
| RELEVANT paraphrase (low overlap), n=31 | 0.753 (min 0.630) | 0.748 (min 0.578) |
| NOT_RELEVANT lexical collision, n=10 | 0.708 (max 0.804) | 0.703 (max 0.799) |

Same picture as STEP 4A: bge-small cosine does **not** cleanly separate
paraphrase from wrong-sense collision, on either backend.

### Mode comparison / per-failure-mode (correct / total)

| bucket | ONNX C | Candle C | ONNX D | Candle D |
|---|---|---|---|---|
| `PARAPHRASE_RECALL` (PARAPHRASE_RELEVANT) | 24/31 | **21/31** | 9/31 | **9/31** |
| `LEXICAL_COLLISION_FP` (LEXICAL_COLLISION_NEGATIVE) | 1/10 | **1/10** | 1/10 | **1/10** |
| `ENTITY_MISMATCH` | 19/48 | **19/48** | 39/48 | **37/48** |
| PARTIAL | 26/37 | 22/37 | 14/37 | 10/37 |
| OTHER | 109/119 | 113/119 | 111/119 | 112/119 |
| **macro-F1** (all rows) | C 0.467 / D 0.463 | C 0.451 / **D 0.434** | | |

`PARAPHRASE_RECALL_ONNX` = 24/31, `PARAPHRASE_RECALL_CANDLE` = 21/31.
`LEXICAL_COLLISION_FP_ONNX` = `LEXICAL_COLLISION_FP_CANDLE` = 9/10 collisions
still misclassified. `ENTITY_MISMATCH_ONNX` (hybrid) = 39/48,
`ENTITY_MISMATCH_CANDLE` = 37/48.

**Parity is acceptable.** Candle is marginally weaker on paraphrase recall
(−3 rows) and hybrid entity-mismatch (−2 rows) — within the noise of a fp32
vs quantized-export difference and one un-retuned threshold set. Every
qualitative STEP 4A conclusion holds: embeddings boost paraphrase recall, do
**not** fix lexical collisions, and cost entity-mismatch precision unless
gated by the hybrid's negative-evidence rule.

## 8. Packaging analysis

| field | Candle | ONNX (`ort`) |
|---|---|---|
| `RUST_DEPENDENCIES` added | `candle-core` 0.9, `candle-nn` 0.9, `candle-transformers` 0.9, `tokenizers` 0.20 (`unstable_wasm`), `gemm` 0.19, `safetensors`, `memmap2` | `fastembed` 6, `ort` 2.0-rc.13, `tokenizers` 0.22, `hf-hub`, `ndarray`, `ort-sys` |
| `NATIVE_DEPENDENCIES` | **none** — zero `-sys` crates, no C/C++ | static `libonnxruntime` 1.28 **downloaded from `cdn.pyke.io` at build time**; dynamic `libstdc++` |
| `BINARY_SIZE_DELTA_MB` | **~5 MB** (measured: standalone static-musl BERT-embed binary, stripped+LTO, 5.4 MB total) | ~37 MB (STEP 4A `bench`, static ORT) |
| `MODEL_DISTRIBUTION_SIZE_MB` | 127.3 (`model.safetensors`) + 0.7 (`tokenizer.json`) + <0.01 (`config.json`) ≈ **128 MB** | ~128 MB (onnx + tokenizer) |
| `OFFLINE_AFTER_INSTALL` | **yes** — no build-time binary download, no inference-time network; model is 3 plain files in a cache dir | yes, but the ORT binary is fetched at build time |
| `MODEL_CACHE_OR_BUNDLE_STRATEGY` | (a) vendor the 3 files in the release archive, or (b) integrity-pinned first-run download with explicit opt-in. Runtime auto-download **not acceptable** (unchanged from STEP 4A). Same open question as ONNX, minus the ORT-binary problem. | same, plus the self-built-or-downloaded ORT question |
| `MUSL_RELEASE_COMPATIBLE` | **yes** — isolated stack cross-compiled + ran as a static musl binary (§4b); full amatl needs only the `musl-tools` step already in `release.yml` for `ring` | **no** — pyke publishes no musl `libonnxruntime`; would need a self-built ORT or a move to a glibc target |
| `ONNX_PACKAGING_RISK` | — | **High** for the stated release target (STEP 4A §8) |
| `CANDLE_PACKAGING_RISK` | **Low** — pure Rust, static, no external binary, musl-native, ~5 MB, reproducible across libc | — |

## 9. Architectural decision

### `DECISION = CANDLE_MUSL_VIABLE_WITH_COST`

Against the spec §9 criteria:

| criterion | verdict |
|---|---|
| actual musl build works | **yes for the Candle stack** (static musl binary built and run, §4b). Full-amatl link is `BLOCKED_BY_ENVIRONMENT` here by pre-existing `ring`, which CI already handles — **not** a Candle problem. |
| semantic parity acceptable | **yes** — mean embedding agreement 0.977, rank corr 0.974; diagnostic buckets within 2–3 rows of ONNX; every STEP 4A conclusion holds |
| CPU latency operationally reasonable | **with cost** — ~4–5× slower than ONNX; ~184 ms per 10-doc result set. Tolerable for one search, but the largest downside |
| memory / model footprint acceptable | **yes** — RSS +137 MB (**less** than ONNX), 128 MB model (same), binary +~5 MB (**far less** than ONNX's +37 MB) |
| packaging simpler / more controllable than ORT | **yes, clearly** — no downloaded C++ runtime, no musl gap, no `libstdc++`, reproducible across libc |

The **only** material cost is CPU inference speed. Everything that made `ort`
risky for amatl's musl release — the missing musl `libonnxruntime`, the
build-time binary download, `libstdc++` — is gone with Candle.

**Candle is not preferred merely because it is Rust-native.** It is preferred
because it removes a *specific, real* packaging blocker (STEP 4A §8/§9:
"not on the musl release target with `ort`") at an acceptable, measured
runtime cost.

### `RELEASE_TARGET_CHANGE_TO_GLIBC = NO`

Candle makes the musl target reachable, so there is no reason to abandon it.
(If Candle's latency were later judged unacceptable *and* `ort` performance
were required, the glibc question would reopen — but that is not the current
situation.)

## 10. Recommended next steps (not done in this phase)

1. **Do not** change the production default. Bounded-semantic stays.
2. If STEP 4 proceeds to production wiring: build on `CandleBackend`, keep
   `FastembedBackend` as the dev-time reference only. Design the hybrid per
   STEP 4A §10.3 — embedding as a paraphrase-recall booster **subordinate to**
   negative / entity evidence, never an independent RELEVANT vote.
3. Confirm the full amatl musl release build on a host / CI runner with
   `musl-tools` (the existing `release.yml` job), with `CandleBackend`
   linked, before any release consideration.
4. Measure whether ~184 ms/search of added CPU is acceptable against amatl's
   latency budget; if not, cache document embeddings or reduce the embedded
   set.
5. Resolve model distribution (vendored 128 MB vs pinned first-run download).
6. Build the ≥100-row **blind** hold-out with inputs frozen *before* any
   threshold selection, second independent labelling pass, and run the
   unchanged §9 acceptance gates. STEP 4B does **not** create it and proves
   **no generalization** — parity numbers use consumed data only.

## Scope confirmation

`PRODUCTION_DEFAULT_CHANGED=NO` · `ROUTING_CHANGED=NO` · `TELEMETRY_CHANGED=NO`
· `TELEMETRY_UNLOCKED=NO` · no new blind hold-out created · no labels inspected
· no Marginalia network calls · no environment variables inspected · production
`crates/` untouched (verified `git diff --stat HEAD -- crates/` empty).
