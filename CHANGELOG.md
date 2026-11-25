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

- `GET /search/batch` (HTTP) executes `1..=5` independent searches in one
  request and returns one outcome per query in caller order —
  `AmatlService::search_batch`. Items are independent: a query that fails
  validation or execution yields that item's catalog `error` while the rest of
  the batch still completes, and each item carries either a full `SearchResponse`
  or an `error` (mutually exclusive). Envelope validation rejects an empty batch
  or more than five queries with `invalid_request`; each query runs sequentially
  with its own budget, recording circuit outcomes and history like a `/search`
  call. Reuses the `Search` scope and stays on `schema_version` `"1"`.
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
  surface, cache hit/miss counters and hit rates, `amatl_storage_available`, and
  *aggregate* source health (`amatl_sources_declared`, `amatl_sources_available`,
  `amatl_sources_circuit_open`, `amatl_sources_success_rate_min`,
  `amatl_sources_latency_ms_max`). Source names are deliberately absent from the
  exposition: the endpoint is public, so per-source identity stays behind
  `/status`, which requires the `read` scope.
- Two new blocking static-analysis gates. `tools/codeql/` is a CodeQL pack with
  AMATL's own invariant queries — "only `amatl-core`'s `config/mod.rs` validates
  a server configuration against the declared `bind`", and "the two places that
  accept a configuration into a running process still anchor that validation to
  the effective listener" — and CI fails when either reports a violation.
  `tools/check_doc_refs.py` fails when a `file:line` citation in `docs/` names a
  file that does not exist or a line past the end of that file;
  that check is how the `config.rs` citations left behind by the config module
  split were found.
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
- Optional grounded answer synthesis ("Resumen con IA", `amatl-core/src/answer.rs`):
  a second remote-inference capability, independent from the embeddings
  backend above, that turns AMATL's own ranked Search results into a short,
  cited answer. The model only ever sees the sources AMATL retrieved
  (bounded by `max_sources`/`max_source_chars`); every `[n]` citation in the
  response is validated against real source indices, and a citation to a
  source that was never retrieved is stripped from the visible text, not
  merely excluded from the count. A response citing no valid source is
  refused (`answer_unavailable`) rather than shown. Requires
  `data_policy.inference = "remote_explicit"` and a new `[answer]`
  configuration section (`enabled`, `endpoint`, `model`, `credential_env`,
  `timeout_ms`, `max_sources`, `max_source_chars`, `max_answer_tokens`).
  Exposed on `POST /answer`, the MCP tool `answer`, and the always-visible,
  visually-disabled-when-unavailable `Resumen con IA` button in the web UI.
  See `docs/resumen-con-ia.md`, ADR-011 and the new `Core → inference
  (answer)` rows in `docs/security/threat-model.md`.
- Admin-scoped `POST /answer/enabled`, backed by `Config::set_answer_enabled`
  (via `toml_edit`): flips only the `answer.enabled` key in the configuration
  file, preserving every comment and every other key, and validates a
  candidate configuration with the flag flipped *before* writing anything —
  turning the feature on with an incomplete `[answer]` section fails closed
  without touching the file. `GET /status`'s `answer` field now reports
  `enabled`/`configured`/`available` as three independent booleans, so a
  settings panel can keep explaining the feature (and offer the toggle) even
  while it is switched off.
- Light/dark theme toggle in the web UI (`#theme-toggle`): follows
  `prefers-color-scheme` by default, can be overridden per visit, and shows
  exactly one of the sun/moon icons at a time via a CSS `is-active` class
  rather than element visibility attributes. A full light-theme token palette
  was added to `styles.css`, verified WCAG-AA.
- `docs/identidad-visual.md`: the canonical reference for AMATL's color
  tokens (dark and light) and typography, with the rule that no new
  functional color ships without being documented there first.
- Primary/expansion provider roles (`[expansion]`, STEP 1): `ExpansionConfig`
  with `mode = "legacy"` (default) or `"primary_expansion"` plus
  `primary_provider`. In `primary_expansion` the primary provider always
  occupies the first round and the expansion providers are only reached in
  later rounds; `RoleAssignment`/`ProviderRole` in the router make the
  PRIMARY→EXPANSION progression observable per round. In `legacy` the section
  is inert and routing keeps its pre-role behavior.
- Provider complementarity metrics (`complementarity.rs`): an objective,
  structural measurement of what the PRIMARY role found, what EXPANSION found,
  what overlapped (confirmed by dedupe) and what was exclusive to each —
  derived entirely from post-dedupe provenance, never a relevance claim.
- Deterministic relevance assessment (`relevance.rs`): `assess_result`
  classifies each result as `Relevant`/`PossiblyRelevant`/`NotRelevant` from
  explicit negative/entity evidence and lexical corroboration. It is a pure
  observation that never mutates routing, ranking, telemetry or provider
  selection.
