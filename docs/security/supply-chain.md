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

Risk-based acknowledgement and remediation deadlines are **pending owner
definition**. The owner must publish targets by critical/high/medium/low
severity and nominate an accountable role before claiming a security SLA. Until
then, a failing audit blocks merge, but no elapsed-time promise is made.

An exception requires a written ADR with advisory, affected path, exploitability,
compensating controls, owner, expiration date, and removal plan. Because no
verified owner identity exists in the repository, no exception is pre-approved.

## SBOM

CI runs `cargo cyclonedx` and uploads all generated `.cdx.json` and `.cdx.xml`
files as the `amatl-sbom` workflow artifact. The workflow fails if none exists.
Consumers should verify the workflow revision, download the artifact from the
corresponding trusted run, and correlate package URLs/versions with
`Cargo.lock`.

Public release attachment, signature/attestation, review cadence, and artifact
retention are **pending owner definition**. Current retention inherits the Git
hosting platform configuration; the repository does not assert a duration.

## Response procedure

1. Reproduce the finding against the locked graph and identify reachable code.
2. Prefer a compatible upgrade; otherwise remove/replace the dependency or
   minimize features.
3. Add a regression/security test where the vulnerable behavior is relevant.
4. Regenerate and compare the SBOM; run the complete contract gate.
5. Publish an advisory/release note if users could be affected.

## SLSA posture

SLSA is aspirational: CI is defined as code and emits an SBOM, but the project
does not yet claim a SLSA build level, hermetic build, provenance attestation,
signed release, or isolated release builder. Such claims require a release
pipeline and independently verifiable provenance, neither of which exists.

References: [SLSA specification](https://slsa.dev/spec/) and
[CycloneDX](https://cyclonedx.org/).
