# amatl — local-embedding integration boundary (STEP 4C seam + STEP 4D Candle)

**Status: experimental, quarantined. Not production code. Not part of the amatl
workspace.**

This crate is the former `amatl-core` module
`crates/amatl-core/src/experimental_embeddings/`, moved here verbatim on
2026-10-25 and listed in the root `Cargo.toml`'s `exclude` next to
`experiments/embedding-relevance`. No production build, no
`cargo {check,clippy,test} --workspace` run and no workspace `Cargo.lock` ever
sees it.

## Why it was quarantined

`candle-core 0.9.2` — the pinned STEP 4B feasibility version — depends on
`zip ^7.2.0`, and `zip < 9.0.0` carries three RustSec advisories:

| Advisory | Severity | Fixed release |
|---|---|---|
| RUSTSEC-2026-0367 | 7.3 high | `zip >= 9.0.0` |
| RUSTSEC-2026-0368 | not rated | `zip >= 9.0.0` |
| RUSTSEC-2026-0369 | 9.1 critical | `zip >= 9.0.0` |

No compatible upgrade reaches the fix from the candle line (`candle-core 0.9.2`
requires `^7.2.0`; the latest stable `0.11.0` requires `^8.6.0`). While the
Candle stack sat in `amatl-core` behind the optional
`experimental-local-embeddings` feature, `zip` was resolved into the workspace
`Cargo.lock`, so the workspace `cargo audit` step of the contract gate could not
pass — even though the feature was off by default and the module had no
production caller.

Keeping the stack in this excluded crate, with its own lockfile, closes the
finding **by exclusion instead of by waiver**: no `ignore` entry was added to
`deny.toml`, no exception was granted, and the production dependency graph is
unchanged. Full rationale and the removal plan:
`../../docs/security/supply-chain.md`.

**Re-admit the stack to the workspace** (and its dependencies back into
`amatl-core`) once a candle release depends on `zip >= 9.0.0`.

## What is here

| file | role |
|---|---|
| `src/lib.rs` | the STEP 4C typed seam: `EmbeddingBackend`, `SemanticRescore`, `evaluate_with_embeddings`, `cosine_similarity`, the deterministic `FixtureBackend` |
| `src/candle.rs` | real `CandleBackend` — BERT (`BAAI/bge-small-en-v1.5`), CLS pooling + L2, batched `embed_documents`, loads from a validated `ModelPackage` (no download path), fails with `EmbeddingUnavailable` |
| `src/model_config.rs` | `ModelPackage` — `MODEL_PATH` / `MODEL_VERSION` plus the pinned sha256 of `model.safetensors` / `tokenizer.json` / `config.json`. Purely local, fails closed |
| `src/pipeline.rs` | `SemanticEvaluator` — the optimized bounded-K execution architecture |
| `src/text.rs` | the casefold/NFKC helper the fixture embedder needs, copied from `amatl-core`'s private `text::normalized_text` (the production API is deliberately not widened for an experiment) |

Tests (all five moved with the code, no longer behind a cargo feature):

- `tests/experimental_candle_integration.rs` — STEP 4D integration + fallback
  matrix. Model-dependent cases are `#[ignore]` and keyed on
  `AMATL_TEST_MODEL_DIR`.
- `tests/experimental_embedding_latency.rs` — STEP 4C end-to-end search-path
  latency with vs without the seam, using the calibrated `FixtureBackend`.
- `tests/experimental_candle_latency.rs` — STEP 4D §6 real forward-pass latency.
- `tests/step4e_evaluation.rs` — the frozen blind STEP4E prediction harness
  (`ARM_A` production path vs `ARM_B` + Candle advisory).
- `tests/step4e_model_package.rs` — infrastructure-only package validation.

`tests/fixtures/relevance/corpus.json` is a copy of the frozen workspace fixture
the latency harness measures against, so this crate stays self-contained.

## Running

```sh
cd experiments/local-embeddings-boundary

cargo check --all-targets          # first run compiles the Candle stack
cargo test                         # fixture-backed tests, no model needed

# tests that need the real ~128 MB model package
AMATL_TEST_MODEL_DIR=/path/to/pkg cargo test --release \
  --test experimental_candle_latency -- --ignored --nocapture --test-threads=1
AMATL_TEST_MODEL_DIR=/path/to/pkg cargo test --release \
  --test experimental_candle_integration -- --ignored --nocapture --test-threads=1
```

No network at inference: the backend reads a local, operator-provided model
package and never downloads. A missing or corrupt package degrades to the
bounded-semantic outcome, never to an error surfaced to a caller.

Full measurements and decisions: `../../docs/experiments/candle-integration-4d.md`,
`../../docs/experiments/candle-e2e-4c.md` and
`../../docs/experiments/candle-model-distribution.md` (all three carry a
quarantine banner pointing here).