- Bounded semantic relevance signals (`relevance_semantics.rs`): an advisory
  semantic layer with fixed precedence
  `explicit negative/entity evidence > semantic embedding evidence > lexical
  corroboration`. It can only *suggest* `PossiblyRelevant → Relevant`, and only
  when there is no contradiction and the bounded-semantic layer independently
  corroborates; it is never a sovereign classifier.
- Empirical validation corpus, a blind held-out corpus and a final
  bounded-semantic held-out campaign (offline, deterministic, mock providers
  only) that pin the relevance layer's behaviour.
- Real pure-Rust Candle integration (STEP 4D) behind the
  `experimental-local-embeddings` feature (still OFF by default):
  `CandleBackend` (`bge-small-en-v1.5`, CLS pooling + L2), `ModelPackage`
  (pinned sha256 of the three model files, purely local, fails closed) and
  `SemanticEvaluator` (query embedded exactly once, one batched document pass
  over a bounded candidate set). Candle/tokenizers/safetensors are optional
  `dep:`-gated dependencies; the default build's dependency graph is unchanged.
- Route/OpenAPI inventory unification (`routes.rs`): a single macro declares
  each public route's path and methods and generates both the Axum
  registration and the inventory used by the OpenAPI coverage guard, so adding
  a route requires declaring it in one place.
- Search-source selector in the web UI: a single-choice control that derives
  from the authoritative `providers.enabled` list and exposes
  Marginalia / SearXNG / All. Picking one issues the ordered per-provider
  `POST /providers/{name}/enabled` toggles to reach exactly that membership, so
  the operator chooses which sources (and therefore what the strategist LLM
  refines over) run per search.
- Retained-documents expiry (`saved_documents_retention_days`): saved
  documents older than the configured window are pruned on the maintenance
  pass, bounding table growth while keeping the unlimited (0) default.
- Cache retention indexes (migration 0008): `created_at` indexes on both cache
  tables so the eviction/filter path is index-backed instead of a linear scan.
- Atomic configuration writes (`config::write_config_document`): every config
  setter writes to a process-temp file, fsyncs and renames over the target, so
  a crash mid-write cannot leave `amatl.toml` truncated.
- Interface language picker in the web UI (`#ui-language`, English/Español). The
  locale is an explicit, persisted operator choice (`localStorage`), resolved as
  stored choice → browser preference → catalog default, so a visitor whose
  browser asks for a locale the catalog does not ship can still reach the
  complete `es` catalog that was already shipping. Switching re-applies every
  translation in place — no reload and no lost results — re-renders the
  catalog-derived panels and status line, and moves `document.documentElement.lang`
  with it. Singular/plural copy needs no ICU dependency: a `<key>One` entry is
  selected when the count is exactly 1, and a locale that ships only the base
  key keeps working.
- `docs/security/hallazgos-2026-10-26.md`, the dated report of the 2026-10-26
  security review over `ce172d9d`: scope, method, one regression test per
  finding and — explicitly — the claims that could not be verified locally (no
  remote CI, no `cargo publish`, no tag). It is a dated *informe*, not a norm:
  the controls stay in `docs/security/threat-model.md` and
  `docs/security/asvs-checklist.md`, and both `docs/README.md` and
  `docs/seguridad.md` link the report.

### Changed

- The STEP 4D experimental local-embedding stack leaves the workspace lockfile,
  closing the three `zip` findings without a waiver. `amatl-core` no longer
  declares the optional `experimental-local-embeddings` feature or the
  `candle-core`/`candle-nn`/`candle-transformers`/`tokenizers`/`safetensors`/
  `memmap2` dependencies: the module
  `crates/amatl-core/src/experimental_embeddings/` and its five integration tests
  moved verbatim to `experiments/local-embeddings-boundary/`, a crate excluded
  from the workspace with its own lockfile — the pattern
  `experiments/embedding-relevance` already used. `candle-core 0.9.2` depends on
  `zip ^7.2.0`, and `zip < 9.0.0` carries RUSTSEC-2026-0367 (7.3 high),
  RUSTSEC-2026-0368 and RUSTSEC-2026-0369 (9.1 critical), patched only from
  `zip 9.0.0`; no candle release reaches that fix (0.9.2 needs `^7.2.0`, 0.11.0
  needs `^8.6.0`). The feature was off by default and the module had no
  production caller (`evaluate_with_embeddings` and `SemanticRescore` had zero
  call sites outside it), so the default dependency graph, every build target and
  the search path are unchanged. Nothing is suppressed: no `ignore` entry was
  added to `deny.toml`, and the workspace `cargo audit` step of the contract gate
  now passes on the graph itself. The quarantine is the removal plan and is
  reversible — re-admit the crate once a candle release depends on
  `zip >= 9.0.0`.
