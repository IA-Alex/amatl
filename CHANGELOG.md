# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for published binaries. Workspace `0.1.0-rc.1` is the first release-candidate
version.

The data-contract `schema_version` is independent from binary SemVer. Compatible
additions may remain on schema `"1"`; a breaking external data change increments
it even if the binary version follows a different cadence. SQLite migration and
adapter/extractor versions are independent axes as well.

## [Unreleased]

### Added

- Provider registry (`ProviderRegistry`, `ProviderFactory`) and an open
  `[providers.<name>]` configuration map, so a search source is added by
  declaring a governance record and registering a factory —
  `AmatlService::with_registry` builds providers without a hardcoded match.
- Local inference layer (`amatl-core/src/inference.rs`): an `EmbeddingBackend`
  contract plus the offline, deterministic `local_hashing_v1` backend that now
  backs Ranking v2's `SemanticScorer` and `DeepReranker`, sized by the new
  `[inference]` configuration section. `remote_explicit` fails closed and Deep
  degrades with `inference_unavailable` when a required backend is missing.
- Shared error catalog (`amatl-core/src/errors.rs`): CLI, API and MCP render the
  same stable codes, transport statuses and messages.
- `amatl doctor` reports inference readiness; `amatl config` lists declared
  providers and the inference backend.
- Local domain HTTP surfaces backed by the existing SQLite tables: `GET /status`
  (source availability, persistence and cache state), `GET/DELETE /history`,
  `DELETE /history/{id}`, `GET/POST /saved` and `DELETE /saved/{id}`. Every
  executed search is recorded in the local history when
  `persistence.history_enabled` is on; all of them require the bearer token and
  fail closed with `storage_unavailable` when persistence is disabled.
- UI panels for service state, search history and saved documents, plus a
  `Guardar` action on each Deep document. The panels appear only when the
  corresponding surface answers.
- `/metrics` now publishes p50/p95/p99 latency over the last 1024 requests per
  surface, per-source availability and observed value
  (`amatl_source_available`, `amatl_source_success_rate`,
  `amatl_source_latency_ms`), cache hit/miss counters and hit rates, and
  `amatl_storage_available`.
- `[persistence] history_enabled` and `[persistence] saved_document_max_bytes`
  configuration keys.
- `contract-gate` runs the workspace test suite on macOS and Windows in addition
  to Linux, so the published Tier 2 archives cannot silently rot.

### Changed

- Semantic and reranker ranking weights now require an inference mode with an
  available backend; the configuration is rejected otherwise.
- HTTP and MCP surfaces report precise failures (`search_planning_failed`,
  `provider_not_registered`, `inference_unavailable`, …) instead of the generic
  `service_unavailable`, `search_failed` and `deep_search_failed` codes.
- The request id now reaches outbound work: `ProviderContext` and `FetchRequest`
  carry it, and provider calls and Deep fetches run inside spans that declare
  it. It is never sent to the provider or origin; MCP tool calls generate one
  per invocation.
- Result pagination is server-side only. The UI always sends `page`/`page_size`
  and renders the returned window as-is instead of keeping a parallel
  client-side pager; `SearchResponse.total_results` describes the whole ranked
  set. `/deep` remains unpaginated.
- UI copy moved out of `app.js` into the `/i18n.js` message catalog; adding a
  language means adding one entry with the same keys as `en`.
- `docs/release.md` states the distribution scope explicitly as Tier 1 (Linux
  x86_64 musl, native packages) and Tier 2 (Linux aarch64, macOS, Windows
  archives); everything else is out of scope.

### Removed

- Empty `api.rs`, `mcp.rs` and `ui.rs` surface markers in `amatl-core`; the real
  transport surfaces live in `amatl-server`.

### Fixed

- Debian release asset names avoid GitHub's `~` normalization so published
  `SHA256SUMS` manifests remain directly verifiable.

## [0.1.0-rc.1] - 2026-08-13

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
- CLI-only local ingestion with bounded file reads, deterministic dispatch for
  text, Markdown, HTML, JSON/JSONL, CSV, source code and PDF, producing Document
  plus Evidence v1/v2 without exposing filesystem access through HTTP or MCP.
- Functional embedded Deep UI with POST-only dispatch, bounded Evidence v2
  fragments, provenance inspection, text-only rendering and browser-side UTF-8
  range/SHA-256 verification; local file ingestion remains CLI-only.

### Changed

- Workspace SemVer advanced to `0.1.0-rc.1` and MSRV is enforced at Rust 1.88.
- SQLite mutations are serialized across storage clones to avoid lock loss under
  concurrent cache access.

The canonical repository is <https://github.com/IA-Alex/amatl>.

[Unreleased]: https://github.com/IA-Alex/amatl/compare/v0.1.0-rc.1...HEAD
[0.1.0-rc.1]: https://github.com/IA-Alex/amatl/releases/tag/v0.1.0-rc.1
