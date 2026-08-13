# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for published binaries. No SemVer release has been published yet; workspace
`0.1.0-rc.1` is the first release-candidate version.

The data-contract `schema_version` is independent from binary SemVer. Compatible
additions may remain on schema `"1"`; a breaking external data change increments
it even if the binary version follows a different cadence. SQLite migration and
adapter/extractor versions are independent axes as well.

## [Unreleased]

### Added

- Multi-provider Search pipeline with explicit Query, Classification,
  SearchPlan, Budget, Normalization, Canonicalization, Deduplication, ranking,
  Diversity, degradation and telemetry contracts.
- Governed Brave and Mojeek adapters; fail-closed DuckDuckGo HTML adapter.
- Optional SQLite provider/document caches and provider telemetry.
- Bounded Deep fetching, optional Trafilatura extraction, Evidence, Ranking v2,
  Gap analysis and SubQuery expansion; Chromium remains fail-closed.
- Shared CLI, embedded UI, HTTP API and MCP surfaces through `AmatlService`.
- SSRF/DNS pinning, bearer authentication, TLS configuration, CORS/Host/Origin,
  CSP, request/rate/concurrency limits, dependency audit/deny, and CycloneDX
  generation.
- Repository legal, security, architecture, API, operations, testing and
  contribution documentation.
- Shared bounded HTTP clients, streaming response limits, extraction deadlines,
  storage degradation reporting and process-wide provider telemetry.
- Fail-closed single-provider network canary with a manually approved CI path.
- Controlled operational Search/Deep/SQLite benchmark with latency, throughput,
  status, contention and peak-RSS evidence.
- Real rustls handshake coverage and a reproducible static Linux musl release
  workflow with CycloneDX SBOMs and SHA-256 checksums; attestation remains
  conditional on repository visibility and GitHub plan support.
- Secret-safe HTTP security events, hostile-newline log regression coverage,
  explicit TLS 1.2 floors and invalid-certificate/message-boundary tests.
- Server-generated request correlation across responses, routing and SSRF audit
  events, with third-party tracing targets excluded from operator logs.
- Contract coverage for aggregate HTTP header rejection and handler timeout
  cancellation, including stable error codes and response correlation IDs.
- Central `data_policy` with fail-closed `isolated` profile, denied provider,
  Deep and MCP egress, explicit disabled/local/remote inference modes and
  secret-safe `egress_denied` reporting. LLM inference remains optional.
- Embedded UI Search now uses POST JSON; the bearer token is excluded from form
  serialization, kept only in page memory, cleared on exit and reported with a
  specific authentication error without exposing its value.
- Additive Evidence v2 output with bounded exact-text fragments, deterministic
  identifiers and hashes, URL/fetch/extractor provenance, query-aware signals
  and the unchanged v1 score basis; Ranking v2 and Gap retain their calibration.

### Changed

- Workspace SemVer advanced to `0.1.0-rc.1` and MSRV is enforced at Rust 1.88.
- SQLite mutations are serialized across storage clones to avoid lock loss under
  concurrent cache access.

The canonical public repository URL is pending owner definition, so this file
does not invent release or comparison links.