- Search now reports `failure` whenever the final normalized result set is
  empty. In particular, an empty transport-level response from one provider
  can no longer mask a peer's failure as `success`; the regression is covered
  by `empty_successful_provider_does_not_mask_a_peer_failure`.
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
- The web UI's brand mark was replaced end to end (`brand-icon.png`,
  `favicon.png`), scoped so only the icon symbol carries the new brown color
  and the rest of the application keeps its functional blue/cyan/emerald
  palette; the wordmark keeps JetBrains Mono. The masthead's promotional
  subtitle ("Búsqueda multifuente y evidencia verificable") was removed
  entirely — it read as marketing copy, not product state, and the header now
  only carries the brand and the theme toggle.
- The circuit breaker admits a single half-open probe per cooldown: the first
  caller after cooldown claims the probe and concurrent callers are refused
  until it completes, so a recovering source is not stampeded. A claim older
  than the cooldown is reclaimed.
- Readiness follows the deployment the operator asked for. `/ready` and
  `GET /status` degrade on an enabled source that cannot serve — not approved,
  reporting `unavailable`, or with its circuit open — and no longer on a source
  that is merely declared. A declared-but-disabled source is a configuration
  choice, so a single-source deployment that is serving traffic stays ready
  instead of being drained by a load balancer; an instance with no enabled
  usable source still answers `503`, because it cannot search at all. The
  per-source detail (name, `provider_disabled`, `provider_not_approved`,
  circuit) stays in `/status` behind the `read` scope.

### Removed

- The MCP surface, retired end to end. `amatl-server/src/mcp.rs` is gone, along
  with the `/mcp` mount and its scope arms, `ServiceSurfaceKind::Mcp`,
  `Scope::Mcp`, `MCP_TOOLS`, the per-credential `tools` allowlist
  (`ServerClient.tools`, the `POST`/`PATCH /server/clients` fields and the
  `ServerClientSummary` output), and the `amatl mcp serve` CLI subcommand. The
  `rmcp` dependency and the MCP-only `schemars`/`chrono` crates leave the
  workspace, so the `RUSTSEC-2026-0543` suppression is closed by exclusion
  (`deny.toml`, both workflows' `--expected-ignore`, and the supply-chain
  contract assertion); `RUSTSEC-2026-0459` remains the single suppression.
  `docs/api/mcp.md` and `docs/diagrams/secuencia-mcp.*` are deleted and the
  related docs, OpenAPI, tests and C4 diagrams updated. Search, Deep, ingestion,
  persistence and the `[inference]` embedding backend are untouched; the
  surfaces are now CLI, UI and API HTTP. See ADR-020.
- The optional answer-synthesis (`answer`) and search-strategist (`strategist`)
  chat capabilities, retired end to end. The modules (`amatl-core/src/answer.rs`,
  `strategist.rs`, `strategist/llm.rs`, `config/strategist.rs`) are gone, and so
  are the `[answer]`/`[strategist]` configuration sections, the `Config` fields
  and their validate-before-write toggles, the `POST /answer`, `PATCH /answer`
  and `POST /answer/enabled`/`/strategist/enabled` routes, the `answer` MCP tool
  (the surface now exposes **five** tools), the `answer`/`strategist` read-only
  blocks in `/status`, the `amatl_answer_*` metrics and the
  `answer_unavailable` error code. AMATL's Search and Deep stay fully
  deterministic and unchanged; the optional remote *embedding* backend
  (`[inference]`) is untouched. The web UI follows: the "AI summary" button, the
  answer card, the AI-configuration panel and the "search with model" toggle are
  removed from the markup, styles, message catalog and docs, and the UI's MCP
  tool vocabulary drops `answer` so a client created with the `mcp` scope
  validates again (`MCP_TOOL_NAMES` mirrors `MCP_TOOLS`). `docs/resumen-con-ia.md`
  is deleted.
- Brave as a shipped source: the adapter module (`providers/brave.rs`), its
  registry factory, its built-in governance record and the UI credential
  affordance are gone, so `brave` in `providers.enabled` now fails configuration
  validation and no request can be sent. The credential routes themselves stay —
  `POST`/`DELETE /providers/{name}/credential` are parameterized by provider, not
  Brave-specific — and now reject `brave` like any other name the registry does
  not carry. The canary workflow's provider choices and secrets follow the
  shipped sources (`searxng`, `marginalia`).
