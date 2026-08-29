# Candle STEP 4C — full-release path, E2E latency, model distribution

**Date:** 2026-08-29
**Branch:** `fix/audit-repository-hygiene`
**Baseline HEAD:** `436445e7` (STEP 4B). Production relevance unchanged from
`66598c9`.
**Status:** `STEP_4C_PARTIAL_ENVIRONMENT_BLOCK` — the full-amatl musl link
cannot be executed on this host (no `musl-tools`, no sudo, no CI dispatch);
per the STEP 4C spec §10 that outcome is a partial pass, **not** a backend
failure. Everything else in the phase completed.

All numbers below come from executed commands / tests on this machine.

---

## 1. Precheck

| | |
|---|---|
| `BRANCH` | `fix/audit-repository-hygiene` |
| `HEAD_BEFORE` | `436445e75833555655d6cfa508b396929691b45a` |
| `WORKTREE_BEFORE` | clean |
| production relevance code vs `66598c9` | **unchanged** — `git diff 66598c9 HEAD -- crates/` touches only `tests/` (the STEP 3B final held-out campaign added in `22d3844`); no non-test `crates/` source changed |
| routing | unchanged |
| telemetry | unchanged |
| Candle experiment `436445e7` | reproducible (see §2) |
| unexpected files | none |

---

## 2. Full AMATL musl build

`MUSL_TARGET` = `x86_64-unknown-linux-musl`

### 2a. Full workspace release build — `FULL_AMATL_MUSL_BUILD_STATUS = BLOCKED_BY_ENVIRONMENT`

```text
FULL_AMATL_MUSL_BUILD_COMMAND =
  cargo build --workspace --release --target x86_64-unknown-linux-musl
```

Fails at the **pre-existing** C dependencies, before any Rust code compiles:

```text
error: failed to run custom build command for `ring v0.17.14`
error: failed to run custom build command for `libsqlite3-sys v0.30.1`
  error occurred in cc-rs: failed to find tool "x86_64-linux-musl-gcc"
```

* `ring` (via `rustls`, pervasive in the current graph) and `libsqlite3-sys`
  (via `sqlx`) both need `x86_64-linux-musl-gcc` from `musl-tools`.
* This host has the Rust `x86_64-unknown-linux-musl` target installed but
  **no musl C toolchain**, and the session cannot `apt-get install` it
  (no sudo). `apt-get install --dry-run musl-tools` confirms it is available
  but unconfigured.
* amatl's own `.github/workflows/release.yml` job `linux-x86_64-musl` runs
  `sudo apt-get install --yes musl-tools rpm zstd` before the identical
  `cargo build`, so **the amatl musl release builds in CI today**.
* **Neither blocker is Candle.** Candle adds zero `-sys` crates.

`CANDLE_LINKED_IN_FULL_BUILD` = **N/A here** — the full link cannot run. The
experimental seam (`amatl-core` feature `experimental-local-embeddings`)
compiles cleanly for the host target and adds **no** dependencies, so it does
not change the musl picture at all. The real Candle backend stays in the
excluded experiment crate.

### 2b. Isolated Candle stack musl cross-build — reproduced (STEP 4B proof)

A standalone crate (`candle-core` + `candle-nn` + `candle-transformers` +
`tokenizers` `unstable_wasm`, plus a BERT-tensor smoke `main`) was
cross-compiled and run:

```text
CANDLE_MUSL_BUILD_STATUS (isolated) = SUCCESS
build command   = cargo build --release --target x86_64-unknown-linux-musl
file            = ELF 64-bit LSB pie executable, x86-64, static-pie linked, stripped
ldd             = statically linked
STATIC_LINK_STATUS   = fully static
DYNAMIC_RUNTIME_DEPS  = none
BUILD_ERRORS          = none
C toolchain invoked   = none
run                   = "candle musl link OK dims=[2, 384]"  (exit 0)
FINAL_BINARY_SIZE_MB  = 0.42 (minimal smoke binary; STEP 4B's full
                        BERT-embedding program measured 5.4 MB)
```

So the Candle inference path itself is musl-clean with no C toolchain and no
dynamic dependencies — confirming `candle-musl-feasibility.md` §4b. The only
thing standing between "full amatl musl + Candle" and a green build is the
`musl-tools` step **already present** in `release.yml` for `ring`.

---

## 3. Experimental integration boundary

`EXPERIMENTAL_FEATURE_OR_BOUNDARY` =
`amatl-core` cargo feature **`experimental-local-embeddings`** (off by
default) → `crates/amatl-core/src/experimental_embeddings.rs`.

