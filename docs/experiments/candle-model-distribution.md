# Candle model distribution — STEP 4C

**Date:** 2026-08-29
**Branch:** `fix/audit-repository-hygiene`
**Baseline HEAD:** `436445e7` (STEP 4B). Production relevance unchanged from
`66598c9`.
**Status:** analysis + decision. Nothing implemented in the production build;
the experimental seam (`amatl-core` feature `experimental-local-embeddings`)
carries no model and no download code.

Resolves spec §6 / §7: how the ~128 MB `bge-small-en-v1.5` model would reach
AMATL users **if** local embeddings were ever promoted out of the experiment.

## Artifacts to distribute (all hashes pinned)

| file | size (bytes) | sha256 |
|---|---|---|
| `model.safetensors` (fp32) | 133 466 304 | `3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad` |
| `tokenizer.json` | 711 396 | `d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66` |
| `config.json` | 743 | `094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750` |
| **total** | **≈ 128.1 MB** | |

Source: `BAAI/bge-small-en-v1.5` (MIT). Hashes measured on the files fetched by
the STEP 4A/4B experiment and are **identical** to the values recorded in
`candle-musl-feasibility.md` §3. All three must be pinned — a mismatched
tokenizer or config silently changes embeddings.

## AMATL release facts these options are judged against

* **Linux-first.** The primary release target is a **fully static
  `x86_64-unknown-linux-musl`** binary (`Cargo.toml`
  `[workspace.metadata.release]`), packaged as `.tar.gz` + `.deb` + `.rpm` +
  `.pkg.tar.zst` (`packaging/build-linux-packages.sh`), plus an aarch64
  archive. Current archive payload is just the binary + README + licences +
  4 CycloneDX SBOMs (`release.yml`), a few MB.
* **Reproducible.** `release.yml` builds with `SOURCE_DATE_EPOCH`, sorted tar,
  numeric owner; publishes `SHA256SUMS` and per-crate SBOMs. Any model
  distribution must not break byte-reproducibility of the archive.
* **Offline.** AMATL runs a search against remote *providers*; there is no
  AMATL-owned model service and no telemetry. A local embedding model must
  therefore be **fully offline after install** and must never trigger a
  network fetch during an ordinary search.
* **No formal latency SLO** (see `candle-e2e-4c.md` §5) — but a
  30 000 ms request deadline (`docs/api/openapi.yaml`).

## Option A — vendor the model inside the release artifact

Ship the 3 files in the `.tar.gz` / `.deb` / `.rpm` / `.pkg.tar.zst` payload
(e.g. `/usr/share/amatl/models/bge-small-en-v1.5/`).

| field | assessment |
|---|---|
| `OFFLINE_FIRST_RUN` | **yes** — model present the instant the package is installed; zero network ever |
| `RELEASE_SIZE_IMPACT` | **+128 MB per artifact**, ×4 Linux package formats + 2 arch archives ≈ the release grows from a few MB to ~130 MB per artifact, ~800 MB of release assets total. `.deb`/`.rpm` payload dominated by an ML blob |
| `REPRODUCIBILITY` | **good** — the blob is content-addressed; `tar --sort=name --mtime` already deterministic; SBOM/`SHA256SUMS` extend to it naturally |
| `USER_FRICTION` | **lowest** — nothing to do; `apt install amatl` just works offline |
| `SUPPLY_CHAIN_RISK` | **low, and fully auditable** — the blob is in the signed/checksummed release; no third party is contacted. Risk = trusting the one-time vendoring step, mitigated by the pinned hash in-repo |
| `PACKAGING_COMPLEXITY` | **moderate** — must add the blob to 6 artifact builds, teach `build-linux-packages.sh` to install it, keep it out of `git` (Git LFS or a release-time fetch-by-hash into the build tree), and the CI cache/bandwidth cost |
| `UPDATE_STRATEGY` | model version is tied to the AMATL release; a new model = a new AMATL point release. Simple, coarse |

## Option B — pinned first-run download with hash verification

Package ships **no** model. On first use of the experimental feature AMATL
downloads the 3 files from a pinned URL, verifies each sha256 against the
in-binary constant, writes them to a cache dir, and refuses to proceed on
mismatch.