- Mojeek as a shipped source, completing the paid-source policy drop: the
  adapter module (`providers/mojeek.rs`), its registry factory and its built-in
  governance record are gone, so `ProviderRegistry::builtin()` now ships exactly
  SearXNG and Marginalia. Removal is enforced at **two** doors instead of one:
  beyond the missing adapter, `RETIRED_PROVIDERS` and `is_retired_provider()`
  (`config/providers.rs`) make `Config::validate()` reject `brave` and `mojeek` by
  name — in a declared `[providers.<name>]` table as well as in
  `providers.enabled` — so a stale configuration fails loudly at startup instead
  of silently shipping one source fewer, and even a hand-built config that lists
  a retired name stops at the registry (`UnknownProvider`). UI selectors, key
  dialogs, message-catalog keys and the canary workflows no longer mention either
  name; the only surviving `brave` in the tree is the upstream SearXNG engine
  name in the `providers/searxng.rs` fixtures.
- Empty `api.rs`, `mcp.rs` and `ui.rs` surface markers in `amatl-core`; the real
  transport surfaces live in `amatl-server`.

### Removals planned for 1.0

- The deprecated `local_model_batch` alias for the `[inference]`
  `local_cache_capacity` key, kept only so configs written against rc.1 still
  parse. Tracked here so it cannot outlive the 1.0 boundary; the `TODO(1.0)`
  markers in `crates/amatl-core/src/config/inference.rs` and `config/mod.rs`
  point back to this entry.

### Fixed

- `crates/amatl-core/Cargo.toml` declared `readme = "../../README.md"`, a path
  outside the package, while the crate ships its own
  `crates/amatl-core/README.md`. Cargo resolves that collision in favour of the
  local file and rewrites the field in the published manifest, so the source
  declaration described a package that was never published, and every
  `cargo package -p amatl-core` — including the `--list` that
  `.github/workflows/publish-crates.yml` runs — printed a warning asking for a
  path relative to the package root. The field is now `readme = "README.md"`,
  which is what Cargo was already publishing: the tarball is unchanged and the
  warning is gone. The other three crates keep `../../README.md`, which is
  correct for them — they ship no README of their own and Cargo copies the root
  one into the package.
- Two TLS advisories that entered the RustSec database on 2026-10-19 and
  2026-10-20 leave the lockfile in one pass: `rustls-webpki` 0.103.14 moves to
  0.103.16 (RUSTSEC-2026-0382, certificate verification can accept a path
  through distrusted emSign intermediates whose private keys are now well known)
  and `webpki-roots` 1.0.9 moves to 1.0.11 (RUSTSEC-2026-0381, the bundled trust
  store still carried `emSign ECC Root CA - G3`). Both sit in the primary TLS
  path — `rustls` → `axum-server` for the listener and `hyper-rustls` →
  `reqwest` for outbound HTTPS — and both advisories had a single-package fix
  inside the locked minor line. `tokio` 1.53.1 moves to 1.53.3 in the same pass
  (RUSTSEC-2026-0386: the safe `AsyncFd` constructors accept any `T: AsRawFd`,
  so a duplicated-then-closed descriptor can be deregistered and freed while
  `epoll` still holds it, a use-after-free when readiness fires). `cargo deny
  check` is clean again — it had been failing on that `unsound` advisory, which
  cargo-deny denies as an error while `cargo audit` reports it as a warning —
  and `cargo audit` reported only the `zip` findings that were still open at that
  point; they were closed by exclusion later the same day, see the
  `### Changed` entry above.
- The offline supply-chain contract asserts the floors those fixes established
  (`rustls-webpki` 0.103.16, `webpki-roots` 1.0.10, `tokio` 1.53.3), so a later
  downgrade fails `cargo test --workspace` on every host instead of passing
  while the crate is vulnerable.
- `docs/security/supply-chain.md` described a `zip`-only audit and claimed the
  offline floors covered every advisory in the lockfile. Both were false once
  the two TLS advisories landed, and the claim is what let the offline test pass
  green while `cargo audit` was red. The document now states the rule the
  contract actually implements — a floor for every *remediated* advisory whose
  fix the locked graph can reach, with unreachable ones recorded as open
  findings — records the three remediations with their dates, and stops
  presenting the offline test as equivalent to the audit.
- The `declared-bind-validation` invariant exempted the whole
  `crates/amatl-core/src/config/` directory, so a *new* module added beside
  `mod.rs` could call the bind-blind `validate_server` unreported; reproduced
  against a CodeQL database built from a scratch checkout carrying such a
  module. The exemption is now the exact file that holds the two legitimate
  callers. Its companion `listener-anchored-count` counted only the anchors, so
  renaming the loader that `listener-anchored-acceptance` keys on left both
  queries silent; they now share that identification through
  `acceptance_points.qll`, and the count asserts the acceptance-point number
  too. Both checks stay presence-based: they prove that the anchored call is
  made, not which address it is handed, which is the boundary these invariants
  are defined at.