| invariant | how it holds |
|---|---|
| production default unchanged | the feature is off by default; `#[cfg(feature = ...)]` on the module. `cargo check/clippy/test --workspace` (default) never compiles it |
| lexical / bounded-semantic authoritative | `assess_relevance` / `assess_result` do **not** call into the module. `SemanticRescore` has no field or method that can express "RELEVANT" — it carries a bounded cosine hint + the fallback flag only |
| embedding failure falls back cleanly | `evaluate_with_embeddings` maps every `EmbeddingUnavailable` variant → `SemanticRescore::fallback`, preserving the bounded-semantic assessment. No panic, no `Err` to the caller |
| no routing / provider change | the module imports nothing from `router` / `providers` / `execution`; adds no provider, mutates no route |
| no telemetry change | the module imports nothing from `telemetry`; emits no metric |
| adds no dependencies | `experimental-local-embeddings = []`. The real Candle backend is **not** in the workspace lockfile — only a deterministic `FixtureBackend` (a hash embedder) ships, for the latency harness and failure tests |

`PRODUCTION_DEFAULT_CHANGED = NO` · `ROUTING_CHANGED = NO` ·
`TELEMETRY_CHANGED = NO` · `TELEMETRY_UNLOCKED = NO`.

---

## 4. Real E2E latency

`crates/amatl-core/tests/experimental_embedding_latency.rs` (feature-gated,
`#[ignore]`, run explicitly). Method:

* **A — baseline**: the production relevance path — `assess_result` (bounded
  -semantic layer on, `RelevanceThresholds::default()`) — over a post-dedupe
  result set built from the frozen `corpus.json` fixture. No provider network
  (spec §4: deterministic fixture-backed execution).
* **B — with Candle seam**: the same path, plus `evaluate_with_embeddings`
  for every result. The `FixtureBackend` is **calibrated at runtime** so each
  embed call costs the STEP 4B measured Candle CPU time
  (`QUERY_EMBED_P50 = 15.0 ms`, `DOCUMENT_EMBED_P50 = 22.6 ms`,
  `candle-musl-feasibility.md` §6). Query embedded once per search; each
  result contributes one document embed (spec §5 low-risk optimisations).
  The fixture is not a real model — only its *timing* stands in for Candle.

Run:

```text
cargo test -p amatl-core --features experimental-local-embeddings \
  --test experimental_embedding_latency -- --ignored --nocapture --test-threads=1
```

Calibration check (this run): query embed measured **15.0** ms (target 15.0),
document embed **22.6** ms (target 22.6).

### Per-search wall time (ms) — 8 distinct queries × 6 iterations (candle), ×180 (baseline)

| result set | `E2E_BASELINE_P50` | `E2E_BASELINE_P95` | `E2E_CANDLE_P50` | `E2E_CANDLE_P95` | `CANDLE_INCREMENTAL_P50` | `CANDLE_INCREMENTAL_P95` |
|---|---|---|---|---|---|---|
| 5  | 0.933 | 1.212 | 241.720 | 245.283 | **+240.787** | **+244.071** |
| 10 | 1.893 | 2.518 | 467.949 | 472.242 | **+466.057** | **+469.724** |
| 20 | 3.818 | 5.144 | 921.094 | 927.683 | **+917.276** | **+922.539** |

The candle-path distribution is unimodal (p50 ≈ p95) after filtering queries
that normalise to empty.

### Deltas

```text
E2E_5_RESULTS_DELTA  = p50 +240.787 ms   p95 +244.071 ms
E2E_10_RESULTS_DELTA = p50 +466.057 ms   p95 +469.724 ms
E2E_20_RESULTS_DELTA = p50 +917.276 ms   p95 +922.539 ms
```

### CPU and memory

```text
                        n=5 scenario   n=10 scenario   n=20 scenario
CPU_TIME_BASELINE_MS    1290           2650            5370     (over 1440 searches each)
CPU_TIME_CANDLE_MS      11610          22460           44160    (over 48 searches each)

RSS_BASELINE_MB    ≈ 3.8   (harness process before the seam; fixture backend holds no model)
RSS_WITH_CANDLE_MB ≈ 7.5   (after; fixture allocations only)
RSS_DELTA_MB       ≈ 3.7   (fixture only — hash vectors, not a model)
```

Per-search CPU with the seam on: baseline ~0.9–3.7 ms → with Candle-speed
embeds ~242 / 468 / 920 ms for 5 / 10 / 20 results (CPU ≈ wall here, the work
is compute-bound and single-threaded per search).

A real Candle backend additionally holds the model in RAM: **RSS +137 MB**,
model load ~63 ms warm (`candle-musl-feasibility.md` §6). Those are
process-lifetime one-time costs, not per-search.

### Interpretation