| field | assessment |
|---|---|
| `OFFLINE_FIRST_RUN` | **no** — the first run that needs embeddings requires network. Offline forever after |
| `RELEASE_SIZE_IMPACT` | **none** — archive stays a few MB |
| `REPRODUCIBILITY` | **build** reproducible; **runtime state** is not part of the artifact. The downloaded blob is pinned by hash so it is deterministic *content*, but provenance now depends on an external host staying up and serving identical bytes |
| `USER_FRICTION` | **moderate** — a ~128 MB download on first use, needs connectivity, needs an explicit opt-in (spec: "no silent network download"). Air-gapped installs need a manual sideload path anyway |
| `SUPPLY_CHAIN_RISK` | **higher** — introduces a runtime dependency on an external download host (HF CDN or an AMATL-owned mirror). Hash-pinning defeats content tampering but not availability loss or a forced-downgrade/DoS. New TLS + HTTP surface in a tool that otherwise only talks to search providers |
| `PACKAGING_COMPLEXITY` | **low for packaging**, **non-trivial for code** — needs a downloader, cache management, atomic write, hash gate, opt-in config, and a documented offline sideload procedure |
| `UPDATE_STRATEGY` | change the pinned URL+hash in a point release; clients re-download lazily. Decouples model cadence from binary size |

## Option C — separate optional model package / artifact

Publish `amatl-model-bge-small` as its own `.deb`/`.rpm`/archive (or a
release asset) that installs the 3 files to the shared path. `amatl` itself
stays small; users who want local embeddings `apt install amatl-model-bge-small`.

| field | assessment |
|---|---|
| `OFFLINE_FIRST_RUN` | **yes, once the model package is installed** — same offline guarantee as A, gated on a second install step |
| `RELEASE_SIZE_IMPACT` | **+128 MB once**, as a clearly separate optional asset; the core `amatl` artifacts stay a few MB |
| `REPRODUCIBILITY` | **good** — the model package is itself checksummed/SBOM'd like any release artifact; version pinned by package version + hash |
| `USER_FRICTION` | **low–moderate** — one extra `apt install`, discoverable via a `Recommends:`/`Suggests:` relationship or a clear error message ("install `amatl-model-bge-small` to enable local embeddings"). No network beyond the normal package repo |
| `SUPPLY_CHAIN_RISK` | **low** — same trust model as the main package; distributed through the same signed channel; no new runtime network surface |
| `PACKAGING_COMPLEXITY` | **moderate** — a second package definition and release job, a shared install path contract, and a feature-detects-model check in `amatl`. But it is *packaging*, done once, not runtime code |
| `UPDATE_STRATEGY` | bump the model package independently; `amatl` declares a version range it accepts. Clean separation of cadences |

## Requirements check (spec §6)

| requirement | how it is met (under the chosen option) |
|---|---|
| model hash pinned | sha256 constants compiled into `amatl-core` (the experimental module), checked on load |
| tokenizer / config hashes pinned | same — all three constants, all three checked |
| inference works fully offline after install | Options A and C: model on disk at install time. Candle links no external runtime (`candle-musl-feasibility.md` §4b) so there is nothing else to fetch |
| no silent network download during ordinary search | no download path in the shipped binary at all (A/C). The seam returns `EmbeddingUnavailable::ModelMissing` and search continues on bounded-semantic |
| model corruption fails closed to bounded-semantic | `evaluate_with_embeddings` maps `ModelCorrupt` / `HashMismatch` → `SemanticRescore::fallback`; `experimental_embeddings` tests + `experimental_embedding_latency::seam_failure_modes_fall_back_to_bounded_semantic` prove it |
| model path / version explicit | a single configured path + a `MODEL_VERSION` constant; no HF-cache auto-discovery in the production seam |

No remote fallback inference is proposed or implemented.

## Decision — spec §7

### `MODEL_DISTRIBUTION_DECISION = MODEL_DISTRIBUTION_OPTIONAL_PACKAGE`

**Option C.** Rationale against AMATL's stated priorities:

* **Linux-first distribution** — AMATL already builds 4 native Linux package
  formats through `packaging/build-linux-packages.sh`. Adding one more
  package (`amatl-model-bge-small`) fits that machinery exactly; a
  `Suggests:`/`Recommends:` line makes it discoverable without forcing it.
* **Reproducibility** — the model ships as a normal checksummed, SBOM'd
  release artifact through the existing signed channel. No runtime download
  host to trust, unlike B.
* **Offline behaviour** — identical guarantee to vendoring (A): once
  installed, zero network, ever. Corruption/absence fails closed to
  bounded-semantic.
* **Package size** — the core `amatl` artifacts stay a few MB. A user who
  never enables the experimental feature never pays 128 MB. Vendoring (A)
  would nearly 30× every Linux artifact for a feature that is off by default.
* **Operational simplicity** — the extra work is a one-time packaging job,
  not new runtime networking code in a tool whose security model is built on
  "only talks to configured search providers". Option B's downloader is the
  opposite of simple here.

Vendoring (A) becomes the better choice only if local embeddings are ever made
the **default**; at that point the "don't pay for what you don't use" argument
disappears and one-install-step simplicity wins. That is a STEP 4D+ decision,
not this one.

`MODEL_DISTRIBUTION_UNRESOLVED` is explicitly **not** the outcome.