- A configuration is now validated against the address the process is actually
  listening on, not the `bind` the document declares. `--bind`/`--port` override
  the file at startup and are never written back to it, so a file declaring
  `127.0.0.1` could carry `no_auth = true` and `POST /reload` would publish an
  unauthenticated service on every interface, including the credential setters
  and the admin routes. `PATCH /server/pending-config` could stage the same
  state in the file for the next restart. Both are now refused
  (`configuration_invalid` / `config_rejected`) with the running service left
  authenticated, and a reload whose document declares a different address logs
  `reload_listener_address_ignored` instead of appearing to move the listener.
  `Config::validate_server_for_listener` exposes the check.
- A configuration write no longer widens the file it publishes. Every
  admin-scoped toggle rewrites `amatl.toml` through a temp file plus an atomic
  rename, and the rename published the *temp* file's permissions, so a file the
  operator had hardened (`chmod 600 amatl.toml`) came back with the umask
  default after the first `POST /answer/enabled`, `PATCH /providers/{name}` or
  any other config write. The target's own mode is now carried across the
  replace, the temp file is created owner-only so configuration is never group-
  or world-readable while it is being written, and a target that does not exist
  yet stays `0600`.
- The public-surface exemption is read-only. `required_scope` exempted
  `/health`, `/ready`, `/metrics` and the UI asset paths from authentication by
  path alone, and the decision is taken before routing: a mutating method on one
  of those paths was therefore unauthenticated instead of fail-closed, so a
  future `POST /health` — or any state-changing route registered under an asset
  path — would have been served without a credential. The exemption now covers
  `GET`/`HEAD` only and every other method lands on the same admin-protected
  default as an undeclared route. `PATCH` also joins the mutating set that
  decides the `/history`/`/saved` scope.
- `/metrics` published the name of every configured source as a label value
  while being reachable without a credential (`amatl_source_available{source}`,
  `amatl_source_circuit_open{source}`, `amatl_source_success_rate{source}`,
  `amatl_source_latency_ms{source}`), so an unauthenticated scraper could read
  the deployment's provider inventory and which of them were failing. `/ready`
  is deliberately coarse for exactly that reason; the scrape now publishes
  aggregate counts plus the worst observed value, and per-source detail stays
  behind `/status`, which requires the `read` scope.
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
- `answer.rs`'s public `synthesize` doc comment linked to a private item,
  which `cargo doc --no-deps` with `RUSTDOCFLAGS="-D warnings"` — part of the
  required gate — rejects as an error, not a lint.
- `.gitignore`'s `/amatl.sqlite3*` pattern did not match the backup file names
  `storage.rs` actually produces (`amatl.backup-<timestamp>.sqlite3`,
  `amatl.migration-<timestamp>.sqlite3`, `amatl.pre-restore-<timestamp>.sqlite3`),
  so a real backup — potentially containing the operator's search history and
  saved documents — was one `git add -A` away from entering history. Widened
  to `/amatl*.sqlite3*`; also added `*~` for stray editor-backup files.
- AUDIT-01: the OpenAPI route inventory (`docs/api/openapi.yaml`) was
  synchronized with the actual router and a coverage-guard test now fails the
  build if a public route is missing from the spec.
- AUDIT-02: `.gitignore` now ignores nested AMATL SQLite state.
- AUDIT-03: `.gitignore` now ignores runtime logs.
- AUDIT-04: the Cargo target artifact lifecycle is documented in
  `DEVELOPMENT.md` and `docs/operacion.md`.
- AUDIT-05: Marginalia credentials now require verification before use
  (`docs/security/secrets.md`, `amatl.example.toml`,
  `docs/gobernanza-providers.md`, `docs/operacion.md`).
- AUDIT-06: SearXNG filter preservation and security contact sync
  (`providers/searxng.rs`, `CODE_OF_CONDUCT.md`, `SECURITY.md`).
- Repository-hygiene remediations from the validation audit (`config.rs`,
  `errors.rs`, `lib.rs`, `i18n.js`, `bench.rs`, `benchmark_plan_runner.py`).
- Migration downgrades 0006/0007 now remove their version row from
  `amatl_schema_migrations`, keeping the ledger consistent with the actual
  schema after a downgrade.
- Provider parsers (marginalia, searxng) tolerate malformed
  envelope items: optional fields default instead of failing the whole parse,
  empty-URL items are dropped, and provider ranks are renumbered over the final
  vector so SearXNG answers and results keep a unique ordinal.
