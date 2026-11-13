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
recorded below and the authoritative gate stays red until it is remediated,
removed from the graph, or waived by a written ADR. This offline test guards the
advisories already closed; it reads a lockfile, never the RustSec database, so it
cannot discover an advisory that no floor names yet. It is a regression guard,
not an equivalent of the audit.

Three advisories were remediated on 2026-10-25 and are now floors:
RUSTSEC-2026-0382 (`rustls-webpki` 0.103.16), RUSTSEC-2026-0381 (`webpki-roots`
1.0.10) and RUSTSEC-2026-0386 (`tokio` 1.53.3). The first two entered the RustSec
database on 2026-10-19 and 2026-10-20 — at or after the date the open list below
was written — and all three sat in the locked graph, the two TLS ones in the
primary HTTPS path, while this document still described a `zip`-only audit. The
floors are what keeps that gap from reopening silently.

### Re-audit 2026-11-13

A re-run of the gate on 2026-11-13 found five advisories that had entered the
locked graph after the 2026-10-26 measurement: `rmcp 3.1.2` (RUSTSEC-2026-0457,
-0461, -0462), `rustix 1.1.4` (RUSTSEC-2026-0486, `unsound`), `quinn-udp 0.5.15`
(RUSTSEC-2026-0538) and `libsqlite3-sys 0.30.1` (RUSTSEC-2026-0459). Four were
remediated the same day by compatible `cargo update` and are now floors:
`rmcp` 3.10.0, `rustix` 1.1.6 and `quinn-udp` 0.5.16.

`libsqlite3-sys` RUSTSEC-2026-0459 remains **open and not remediable by a
compatible upgrade**. The advisory (patched in `>=0.39.0`) applies only to the
`bundled` SQLite source, which this workspace does enable (`sqlx`'s `sqlite`
feature → `sqlx-sqlite`'s `bundled` → `libsqlite3-sys`'s `bundled`). No released
`sqlx` reaches the fix: `sqlx 0.8.6` caps it at `^0.30` and `sqlx 0.9.0` caps it
at `>=0.30.1, <0.38.0`. The attack is local (`AV:L`, `UI:P`): it needs a less
trusted party to place a crafted database file and its `-wal` sibling where the
operator opens them, and the AMATL database path is operator-controlled and
local, which bounds the exposure. It is therefore **not** a floor. The
authoritative `cargo audit` and `cargo deny check` gates stay **red** until it is
remediated (a future `sqlx` release admitting `libsqlite3-sys >= 0.39`), removed
from the graph, or waived by a written ADR approved by `@IA-Alex`. Adding an
`ignore` entry without that ADR is not permitted by the policy above.

**Exploitability: low — no remote surface.** The advisory is explicit that only a
process which opens a database file, together with its `-wal`, that a less
trusted party could have written is exposed. In AMATL the SQLite file *is* the
local store the user owns and points at: there is no server that accepts a
database path, no endpoint that ingests a `.sqlite3`/`-wal` pair, and no remote
attacker path to those bytes — an adversary who can already write beside the
operator's database file has local write access to the host. The finding is
therefore **bounded local debt, not an open production risk**: the red gate is
read as "a pinned dependency awaits an upstream fix", and it must not be
paraphrased as a remotely reachable vulnerability. This is the exploitability
statement the eventual ADR (task below) rests on; it does not, by itself, waive
the finding.

The accountable security role is repository owner `@IA-Alex`. The response SLA
in `SECURITY.md` requires acknowledgement in 1/2/5/10 business days and targets
remediation in 14/30/60/90 calendar days for critical/high/medium/low findings,
respectively. A failing audit blocks merge immediately; an overdue remediation
must record its compensating control and revised date in a private advisory.

An exception requires a written ADR with advisory, affected path, exploitability,
compensating controls, owner, expiration date, and removal plan. `@IA-Alex` must
approve it explicitly; no exception is pre-approved.

## Findings closed by exclusion

`cargo audit` scans every package in `Cargo.lock`; `cargo deny check` resolves
the default-feature graph and classifies severity differently — `unsound` is an
error that fails the job, while `cargo audit` reports it as a warning. Neither
gate subsumes the other: a dependency reached only through an optional feature is
visible to the audit and invisible to `cargo deny`, and the two disagree on
severity, so both are read.

On 2026-10-25 the workspace audit reported three open vulnerabilities in
`zip 7.2.0`, reached only through the optional `experimental-local-embeddings`
stack (`amatl-core` → `candle-core 0.9.2` → `zip`):

| Advisory | Severity | Fixed release |
|---|---|---|
| RUSTSEC-2026-0367 | 7.3 high | `zip >= 9.0.0` |
| RUSTSEC-2026-0368 | not rated | `zip >= 9.0.0` |
| RUSTSEC-2026-0369 | 9.1 critical | `zip >= 9.0.0` |

No compatible upgrade closed them. `zip` is a mandatory, non-optional dependency
of `candle-core` (`^7.2.0` in the locked 0.9.2, `^8.6.0` in the latest stable
0.11.0) and the advisories are patched only from `9.0.0`, so the whole candle
line was exposed. The affected code is the extraction helpers that apply symlink
targets, Unix permissions and SUID/SGID bits (`ZipArchive::extract`,
`ZipStreamReader::extract`); `candle-core` calls only `ZipArchive::new`,
`by_index`, `by_name` and `ZipWriter` (`src/npy.rs`), and its only input is a
local, operator-provided model package. That bounded the impact but did not close
the finding.

**Closed by exclusion, not by waiver.** The Candle integration boundary was
quarantined out of the workspace:
`crates/amatl-core/src/experimental_embeddings/` and its five integration tests
moved to `experiments/local-embeddings-boundary/`, a standalone crate listed in
the root `Cargo.toml`'s `exclude` with its own `Cargo.lock` — the same pattern
`experiments/embedding-relevance` already used. `amatl-core` no longer declares
the optional `experimental-local-embeddings` feature, the Candle dependencies or
`zip`, so the workspace `Cargo.lock` contains none of them and `cargo audit` is
green. No `ignore` entry was added to `deny.toml` and no exception was granted:
nothing is suppressed. Production behaviour is unchanged — the feature was off by
default, the module had no production caller (`evaluate_with_embeddings` and
`SemanticRescore` had zero call sites outside the module), and neither the
default dependency graph nor any build target changed.

The exclusion is itself the removal plan, and it is reversible: re-admit the
crate to the workspace and its dependencies to `amatl-core` once a candle release
depends on `zip >= 9.0.0`. `candle-core 0.11.0` still requires `^8.6.0`, so no
release does today.

Warnings are separate from advisories, and the class decides, not the entry:
`unmaintained` and `yanked` warn, `unsound` is an error. `tokio 1.53.1` failed
`cargo deny check` that way until it was remediated to `1.53.3` on 2026-10-25,
and `chacha20 0.10.1` — yanked, via `rand 0.10.2` → `rmcp 3.1.2` — was
remediated to `0.10.2` on 2026-10-26: a one-line `cargo update -p chacha20` that
the policy above would have let a reviewer accept, but that is cheaper to fix
than to carry. `paste 1.0.15` (unmaintained) disappeared with the Candle stack,
which was its only path. With `chacha20 0.10.2` locked, `cargo audit` reports no
yanked package in the dependency graph it scans, and no `yanked` warning is open
today. This paragraph describes the state on 2026-10-26; the 2026-11-13 re-audit
above supersedes its claim that `cargo deny check` is clean — it is red on the
single open `libsqlite3-sys` advisory (RUSTSEC-2026-0459), which is unrelated to
the yanked-package class discussed here.

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
