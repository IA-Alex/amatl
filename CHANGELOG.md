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
  same stable codes, transport statuses and messages. The CLI prints
  `error_code=…` on stderr for any failure carrying a catalog code, and reports
  a failed Search with the composite codes the response already contains.
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
- Governed remote inference: `data_policy.inference = "remote_explicit"` now has
  a real backend (`remote_embeddings_v1`) behind the async `EmbeddingBackend`
  contract, configured by `[inference] remote_endpoint`, `remote_model`,
  `remote_credential_env`, `remote_timeout_ms` and `remote_max_batch`. It is
  built only under a standard profile with governed egress, refuses endpoints
  that are not HTTPS (or loopback HTTP) or that embed credentials, sends the
  credential only as a bearer header, and bounds batch, input, response width
  and time. Anything else fails closed.
- Persistent provider circuit breaker (`[circuit_breaker]`, migration 0006): a
  source that fails repeatedly is skipped for a cooldown, then probed once. The
  state survives restarts when persistence is enabled, and is visible through
  `GET /status`, `/metrics`, the MCP `status` tool and `amatl db circuits`.
- Runtime configuration reload: `POST /reload` and `SIGHUP` rebuild the service
  from the configuration file and swap it atomically, so adding, removing or
  re-approving a source no longer needs a restart. HTTP and MCP share the same
  handle. `ProviderRegistry::unregister` completes the registry lifecycle.
- MCP `status` tool, server-side pagination on `search`, cancellation and
  progress notifications for `deep_search`.
- CLI commands for the local domain and maintenance: `amatl history
  list|delete|purge`, `amatl saved list|show|delete`, and `amatl db
  health|backups|restore|downgrade|circuits`. `serve` and `mcp serve` accept
  `--bind`, `--port` and `--json`.
- Named credentials (`[[server.clients]]`): each one carries its own HTTP
  scopes, MCP tool allowlist and optional expiry. Secrets stay out of
  configuration — declare the environment variable or the SHA-256 digest — and
  are matched as digests in constant time. `POST /reload` and `SIGHUP` rotate,
  add and revoke credentials without a restart, while deliberately preserving
  rate-limit windows. The single `server.token_env` token still works as the
  `default` client.
- Per-tool MCP authorization: every tool checks the authenticated identity's
  allowlist, so `fetch` can be denied to a client without disabling egress for
  everyone. The decision never reads a client-supplied header.
- Durable security audit (`security_events`, migration 0007): edge rejections
  are persisted with request id, identity, path and address, queryable through
  `GET /security-events` (admin scope) and `amatl db security-events`, with
  `persistence.audit_retention_days` retention. Writes are backgrounded and
  bounded; drops are counted in `amatl_audit_events_dropped_total`.
- `robots.txt` compliance for crawl-discovered links (`[deep] respect_robots`):
  user-requested URLs are fetched as a user agent, while links AMATL discovers
  itself at depth ≥ 1 obey the origin's rules, including `Crawl-delay` within
  the Deep deadline. An unreachable `robots.txt` stops the crawl instead of
  assuming consent.
- Ranking v2 is gated in CI: `contract-gate` and the release workflow run
  `amatl benchmark ranking-v2`, and a unit test pins the recorded calibration so
  a silent drift fails the build.

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
- The provider governance record is enforced at call time, not only at startup:
  an enabled source whose approval is incomplete or expired is never built and
  the response carries a `provider_not_approved` degradation naming it.
- MCP `fetch` limits are derived from `ExecutionLimits::for_surface` instead of
  being hardcoded, so lowering a configured limit lowers it for MCP too. The
  effective numbers are reported by the `status` tool.
- `SemanticScorer`, `DeepReranker`, `EmbeddingBackend` and `RankingV2Engine::rank`
  are async, which is what makes a remote backend possible without blocking the
  runtime.
- The document cache is namespaced by the active vector space
  (`backend@dimensions`), so changing the embedding backend or width stops
  matching old entries instead of silently reusing artifacts from another space.
- Local file ingestion remains CLI-only by design; a contract test now asserts
  that no MCP tool exposes it.