- BA-01: `rustls` moves from 0.23.43 to 0.23.46 in `Cargo.lock`, the first
  release that closes every advisory the locked graph was exposed to —
  RUSTSEC-2026-0285 (TLS 1.3 handshake messages accepted across encryption
  level boundaries, fixed in 0.23.45), RUSTSEC-2026-0346 (`Debug` on client
  session values exposes TLS secrets) and RUSTSEC-2026-0349 (key-exchange
  downgrade recorded before the server is authenticated), both fixed in
  0.23.46. RUSTSEC-2026-0348 (Brotli certificate decompression can allocate up
  to 1 GiB before authentication) requires the non-default `brotli` feature,
  which this graph never enabled: the locked feature set is `ring,std,tls12`.
  The bump is a one-package lockfile change. `cargo deny check` is clean again
  and `cargo audit` no longer reports `rustls`; the findings that remain in that
  audit belong to the optional `experimental-local-embeddings` stack and are
  recorded as open in `docs/security/supply-chain.md`.
- Five `quinn-proto` advisories leave the lockfile in the same pass: 0.11.16
  moves to 0.11.19, at or above the 0.11.17 fixes for RUSTSEC-2026-0339/0341/
  0342 and the 0.11.18 fixes for RUSTSEC-2026-0338/0340. The crate is an
  optional HTTP/3 dependency of `reqwest` that this graph never enables
  (`cargo tree -i quinn-proto` prints nothing with default or with all workspace
  features), so the change cannot alter a compiled path; it is recorded because
  `cargo audit` scans every locked package, not only the resolved build graph.
- A hermetic advisory-floor guard (`amatl-core/tests/supply_chain_contract.rs`)
  fails `cargo test --workspace` offline, on every host, when a locked version
  falls below an advisory's fixed release. It reads the same `Cargo.lock` CI
  installs with `--locked`, so the boundary is enforced by the test suite and
  not only by the network-dependent audit in the Linux job. It asserts the
  remediated `rustls` and `quinn-proto` floors; an advisory that no compatible
  release can reach is recorded as open in `docs/security/supply-chain.md`
  instead of as a floor the locked graph cannot meet.