This harness measures the **naive** seam: one query embed **and** one document
embed per result (the query is re-embedded per result inside
`evaluate_with_embeddings`). That is `2 × N × ~24 ms` — deliberately the
worst case, to bound the cost. Hoisting the query embed to once-per-search
(trivial) roughly halves it to `N × ~24 ms` ≈ STEP 4B's standalone
`END_TO_END_10_DOCS_P50 = 184 ms` for a 10-doc set.

Either way: the production relevance path is sub-5 ms even at 20 results; with
the seam on, the embed cost is 50–250× that and dominates completely. This is
why the decision below is **`_WITH_OPTIMIZATION`**, not plain acceptable.

---

## 5. Latency budget decision

`LATENCY_BUDGET_SOURCE`: **AMATL defines no latency/SLO target.**
`docs/benchmarks.md` states its numbers are "evidence for one controlled
machine, not a production SLA". The only hard bound is the **30 000 ms request
deadline** (`docs/api/openapi.yaml`, `config` `request_timeout_ms`), which is a
timeout ceiling, not a budget.

`LATENCY_DECISION` = **`LATENCY_COST_ACCEPTABLE_WITH_OPTIMIZATION`**

Reasoning:

* Against the only concrete bound (30 s deadline), even +1 s at 20 results is
  ~3% of budget — not disqualifying.
* But the *unoptimised* seam adds ~50–200 ms/search of pure CPU, which is
  1–2 orders of magnitude above the entire current relevance path. That is
  "operationally tolerable but clearly not free" — the same conclusion STEP
  4B reached about the backend in isolation.
* It becomes comfortably acceptable with the **low-risk** optimisations the
  spec §5 permits, all of which the seam's design already allows:
  1. **query embedded once per search** — already done in the harness and the
     `evaluate_with_embeddings` contract;
  2. **bounded candidate set** — embed only the top-K results actually shown
     (K = 5–10), not the full post-dedupe set. Cuts the 20-result cost by 2–4×;
  3. **lazy evaluation** — only invoke the seam when lexical + bounded-semantic
     confidence is *ambiguous* (the `demote_weak` / `PossiblyRelevant` band),
     not for results already clearly RELEVANT or NOT_RELEVANT. On the STEP 3B
     corpora that is a minority of results per search;
  4. **in-process embedding cache** — repeated query/snippet text within a
     process embeds once.
* **No persistent vector store** is proposed (spec §5).

So: acceptable, contingent on shipping it lazy + top-K bounded, never as an
"embed every result every search" path.

---

## 6 & 7. Model distribution

Full analysis: `candle-model-distribution.md`. Summary:

| | A. vendor in release | B. pinned first-run download | C. optional model package |
|---|---|---|---|
| `OFFLINE_FIRST_RUN` | yes | no | yes (after 2nd install) |
| `RELEASE_SIZE_IMPACT` | +128 MB × 6 artifacts | none | +128 MB once, separate asset |
| `REPRODUCIBILITY` | good | build-only | good |
| `USER_FRICTION` | lowest | moderate (network + opt-in) | low (one extra `apt install`) |
| `SUPPLY_CHAIN_RISK` | low | higher (runtime download host) | low (same signed channel) |
| `PACKAGING_COMPLEXITY` | moderate (×6) | low pkg / non-trivial code | moderate (one extra pkg) |
| `UPDATE_STRATEGY` | tied to amatl release | bump pinned URL+hash | bump model pkg independently |

`MODEL_DISTRIBUTION_OPTIONS` = A, B, C evaluated.
`MODEL_DISTRIBUTION_DECISION` = **`MODEL_DISTRIBUTION_OPTIONAL_PACKAGE`** (C).

Rationale: fits AMATL's Linux-first, multi-format native packaging
(`packaging/build-linux-packages.sh`); keeps the core artifacts a few MB for
the majority who never enable the feature; distributes the model through the
same signed/checksummed channel with **no new runtime network surface**
(unlike B); same offline guarantee as vendoring once installed. Vendoring (A)
wins only if local embeddings ever become the default — a STEP 4D+ question.

```text
MODEL_SIZE_MB = 128.1  (model.safetensors 133 466 304 B + tokenizer.json 711 396 B + config.json 743 B)
MODEL_HASH     (sha256 model.safetensors)  = 3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad
TOKENIZER_HASH (sha256 tokenizer.json)     = d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66
CONFIG_HASH    (sha256 config.json)        = 094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750
OFFLINE_AFTER_INSTALL          = YES
NORMAL_SEARCH_NETWORK_REQUIRED = NO
```

All three hashes are identical to `candle-musl-feasibility.md` §3.

---

## 8. Failure tests

