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

The accountable security role is repository owner `@IA-Alex`. The response SLA
in `SECURITY.md` requires acknowledgement in 1/2/5/10 business days and targets
remediation in 14/30/60/90 calendar days for critical/high/medium/low findings,
respectively. A failing audit blocks merge immediately; an overdue remediation
must record its compensating control and revised date in a private advisory.

An exception requires a written ADR with advisory, affected path, exploitability,
compensating controls, owner, expiration date, and removal plan. `@IA-Alex` must
approve it explicitly; no exception is pre-approved.

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
