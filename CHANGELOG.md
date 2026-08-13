# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for published binaries. No SemVer release has been published yet; workspace
`0.1.0` is a development version.

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

The canonical public repository URL is pending owner definition, so this file
does not invent release or comparison links.
