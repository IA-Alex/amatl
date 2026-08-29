# amatl — local embedding relevance experiment (STEP 4A + 4B)

**Status: feasibility experiment. Not production code. Not wired into
`amatl-core`.**

This crate is deliberately **outside** the amatl workspace (`exclude` in the
root `Cargo.toml`). `cargo {check,clippy,test} --workspace` and every
production build never see it. It carries heavy ML dependencies
(`fastembed` → `ort` → a downloaded static ONNX Runtime; **and** the STEP 4B
`candle` pure-Rust stack) and downloads an embedding model **at development
time only**.

## STEP 4B — Candle (pure-Rust, musl) backend

`src/candle_backend.rs` — `CandleBackend`, same `EmbeddingBackend` trait,
same model (`BAAI/bge-small-en-v1.5`, loaded as fp32 `model.safetensors`),
CLS pooling + L2 normalize, query instruction prefix. Pure Rust: **zero
`-sys` crates, no ONNX Runtime, no C++**. The tokenizer uses the
`unstable_wasm` (fancy-regex) feature so there is no `onig_sys` C dependency
— which is what lets the whole stack cross-compile to
`x86_64-unknown-linux-musl` with only `musl-tools` (already installed by
amatl's `release.yml` for `ring`).

- `src/bin/candle_bench.rs` — Candle latency / RSS / batch, same methodology
  as `bench.rs`.
- `src/bin/agreement.rs` — ONNX vs Candle embedding cosine agreement and
  Spearman rank correlation on the consumed diagnostic set. **Links both
  backends** → needs the large code model (see `.cargo/config.toml`); a real
  build would ship exactly one backend and would not.
- `src/bin/candle_diagnostic.rs` — the A/B/C/D diagnostic with Candle
  similarities, thresholds **reused** from the ONNX sweep (no re-tuning).
- `tests/candle_failure_modes.rs` — §5 safety: empty / non-ascii / long /
  determinism / batch-vs-single / corrupt-model → clean `Err`.

```sh
cargo run  --release --bin candle_bench        # downloads safetensors once
cargo run  --release --bin agreement
cargo run  --release --bin candle_diagnostic -- 0.72 0.55
cargo test --release --test candle_failure_modes -- --ignored
```

Full measured numbers, the musl cross-build proof, and the decision:
`../../docs/experiments/candle-musl-feasibility.md`.

## What it does

Measures whether a small, fully-local sentence-embedding model can address the
paraphrase / synonymy / conceptual-equivalence failure mode identified in the
STEP 3B hold-out campaign (RELEVANT recall 0.356, ~68% of errors being
zero/low-overlap paraphrase).

- `EmbeddingBackend` trait — the experimental boundary. No coupling to
  providers, routing, RRF, ranking or telemetry.
- `FastembedBackend` — real inference via `fastembed` 6 / `ort` 2.0-rc.
  Models: `BAAI/bge-small-en-v1.5` (default) and
  `sentence-transformers/all-MiniLM-L6-v2`.
- `src/bin/bench.rs` — real latency / RSS / batch measurements.
- `src/bin/diagnostic.rs` — A/B/C/D mode comparison on the **consumed**
  labeled corpora (`corpus.json`, `holdout.json`), with per-failure-mode
  buckets. Diagnostic only — no generalization claim, not a final gate.
- `tests/failure_modes.rs`, `tests/model_missing.rs` — spec §7/§8 safety:
  empty/`non-ascii`/long input, determinism, missing/corrupt model → clean
  `Err`, never a panic.

## Running

```sh
cd experiments/embedding-relevance

# one-time: downloads ORT (~), then the model into ./.fastembed_cache
cargo run --release --bin bench                 # bge-small-en-v1.5
cargo run --release --bin bench -- minilm       # all-MiniLM-L6-v2

cargo run --release --bin diagnostic            # A/B/C/D on consumed corpora
cargo run --release --bin diagnostic -- minilm

cargo test  --release -- --include-ignored      # safety / fallback tests
```

Offline after the first run: `ort` links a **static** ONNX Runtime at build
time and the model lives in `./.fastembed_cache`. No network at inference.

## Results

See `../../docs/experiments/local-embedding-feasibility.md` for the full
measured numbers, the packaging analysis (ONNX Runtime vs the amatl musl
release target), and the decision.
