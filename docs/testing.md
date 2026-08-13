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
| Fetcher/SSRF | Yes | Deep with fakes + MCP audit path | URL properties | private/mapped IP, mixed DNS, headers, redirect, secret-safe audit | Yes | No live DNS rebinding, slow-stream or real oversized SafeFetcher fixture; provider TLS-negative is covered separately |
| Extractor | Missing executable | Deep with fakes | No | fixed args/limits by review | Partial | Missing process typed; real Trafilatura success/timeout/output integration absent |
| Renderer | Fail-closed unit | No active backend | No | no unsafe fallback | Current contract only | Chromium/CDP is unavailable; isolation tests must precede activation |
| Deep/Evidence/Gap | Unit engines | `deep_phase5` | Unicode fragment ranges | budget/blocked fetch via fakes | Yes | Evidence v2 verifies exact offsets, hashes, provenance, UTF-8 safety and 8 × 512-byte limits; fetch/extract failures, ranking gate, max two subqueries and cache rights covered |
| CLI | Formatting/behavior through process | `amatl-cli/tests/cli.rs` | No | JSON/log separation indirectly | Surface contract | Exit codes and commands covered; no packaged binary/install matrix |
| UI | Asset unit tests | Served by server tests | No | CSP, safe DOM, URL protocols, POST-only UI flow and non-serializable bearer field | Presentation contract | No browser E2E/accessibility automation |
| HTTP API | Handler tests | In-process Axum + TCP/rustls | No | auth, Host, Origin/CORS, body, headers, rate, timeout, framing, request correlation | Yes | Real TCP/TLS, untrusted certificate, aggregate header rejection and handler cancellation covered; no connection-saturation or proxy test |
| MCP | Tool/limit units | initialize/list/call | No | shared HTTP gate + SafeFetcher | Yes | Exactly four tools covered; `fetch` network behavior is unit-tested below transport |
| Data policy | Config/service units | CLI + MCP | No | isolated fail-closed before network | Yes | Contradictory profile, provider/canary, Deep degradation and MCP bypass regressions covered; OS firewall remains deployment evidence |

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
```

Tests use deterministic mocks/fakes and temporary SQLite files. Provider
credentials and network access must never be required for the merge gate.

## Required additions

Add a regression test before a bug fix. Add property tests when an input space
has broad combinatorics. Add security tests whenever data crosses HTTP, DNS,
process, filesystem or provider boundaries. A changed public JSON shape requires
compatibility fixtures and `schema_version` analysis; a changed calibration
requires benchmark evidence, not schema churn.