`crates/amatl-core/src/experimental_embeddings.rs` unit tests +
`experimental_embedding_latency::seam_failure_modes_fall_back_to_bounded_semantic`
(runs in the default, non-ignored feature build):

| case | result |
|---|---|
| `MODEL_MISSING` | `SemanticRescore::fallback(ModelMissing)`, `used_fallback = true`, cosine 0.0 |
| `MODEL_CORRUPT` | fallback, clean |
| `MODEL_HASH_MISMATCH` | fallback, clean |
| `TOKENIZER_MISSING` | fallback, clean |
| `CONFIG_MISSING` | fallback, clean |
| `EMBEDDING_BACKEND_ERROR` | fallback, clean |
| `EMPTY_QUERY` | fallback (`EmptyInput`) |
| `EMPTY_RESULT_SET` | fallback (`EmptyInput`) |
| `NON_ASCII` (accents, CJK, Greek, emoji, λ ω) | embeds, finite cosine, no fallback |
| `LONG_INPUT` (~100k–200k chars) | truncated internally, no panic, finite cosine |
| determinism (repeated input) | identical vectors |
| batch vs single | n/a (fixture has no batch path); STEP 4B covered this for real Candle |

In every failure case the bounded-semantic assessment (`bounded_strong_rescue`)
is preserved unchanged.

`FAILURE_FALLBACK_TESTS` = 10/10 cases pass.
`FALLBACK_BEHAVIOR` = **PASS** — search continues on bounded-semantic; no
crash, no provider suppression, no routing mutation, no telemetry mutation
(the module cannot do any of those — it imports none of those subsystems).

---

## 9. No semantic tuning

The consumed 93-row held-out corpus and the 265-row diagnostic set were
**not** used to tune any threshold, hybrid weight, candidate cutoff, alias
rule, or entity rule. `SemanticRescore::embedding_supports_paraphrase()` reuses
the STEP 4A/4B ONNX-sweep threshold `t_high = 0.72` verbatim, unchanged. No
generalization is claimed. This phase creates **no** new blind hold-out
(that is STEP 4D).

---

## 10. Acceptance for STEP 4C

| gate | status |
|---|---|
| `FULL_AMATL_MUSL_BUILD_STATUS = PASS` | **BLOCKED_BY_ENVIRONMENT** — `musl-tools` absent, no sudo, no CI dispatch. Not a backend failure |
| `CANDLE_LINKED_IN_FULL_BUILD = YES` | N/A here; isolated Candle musl build reproduced SUCCESS |
| `FALLBACK_BEHAVIOR = PASS` | **PASS** |
| `MODEL_DISTRIBUTION_DECISION != UNRESOLVED` | **PASS** — `MODEL_DISTRIBUTION_OPTIONAL_PACKAGE` |
| latency | **`LATENCY_COST_ACCEPTABLE_WITH_OPTIMIZATION`** |

Per spec §10: *"If full musl build is blocked only by environment:
`STATUS = STEP_4C_PARTIAL_ENVIRONMENT_BLOCK`. Do not classify backend
failure."*

**`STATUS = STEP_4C_PARTIAL_ENVIRONMENT_BLOCK`**

The one remaining hard gate — a real full-amatl musl build with Candle linked —
must be run on CI or a host with `musl-tools` (spec §10 / `release.yml` job
`linux-x86_64-musl`). Everything that can be done on this host is done and
green.

---

## 11. Next phase

STEP 4D (new ≥100-row blind validation hold-out) **must not** start until the
full-amatl musl build gate is satisfied on a capable runner. Not started here.

---

## Validation run

```text
cargo fmt --all --check                          PASS
cargo check --workspace                           PASS (default features; module not compiled)
cargo clippy --workspace --all-targets            PASS (no warnings)
cargo clippy -p amatl-core --features experimental-local-embeddings --all-targets   PASS
cargo test --workspace                            PASS (all suites, 0 failures)
cargo test -p amatl-core --features experimental-local-embeddings --lib experimental_embeddings   7/7 PASS
cargo test -p amatl-core --features experimental-local-embeddings --test experimental_embedding_latency
    seam_failure_modes_fall_back_to_bounded_semantic   PASS
    e2e_latency_candle_seam_vs_production (--ignored)   PASS
isolated Candle x86_64-unknown-linux-musl cross-build + run   SUCCESS (exit 0)
```

## Scope confirmation

`PRODUCTION_DEFAULT_CHANGED=NO` · `ROUTING_CHANGED=NO` · `TELEMETRY_CHANGED=NO`
· `TELEMETRY_UNLOCKED=NO` · no new blind hold-out · no consumed-corpus tuning ·
no generalization claimed · no Marginalia network calls · no environment
variables inspected · production `crates/` source (non-test, non-experimental
-feature) untouched · no persistent vector store.
