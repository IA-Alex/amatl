# Software supply-chain policy

## Reproducibility and source admission

`Cargo.lock` is versioned and CI uses locked installations for security tools.
Runtime dependencies must come from crates.io; unknown registries and Git
sources are denied (`deny.toml`). A new dependency is admitted only when the pull
request documents purpose, maintenance/activity, transitive cost, license,
security history, feature minimization, rustls compatibility where relevant,
and why existing workspace code is insufficient. Wildcard versions are denied.

`cargo deny` allows Apache-2.0, Apache-2.0 with LLVM exception, BSD-3-Clause,
CDLA-Permissive-2.0, ISC, MIT, MPL-2.0, Unicode-3.0, and Zlib. Multiple versions
and yanked crates warn; unknown Git/registries and wildcard dependencies deny.
Warnings require reviewer judgment and must not be described as failures.

## Automated gate

Every push and pull request runs format, workspace tests, benchmark compilation,
Clippy with warnings denied, `cargo audit`, `cargo deny check`, and CycloneDX SBOM
generation (`.github/workflows/ci.yml`). An advisory reported by `cargo audit`
fails the job; it is not silently allowlisted in repository configuration.

Independently of the network-dependent audit, `cargo test --workspace` asserts a
floor for every **remediated** advisory whose fixed release the locked graph can
reach (`crates/amatl-core/tests/supply_chain_contract.rs`). The test reads the
workspace `Cargo.lock`, the same file CI installs with `--locked`, so it fails
offline on every host — including the macOS/Windows jobs that never run `cargo
audit`. Each entry records the advisory IDs and their fixed releases next to the
version, so the number is re-derived from the advisory text instead of trusted;
removing a crate from the graph requires deleting its entry deliberately. The
floor is a minimum, never a pin: a newer compatible release passes.

An advisory whose fix no compatible release reaches is **not** a floor: asserting
one would redden the suite on every host without remediating anything. It is
recorded as an open finding below, and the authoritative gate stays red until it
is remediated or waived by a written ADR. This offline test guards the advisories
already closed; it reads a lockfile, never the RustSec database, so it cannot
discover an advisory that no floor names yet. It is a regression guard, not an
equivalent of the audit.

Three advisories were remediated on 2026-10-25 and are now floors:
RUSTSEC-2026-0382 (`rustls-webpki` 0.103.16), RUSTSEC-2026-0381 (`webpki-roots`
1.0.10) and RUSTSEC-2026-0386 (`tokio` 1.53.3). The first two entered the RustSec
database on 2026-10-19 and 2026-10-20 — at or after the date the open list below
was written — and all three sat in the locked graph, the two TLS ones in the
primary HTTPS path, while this document still described a `zip`-only audit. The
floors are what keeps that gap from reopening silently.

The accountable security role is repository owner `@IA-Alex`. The response SLA
in `SECURITY.md` requires acknowledgement in 1/2/5/10 business days and targets
remediation in 14/30/60/90 calendar days for critical/high/medium/low findings,
respectively. A failing audit blocks merge immediately; an overdue remediation
must record its compensating control and revised date in a private advisory.

An exception requires a written ADR with advisory, affected path, exploitability,
compensating controls, owner, expiration date, and removal plan. `@IA-Alex` must
approve it explicitly; no exception is pre-approved.

## Open findings

`cargo audit` scans every package in `Cargo.lock`; `cargo deny check` resolves
the default-feature graph and classifies severity differently — `unsound` is an
error that fails the job, while `cargo audit` reports it as a warning. Neither
gate subsumes the other: dependencies that only the optional
`experimental-local-embeddings` feature pulls in are visible to the audit and
invisible to `cargo deny`, and the two disagree on severity, so both are read.
As of 2026-10-25 the workspace audit reports three open vulnerabilities in
`zip 7.2.0`, reached only through that feature (`amatl-core` →
`candle-core 0.9.2` → `zip`):

| Advisory | Severity | Fixed release |
|---|---|---|
| RUSTSEC-2026-0367 | 7.3 high | `zip >= 9.0.0` |
| RUSTSEC-2026-0368 | not rated | `zip >= 9.0.0` |
| RUSTSEC-2026-0369 | 9.1 critical | `zip >= 9.0.0` |

No compatible upgrade closes them. `zip` is a mandatory, non-optional dependency
of `candle-core` (`^7.2.0` in the locked 0.9.2, `^8.6.0` in the latest stable
0.11.0) and the advisories are patched only from `9.0.0`, so the whole candle
line is exposed. The affected code is the extraction helpers that apply symlink
targets, Unix permissions and SUID/SGID bits (`ZipArchive::extract`,
`ZipStreamReader::extract`); `candle-core` calls only `ZipArchive::new`,
`by_index`, `by_name` and `ZipWriter` (`src/npy.rs`), the feature is off by
default, and its only input is a local, operator-provided model package. That
bounds the impact but does not close the finding.

This is an open finding, not a waiver: no ignore entry exists in `deny.toml`,
and the workspace audit stays red until `@IA-Alex` decides between a written ADR
exception (with the fields listed above) and removing the experimental stack
from the workspace lockfile, the way `experiments/embedding-relevance` is
excluded from the workspace and keeps its own lockfile. Warnings are separate:
`paste 1.0.15` (unmaintained, same optional stack) and `chacha20 0.10.1`
(yanked, via `rand 0.10.2` → `rmcp 3.1.2`) are reviewer judgement under the
policy above; `cargo deny check` passes with both. The class decides, not the
entry: `unmaintained` and `yanked` warn, `unsound` is an error. `tokio 1.53.1`
failed `cargo deny check` that way until it was remediated to `1.53.3` on
2026-10-25.

## SBOM

CI runs `cargo cyclonedx` and uploads all generated `.cdx.json` and `.cdx.xml`
files as the `amatl-sbom` workflow artifact for 14 days. The workflow fails if
none exists.
Consumers should verify the workflow revision, download the artifact from the
corresponding trusted run, and correlate package URLs/versions with
`Cargo.lock`.

The release-candidate workflow builds a static Linux musl archive plus Debian,
RPM and Arch packages from the same binary, verifies its type and version,
includes four SBOMs, checks SHA-256 and retains the private CI artifact for 30
days. GitHub artifact attestation is skipped explicitly for this user-owned
private repository because the current plan does not support it. An annotated
matching tag is still required before the workflow creates a prerelease.

## Response procedure

1. Reproduce the finding against the locked graph and identify reachable code.
2. Prefer a compatible upgrade; otherwise remove/replace the dependency or
   minimize features.
3. Add a regression/security test where the vulnerable behavior is relevant.
4. Regenerate and compare the SBOM; run the complete contract gate.
5. Publish an advisory/release note if users could be affected.

## SLSA posture

SLSA is aspirational: CI is defined as code and emits an SBOM and reproducible
archive, but the project does not claim a SLSA build level, hermetic build,
provenance attestation, signed release, or isolated release builder. The private
repository limitation and absence of a tagged release prevent stronger claims.

References: [SLSA specification](https://slsa.dev/spec/) and
[CycloneDX](https://cyclonedx.org/).