- `clippy::single_element_loop` in `config/mod.rs` no longer fails
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`: the
  one-element `for` over the paid providers became a named, reusable assertion
- The documented readiness rule is stated as a table instead of a sentence,
  because the sentence was read as "a deployment whose sources are all declared
  and disabled is ready", and the two cases differ. `docs/operacion.md` now
  spells out that one or more *enabled* sources with at least one usable answers
  `200`, that enabled-but-unusable answers `503`, and that no enabled source
  answers `503`: declaring and disabling sources is neither an incident nor
  capacity.
- Readiness counts *available* sources, not usable ones, and the two diverge
  whenever a circuit is open, so `sources_available` can read `1` on a `503`
  response. That was true by construction and undocumented; the rustdoc on the
  `/ready` handler and `docs/operacion.md` now state it, and
  `ready_reports_degraded_while_counting_an_available_source_with_an_open_circuit`
  pins it against a later "cleanup" of the count. The field's meaning is
  deliberately unchanged: renaming or redefining it is a contract change that
  needs its own consumer review.
- The `file:line` citations into `crates/amatl-server/src/lib.rs` in
  `docs/security/threat-model.md`, `docs/security/http-hardening.md` and
  `docs/security/asvs-checklist.md` were re-anchored after the new rustdoc above
  the `/ready` handler shifted that file by nine lines, so every citation still
  resolves to the code it names.
- The yanked `chacha20` 0.10.1 leaves the lockfile, at 0.10.2. The path is the
  one the supply-chain notes already described (`chacha20` ← `rand 0.10.2` ←
  `rmcp 3.1.2` ← `amatl-server` ← `amatl-cli`, per `cargo tree -i chacha20
  --locked`), so this is the one-line `cargo update -p chacha20` the warning
  class would have let a reviewer accept, not a dependency change. Evidence with
  the real advisory database: `cargo audit` loads 1352 advisories, scans 382
  locked dependencies and exits 0 with no yanked warning, and `cargo deny check`
  answers `advisories ok, bans ok, licenses ok, sources ok` with no new `ignore`
  entry — the `yanked` class stays a warning, there is simply nothing left in it.
  `docs/security/supply-chain.md` no longer lists the case as an open warning.
- `docs/release.md` records which event runs which workflow, because they are not
  interchangeable: `contract-gate` runs on push, pull request, a nightly cron and
  manually, with `soak` only on cron or dispatch; `static-analysis` and
  `codeql-invariants` are path-filtered and also run weekly; `chromium-isolation`
  is a Linux-only contract; `provider-canary` is manual-only;
  `release-candidate` accepts a manual dispatch, a `packaging/**`-only push to
  `main` and `v*-rc.*` tags; and `publish-crates`/`publish-aur` accept stable
  `v[0-9]+.[0-9]+.[0-9]+` tags or a manual dispatch. The section states the two
  rules the split implies: a local `cargo test --workspace` runs none of the
  macOS/Windows, Chrome WebDriver, MSRV, CodeQL or soak jobs — those are
  reproducible individually by running each job's own commands, which
  `CONTINUIDAD.md` records, but reproducing a command is not reproducing the run
  and no local command can accredit `cross-platform` on a Linux host — and a tag
  is a publication rather than a marker — crates.io only yanks, the AUR replaces
  the package every Arch user installs — so neither tags nor manual dispatches are
  taken without a specific owner decision.
- The `/metrics` source gauges and `/ready`'s `sources_available` are now
  documented as the pair they are: `amatl_sources_declared` counts every declared
  source including the disabled ones — which report `unavailable` with
  `provider_disabled` — while `amatl_sources_available` counts only those whose
  surface status is `available`, the same quantity `/ready` publishes in a real
  deployment. So `declared − available` is not an incident count — it is two on a
  single-source deployment by configuration — and `docs/operacion.md` says so
  where operators read the readiness rule.
  `source_gauges_count_declared_sources_while_readiness_counts_available_ones`
  pins all three numbers against one deployment, so neither gauge can drift into
  the other's meaning.
- The one deployment where those two surfaces disagree is pinned too, because it
  is a real trap rather than a hypothetical one: `amatl serve --mock` reports the
  mock sources it searches with from `/ready` (`200`, `"sources_available":2` in
  the isolated mock deployment) while `/metrics` keeps describing the declared
  real sources (`amatl_sources_declared 3`, `amatl_sources_available 0`), so a
  healthy mock deployment publishes `amatl_sources_available 0` and an
  availability alert on that gauge would fire forever. It is the same shape D2
  removed from `/status`, and it stays documented and pinned
  (`mock_deployments_disagree_between_ready_and_the_source_gauges`) instead of
  being silently aligned: making `/metrics` mock-aware changes the value of a
  published metric, which needs its own consumer review.
- `tools/check_doc_refs.py` is back in `ruff format`'s canonical shape, which
  restores the format baseline the `static-analysis` comment quotes. `365b5836`
  added the six candle experiment tuples in the compact one-line style while the
  file's other tuples were already expanded, so the file stopped being formatted
  and the advisory count read 53 files where the comment said 52. The reflow
  touches no logic, the tool still answers `12 known-dead, 0 new` with exit 0,
  and the workflow comment now dates the number it quotes so a future drift shows
  up as a stale date instead of a silently false claim.
- The documents that describe the Brave/Mojeek retirement cited the deleted
  adapter modules as their evidence (`providers/brave.rs`, `providers/mojeek.rs`),
  which the blocking `tools/check_doc_refs.py` gate refuses by design: it
  reported five dead references and exit 1, so the removal was not shippable
  while its own documentation pointed at files that no longer exist. Both
  documents now cite what does exist — `RETIRED_PROVIDERS` in
  `config/providers.rs:148`, which is what `Config::validate()` reads — and the
  gate is back to `12 known-dead, 0 new`.
- The `[providers.<name>]` semantics the same smoke test exposed are now
  documented instead of being a silent trap: a declared table **replaces** the
  built-in record wholesale (the wire struct is `#[serde(default)]`), it does not
  merge field by field, so an approval table carrying only `approval_status`,
  `reviewed_at` and `reviewer` drops `terms_url`, `rate_limit`, `cost_model` and
  the rest, and the source falls to `provider_not_approved` with no startup error
  (`docs/configuracion.md`, `docs/operacion.md`). Pinned by
  `declaring_a_table_replaces_the_builtin_record_instead_of_merging_it`.
- Every external process AMATL spawns started from the parent's whole
  environment, which is exactly where `server.token_env` and each provider's
  `credential_env` live. Trafilatura, `pdftotext` and Chromium are the three
  parsers that read attacker-controlled bytes, so a memory-safety bug in one of
  them was silent credential theft rather than a crash. The new
  `amatl-core/src/child_process.rs` owns the policy: `scrub_environment` clears
  the environment and restores only `PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_ALL`
  and `TZ` — none of which can hold a secret, and a test asserts that of the list
  itself — while a spawn site that needs more names it explicitly through
  `forward_environment` (the renderer forwards `AMATL_CHROMIUM_BIN`,
  `AMATL_CHROMIUM_TASKS_MAX`, `XDG_RUNTIME_DIR` and
  `DBUS_SESSION_BUS_ADDRESS`). All four spawn sites (`extract.rs` twice,
  `ingest.rs`, `render.rs`) call it, `bwrap` is handed `--clearenv` so the
  sandbox clears again before applying its own `--setenv`, and the wiring is
  proved per path with a canary instead of by convention: canary tests in
  `extract.rs`, `ingest.rs` and `deep_phase5.rs` (the last one through
  `ChromiumRenderer::render` with a stand-in harness), plus a new
  `chromium-isolation` step whose stand-in "Chromium" dumps `env` into the DOM
  and must not find the canary while still finding `HOME`.
- `deep.extractor.executable` and `deep.renderer.sandbox_path` accepted a
  *relative* path containing a separator (`./trafilatura`, `bin/extract`), which
  `Command::new` resolves against the process working directory: whoever could
  write there chose which binary the service executed. Both are now an absolute
  path or a bare program name (`is_relative_program_path`), and the harness path
  is checked whenever the renderer is enabled, so enabling it later re-validates
  instead of skipping the check by ordering.
- `HttpRequest` no longer derives `Debug`. A hand-written impl prints the same
  URL `sanitized_url` produces and the header *names* only, so the obvious
  `tracing::warn!(?request, …)` can no longer publish the API key some providers
  carry as a query parameter or the `Authorization` value the operator
  authenticates with.
- The two intermittent failures the review reproduced under load are closed.
  `extract::tests::fixed_cli_contract_requests_metadata_without_network_arguments`
  could fail with `ETXTBSY`: `execve` refuses to start a file that still has a
  writer, and the test writes the script it then executes, so a concurrent `fork`
  in another test can hold the descriptor open — a transient
  `ExecutableFileBusy` no caller ever wanted to see.
  `child_process::spawn_with_busy_retry` retries exactly that error (seven
  doublings from 1 ms, a 127 ms worst case, bounded well below every caller's
  timeout) at the spawn sites instead of the test sleeping its way out.
  `execution::tests::success_serializes_an_empty_error_list` asserted on a
  1 000 ms budget while the production `search_policy_v1` reserve is 750 ms, so
  one scheduling hiccup pushed the round into `DeadlineNear`, appended a
  `deadline_near` degradation and downgraded the status to `PartialSuccess`; the
  budgets of those tests move to 8 000 ms (the production `timeouts.global_ms`
  default) in `execution.rs` and in the `operational.rs` benchmark harnesses.
  **No production budget changed.** Twelve full-suite runs under load
  (`--test-threads 16`, 24 `yes` processes on 24 cores) are green: 5 976 tests,
  zero failures, no `DeadlineNear` stop.
- The web UI's `reportFailure` read a `message` identifier that `ac11624a` had
  already removed, so any failure reaching the `504` branch threw
  `ReferenceError: message is not defined` instead of reporting the timeout —
  the operator saw no status at all. Nothing caught it because the asset tests
  assert string presence rather than JS semantics and CI ships no JS linter;
  `report_failure_reads_only_error_properties_never_a_bare_message` now parses
  the function body and refuses a bare `message` reference.
- The web UI's form controls sat below WCAG 2.2 AA (1.4.11): `--border` is a
  divider and measures ~1.26:1 against `--surface`. Inputs, selects and the new
  language picker use `--border-control`, a semantic alias of the existing
  `--muted` (5.20:1 dark, 5.60:1 light) rather than a new colour, and the
  boundary is recorded in `docs/identidad-visual.md`. The same pass fixes the
  announcement path: `aria-busy` moves from the query input to the `#results`
  region that actually changes, the search form is `novalidate` so the
  catalog-derived messages are what the operator sees, the static `lang`
  attribute matches the catalog default and is corrected before the first paint,
  and the result-language field gains a `datalist` plus `aria-describedby`
  instead of relying on its placeholder.
- The security documents that the review proved wrong were corrected in the same
  pass: `docs/security/secrets.md` gains the child-process section,
  `asvs-checklist.md` moves 13.3.1 to cover every spawned parser and 13.4.1 to
  record `--clearenv`, `http-hardening.md` states the cost of not trusting
  `X-Forwarded-For` instead of listing a trusted reverse proxy as pending,
  `chromium-isolation.md` documents the environment probe, and
  `docs/configuracion.md` names the accepted shape of both program paths. The
  `file:line` citations the `providers/http.rs` insertion had displaced were
  re-anchored, and `tools/check_doc_refs.py` stays at `12 known-dead, 0 new`.

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

### Changed

- The HTTP contract moved from `docs/api/openapi.yaml` to
  `crates/amatl-server/openapi.yaml`, next to the crate that serves it, and the
  package now carries it: the four `include_str!` guards in
  `crates/amatl-server/src/tests.rs` read a path that no longer escapes the
  package, so `cargo test` compiles and runs on an extracted tarball (C1 in
  `docs/security/hallazgos-2026-10-26.md`). `README.md`, `docs/README.md` and the
  two `docs/experiments/` documents point at the new path; `docs/api/` keeps
  `mcp.md`.

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