### Removed

- Empty `api.rs`, `mcp.rs` and `ui.rs` surface markers in `amatl-core`; the real
  transport surfaces live in `amatl-server`.

### Fixed

- Debian release asset names avoid GitHub's `~` normalization so published
  `SHA256SUMS` manifests remain directly verifiable.
- SQLite downgrade from migration 5 now applies: the script was present but
  outside `migrations/downgrade/` and unreferenced, so a downgrade silently
  skipped it. `amatl db downgrade` exercises the whole chain.
- Database backups are written with `VACUUM INTO` instead of copying the file
  while WAL mode is active. A plain copy could omit the most recent commits or
  capture a torn state, and this applied to the copy taken before a destructive
  schema migration as well. Backup verification now opens the copy read-only, so
  certifying an artifact no longer modifies it.
- `amatl db backups` lists automatic backups, and `amatl db restore` can select
  them. The three naming schemes (automatic, pre-migration, pre-restore) had
  diverged and the listing recognised only two, leaving every automatic backup
  unreachable from the product. Rotation now only removes automatic copies.
- Dropping `AmatlService` stops its background maintenance task. The task held a
  `CancellationToken`, which does not cancel on drop, so it outlived the service
  and kept the connection pool and the advisory file lock alive; with
  `locking_mode = "exclusive"` no other process could reopen the database.
- The native HTML extractor traverses the DOM iteratively. The recursive walk
  overflowed the stack on deeply nested markup — an abort, not a catchable
  panic — reachable from any fetched page well within the size budget.
- `inference.backend = "local_model_v1"` can be selected from a configuration
  file. Validation accepted only `local_hashing_v1`, so the documented backend
  failed to load and was unreachable end to end.
- `FallbackExtractor::version()` reports a composite identity instead of its
  primary's. Deep keys the document cache on this value, so natively extracted
  documents were stored under Trafilatura's version and served as if that
  extractor had produced them.
- Filesystem space reporting in `db health` uses `statvfs` rather than the
  directory inode's block count, which reported ~32 KiB total and pinned disk
  usage at 0%, making the "disk critically full" warning unreachable.
- Confusable folding no longer transliterates Greek phonetically. Mapping θ and
  φ onto `o`, or γ and ψ onto `y`, collapsed letters that look nothing alike and
  could mark unrelated Greek titles as possible duplicates. The final sigma `ς`
  now agrees with the medial `σ`, an invariant the folding itself had broken.
- The embedding cache evicts by real recency and persists its ordering, is
  written atomically, and warns instead of silently discarding an unreadable
  file. Ordering was restored from a map in hash order, making eviction
  arbitrary after every restart.
- The local model file is size-bounded before loading, so a mistyped path no
  longer risks an out-of-memory abort at startup.
- The soak test negotiates the MCP protocol version. Every MCP request in it had
  been rejected, a steady 33% error rate that went unnoticed because the test is
  `#[ignore]`d and no workflow ran it; a nightly job now does.
- `publish-aur.yml` runs `makepkg` in an Arch container as an unprivileged user
  (it is absent from `ubuntu-latest` and refuses to run as root), and rewrites
  `sha256sums` unconditionally — anchoring on the initial `SKIP` matched nothing
  from the second release onward and would have shipped a stale checksum.

### Changed

- Deep's default reranker stays lexical. `local_hashing_v1` is a feature hash,
  not a model, and ranking the labeled corpus by cosine similarity over those
  hashes scores measurably worse than lexical coverage (nDCG@3 0.925 against
  1.000). Embedding-based reranking is selected only when a genuine model
  backend is configured, and never for a remote backend, which would ship every
  candidate document's text to a third party on each Deep call. A degradation
  to lexical is now logged instead of discarded silently.
- `publish-crates.yml` and `publish-aur.yml` trigger on stable tags only, with
  `workflow_dispatch` for candidates. Publishing to crates.io is irreversible
  and an AUR push replaces the package every Arch user installs.
- The crates.io publish waits on the sparse index for the leaf crates instead of
  sleeping a fixed 30 seconds before publishing their dependents.

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
