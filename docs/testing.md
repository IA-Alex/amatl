# Testing strategy and current coverage

The normative merge rule is in `plan_amatl.md` §16: provider,
Canonicalization, Deduplication, Budget, ranking, Fetcher, extractor, router and
Normalization are incomplete without contract tests. CI runs the whole
workspace, not a hand-picked suite.

## Matrix

| Area | Unit | Integration | Property | Security | Contract | Current evidence / known gap |
|---|---|---|---|---|---|---|
| Query | Yes | Search flow | Yes | malformed/operator limits | Yes | `query.rs`; `tests/properties.rs`; no large Unicode confusable corpus |
| Classification/plan | Yes | Search flow | Indirect | N/A: no privilege boundary | Yes | `classify.rs`, `planning.rs`, `search_contract.rs` |
| Provider/router | Adapter mapping | `providers_phase1`, adaptive routing | No | secret URL redaction | Yes | Real provider networks/quotas/regions are intentionally not exercised in CI |
| Budget/execution | Yes | concurrent Search | Invariants through property/contract tests | exhaustion/timeouts | Yes | No deterministic scheduler/model checker for every cancellation race |
| Normalization | Yes | result pipeline | URL inputs | invalid URL/degradation | Yes | `normalize.rs`, `result_pipeline_phase2.rs`, `properties.rs` |
| Canonicalization | Yes | result pipeline | Yes | URL parsing | Yes | Conservative rules covered; corpus is finite |
| Deduplication | Yes | result pipeline | Yes | poisoning is heuristic | Yes | Exact/possible/distinct covered; multilingual title similarity remains limited |
| Ranking/Diversity | Yes | result pipeline | score/rank invariants | N/A: deterministic product logic | Yes | MVP and v2 tested; operational relevance drift needs external evaluation |
| SQLite/cache | Yes | `persistence_phase3`, Deep cache | No | corruption/quarantine | Boundary tests | WAL/TTL/LRU/quotas covered; real filesystem failure and contention load absent |
| Telemetry/router | Yes | `adaptive_routing_phase4` | No | no secrets by schema review | Yes | 30-day decay/state/fallback covered; multi-process behavior absent |
| Credentials and scopes | Config validation units | In-process Axum | No | digest comparison, per-route scope, expiry, rotation on reload, rate window preserved | Yes | Several credentials, scope refusal (403 `scope_denied`), expired credential and hot rotation covered |
| MCP tool policy | Yes | tools/call | No | allowlist from the authenticated identity, header spoof refused by the transport | Yes | `fetch` denied to a restricted client while `search` is allowed |
| Security audit | Yes | HTTP rejections → SQLite → `/security-events` | No | admin-only, no secrets stored, bounded in-flight writes | Yes | Persistence, query order, admin-only access and fail-closed without SQLite covered |
| Crawl politeness | Parser units | Deep crawl with a routing fetcher | No | discovered links only, fail-closed on unreachable robots.txt | Yes | Group precedence, longest match, wildcards, delay cap, allow/disallow/unavailable paths covered |
| Circuit breaker | Yes | Service search rounds | No | open circuit removes a source, never adds one | Yes | Trip, cooldown, half-open probe, disabled policy and restart persistence covered |
| Inference | Local + remote units | Ranking v2 engine | No | endpoint scheme/credential rules, bearer only in headers, response width and batch bounds | Yes | Remote batching, credential handling, malformed responses and fail-closed policy resolution covered; no live vendor endpoint in CI |
| Governance gate | Yes | Service search rounds | No | unapproved enabled source is never built | Yes | Refusal is reported as a degradation, canary preflight unchanged |
| Fetcher/SSRF | Yes | Deep with fakes + MCP audit path | URL properties | private/mapped IP, mixed DNS, headers, redirect, secret-safe audit | Yes | No live DNS rebinding, slow-stream or real oversized SafeFetcher fixture; provider TLS-negative is covered separately |
| Extractor | Missing executable + fixed CLI contract | Real pinned Trafilatura 2.2.0 job | No | fixed args, stdin-only, output/time limits | Yes for optional CLI capability | Real HTML, metadata, noise exclusion, unavailable/timeout/output contract covered |
| Renderer | Fail-closed core unit | Real isolated Chromium harness job | No | bubblewrap network namespace, private profile, systemd memory/task/time limits | Isolation harness only | JavaScript DOM and blocked loopback proved; core CDP bridge remains unavailable |
| Deep/Evidence/Gap | Unit engines | `deep_phase5` | Unicode fragment ranges | budget/blocked fetch via fakes | Yes | Evidence v2 verifies exact offsets, hashes, provenance, UTF-8 safety and 8 × 512-byte limits; fetch/extract failures, ranking gate, max two subqueries and cache rights covered |
| Local ingestion | detector/extractors | CLI process | Evidence v2 reuse | input/output/time limits, binary rejection, isolated PDF denial | CLI JSON contract | Text/Markdown/HTML/JSON/JSONL/CSV/code dispatch and fake bounded PDF process covered; malformed real-world corpus and hostile parser fuzzing remain pending |
| CLI | Formatting/behavior through process | `amatl-cli/tests/cli.rs` | No | JSON/log separation indirectly | Surface contract | Exit codes and commands covered, including history/saved lifecycle, `db health/backups/downgrade/circuits` and listener overrides; no packaged binary/install matrix |
| UI | Asset unit tests | Served by server tests; Deep POST contract | No | CSP, safe DOM, URL protocols, POST-only Search/Deep, bounded Evidence v2, provenance linkage, Web Crypto verification and non-serializable bearer field | Presentation contract | Server-side-only pagination, bounded saved payloads, token on every protected call and locale-catalog parity asserted on the assets; no browser E2E/accessibility automation |
| HTTP API | Handler tests | In-process Axum + TCP/rustls | No | auth, Host, Origin/CORS, body, headers, rate, timeout, framing, request correlation | Yes | Real TCP/TLS, untrusted certificate, aggregate header rejection and handler cancellation covered; `/status`, `/history` and `/saved` covered for bearer requirement, fail-closed persistence, input validation and round trip; `/metrics` covered for latency quantiles, source and cache gauges; no connection-saturation or proxy test |
| MCP | Tool/limit units | initialize/list/call | No | shared HTTP gate + SafeFetcher | Yes + Protocol conformance | Exactly five tools covered, absence of an ingestion tool asserted, `status` limits compared against `ExecutionLimits`, server-side pagination and cancellation covered; `fetch` network behavior is unit-tested below transport; JSON-RPC conformance (invalid version, missing fields, unsupported methods, malformed arguments, auth, rate-limit) covered in `tests.rs` |
| Data policy | Config/service units | CLI + MCP | No | isolated fail-closed before network | Yes | Contradictory profile, provider/canary, Deep degradation and MCP bypass regressions covered; OS firewall remains deployment evidence |
| Soak/load | N/A | Sustained concurrent MCP + HTTP | No | N/A | Operational harness | 15 s, 16-way concurrency, p50/p95/p99, throughput, error rate, peak RSS; `#[ignore]` by default for nightly/soak CI lanes |

“N/A” is used only where the test class does not represent a meaningful threat;
it does not waive unit or contract coverage.

## Commands

```bash
cargo test --workspace
cargo test -p amatl-core --test properties
cargo test -p amatl-core --test providers_phase1
cargo test -p amatl-core --test result_pipeline_phase2
cargo test -p amatl-core --test persistence_phase3
cargo test -p amatl-core --test adaptive_routing_phase4
cargo test -p amatl-core --test deep_phase5
cargo test -p amatl-server
cargo test -p amatl-cli --test cli
cargo test -p amatl-server --test soak -- --ignored --nocapture  # soak/load test
```

Tests use deterministic mocks/fakes and temporary SQLite files. Provider
credentials and network access must never be required for the merge gate.

## Required additions

Add a regression test before a bug fix. Add property tests when an input space
has broad combinatorics. Add security tests whenever data crosses HTTP, DNS,
process, filesystem or provider boundaries. A changed public JSON shape requires
compatibility fixtures and `schema_version` analysis; a changed calibration
requires benchmark evidence, not schema churn.
