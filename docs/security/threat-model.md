# AMATL threat model

Status: baseline `51c6d34`, verified against the current tree on 2026-08-13;
re-reviewed 2026-08-16 after the optional remote-inference capability below was
added, per the "adding … an inference backend" review trigger this document
itself mandates; re-reviewed 2026-10-26 for the child-process environment scrub
and for the accepted `admin`/probe-surface risks recorded below; re-reviewed
2026-11-22 after the optional answer/strategist chat capability was retired.
Method: STRIDE per trust boundary. This model describes implemented controls; a
missing control is residual risk, not an implied capability.

## Assets and security objectives

Assets are provider credentials, the server bearer token, user queries,
retrieved documents, provider/cache telemetry, local SQLite data, host network
reachability, compute/memory/bandwidth budgets, and the integrity of result
ranking. The objectives are confidentiality of secrets, integrity and provenance
of results, bounded availability, and prevention of access to non-public network
resources.

## Data flow and trust boundaries

```mermaid
flowchart LR
  U[User] -->|query/token| S[CLI or embedded UI]
  S -->|HTTP| H[Axum API]
  H --> C[AmatlService / core]
  C --> G{Central data policy}
  G -->|standard / governed| P[External providers]
  G -->|standard / governed| F[Deep public Internet]
  G -. isolated / deny .-> X[Denied before DNS]
  C -->|optional state| DB[(SQLite)]
  C -->|HTML over stdin| T[Trafilatura process]
  C -. core bridge disabled .-> R[Isolated Chromium harness / future CDP]
  C -. optional, gated by data_policy.inference .-> L[Local or remote inference backend]
```

Each arrow crosses a trust boundary. CLI input is untrusted. The browser and
HTTP client are untrusted callers. Provider responses and all Internet content
are hostile. SQLite is local but corruptible. Trafilatura is an external
process. Chromium has a separately verified no-network Linux harness, but the
core bridge is not active.

The data policy models `disabled`, `local_only`, and `remote_explicit` so an
optional inference backend must obtain explicit permission. `isolated` can
allow local inference but never remote inference. One remote backend exists
today, opt-in and requiring `remote_explicit`: `[inference]`'s embedding
backend, used by Ranking v2's semantic scorer/reranker, but only if their
weights are raised above the default `0.0`. It is not required for Search or
Deep to function, and it is not reachable under `isolated`.

## STRIDE analysis by boundary

| Boundary | STRIDE | Threat | Implemented control | Evidence | Residual risk |
|---|---|---|---|---|---|
| User → CLI/UI | S/T | Forged or malformed query alters intended filters | Query grammar and contradictions are parsed centrally; API rejects empty or >2048-byte queries | `query.rs:16-216`; `amatl-server/src/lib.rs:948-951,987-991 (query rejection)`; `query.rs:222-250` | Unicode confusables and semantic abuse remain possible; client-side validation is not a control |
| User → CLI/UI | R/I | Query or token leaks through URL/history, UI state, or logs | UI uses POST JSON; token control has no form name, stays in page memory, clears on exit and is sent only as bearer; structured routing logs omit body/query/token | `amatl-ui/assets/index.html`; `amatl-ui/assets/app.js`; `amatl-server/src/tests.rs` | Shell history/process environment, browser memory/extensions and upstream body logging are operator-controlled |
| UI → API | S/E | Caller impersonates an authorized local client | Protected routes require a bearer token of at least 32 bytes, matched as a SHA-256 digest in constant time against named credentials with per-route scopes and optional expiry; rotation and revocation apply on `POST /reload` or `SIGHUP`; `no_auth` is loopback-only, and every reload and `[server]` patch is re-validated against the address the process is actually listening on, so a document cannot weaken the listener it is applied to | `amatl-server/src/lib.rs:53,241-267 (validate_server_for_listener),460-528 (resolve_clients),708-726 (ReloadHandle),910-938 (install_reload_signal),2358-2410 (update_server_pending_config),3144-3204 (authenticate),3206-3215 (constant_time_eq)`; `config/server.rs:446-455`; `config/mod.rs:1153-1162`; `amatl-server/src/tests.rs:49-76,2373-2496` | A deployment that keeps the single default token still has one shared identity; revocation is by configuration edit plus reload, not by a distributed revocation list |
| UI → API | T/I | Host, Origin, CORS, framing, or content-type abuse | Exact Host/Origin lists, restrictive CORS, CSP, `nosniff`, no inline/eval, frame denial | `amatl-server/src/lib.rs:3096-3118 (secured),3217-3253 (valid_host, valid_origin),3292-3323 (effective_origins, cors_layer)`; `amatl-ui/src/lib.rs:7-53`; `amatl-server/src/tests.rs:31-151`; `amatl-ui/src/lib.rs:79-106` | Reverse-proxy trust and request-smuggling behavior are not integration-tested |
| UI → API | I | A public surface discloses deployment internals, or serves a mutating method without a credential | The public exemption is declared per path *and* method: `/health`, `/ready`, `/metrics` and the UI assets are public only for `GET`/`HEAD`, so a mutating method falls back to the same fail-closed default as an undeclared route. `/ready` and `/metrics` are deliberately coarse — aggregate booleans, counts and latency quantiles, never source names or per-source values, which stay behind `/status` (`read` scope) | `amatl-server/src/lib.rs:557-642 (required_scope),2557-2611 (ready),2613-2682 (metrics),2438-2442 (status)`; `amatl-server/src/tests.rs:2196-2256,3675-3793` | An unauthenticated caller still learns the request counters, the latency profile and how many sources exist, and reaches the UI assets; if the port is exposed beyond the host, that exposure is the operator's to prevent |
| UI → API | D | Oversized, slow, concurrent, or automated requests exhaust service | 64 KiB body, 16 KiB headers, 30 s request/idle timeouts, 64 connections, 60 requests/minute keyed by socket IP before authentication | `config/server.rs:121-141`; `amatl-server/src/lib.rs`; `amatl-server/src/tests.rs` | In-memory rate windows are per process and not proxy-aware; many source IPs can distribute load |
| Core → outbound network | I/E | A provider, Deep fetch, canary, or future inference path leaks exercise data | Central `data_policy`; isolated profile requires denied egress and loopback, rejects remote inference/providers/renderer, installs denied fetcher/transport, and emits value-free `egress_denied` | `config/data_policy.rs`; `service.rs`; config/service tests | Application policy is not an OS firewall; external extractors are outside its process boundary |
| Core → providers | S/T | Provider endpoint or response is spoofed/poisoned | rustls certificate validation; provider-specific parsers; normalization, canonicalization, conservative dedupe, provenance, deterministic ranking/diversity | `Cargo.toml`; `providers/marginalia.rs`; `normalize.rs`; `canonical.rs`; `dedupe.rs`; `ranking.rs`; `tests/providers_phase1.rs`; `tests/result_pipeline_phase2.rs` | No cryptographic authenticity for search results; malicious but valid text/URLs may rank highly |
| Core → providers | R/I | Secrets leak in URL, error, or log | Credentials come from configured environment variables; provider errors are generic; sensitive query keys have a redaction view; `HttpRequest` has no derived `Debug` — its hand-written one prints the sanitized URL and the header *names* only, so the natural `?request` log site is redacted by default | `service.rs:384-390`; `providers/http.rs:12-29,72-101`; `providers/http.rs:140-148`; `providers/marginalia.rs:495-496`; `providers/http.rs` (redacted `Debug`) | `sanitized_url()` and the redacted `Debug` are tested, but nothing stops a future adapter from logging a raw `Url` it kept its own copy of: the redaction belongs to the request type, not to a wrapper every call site must go through |
| Core → providers | D | Provider latency, retries, quota, or large response drains resources | Provider/global deadlines, bounded concurrency, maximum two retries, jitter, no redirects, 2 MiB response cap, global provider-call Budget | `config/execution.rs:6-70`; `config/mod.rs:453-470`; `service.rs:123-164,325-326`; `providers/http.rs:108-183`; `budget.rs:165-204`; `execution.rs:44-155` | Retry policy can amplify provider traffic; no distributed quota coordination between processes |
| Deep → Internet | S/E | SSRF or DNS rebinding reaches loopback, private, link-local, internal names, or alternate schemes | URL checked before DNS, entire DNS answer validated, addresses pinned to connection, redirects disabled in client and revalidated each hop | `security.rs:4-76`; `fetch.rs:96-143,161-218,251-260`; `security.rs:83-116`; `fetch.rs:301-333` | Public IPs that route internally through operator infrastructure cannot be classified by application logic; no domain allowlist |
| Deep → Internet | T | Malicious `Location` changes scheme/host or injects credentials | Relative locations are structurally joined, schemes/credentials/host are validated, then DNS is resolved and validated again | `fetch.rs:122-130,255-260,102-120`; `fetch.rs:326-333` | Redirect tests cover private literal targets, not a live DNS rebinding server |
| Deep → Internet | D | Oversized content, redirect loops, crawl explosion, or expensive subqueries | DeepBudget accounts fetches, bytes, redirects, browser calls, crawl URLs, subqueries, cost, and deadline; depth ≤2 | `budget.rs:29-156`; `config/deep_config.rs:60-80,416-445`; `service.rs:209-267`; `budget.rs:219-245`; `tests/deep_phase5.rs:221-269` | Memory/CPU used while parsing within accepted byte limits is not separately metered |
| Deep → consumer | T/S | A hostile fragment is altered, detached from its source, injects markup, or is presented as authenticated truth | Evidence v2 binds exact UTF-8-safe byte ranges to source/extracted hashes, URL lineage, fetch/extractor metadata and deterministic IDs; fragments are capped at 8 × 512 bytes per document. The UI additionally caps documents/fragments, requires matching provenance, allowlists HTTP(S), renders with text-only DOM APIs and recomputes range/hash in-browser | `evidence.rs`; `tests/properties.rs`; `tests/deep_phase5.rs`; `amatl-ui/assets/app.js`; `amatl-ui/src/lib.rs`; `docs/evidence-v2.md` | Hashes detect mismatch but do not prove publisher identity, factual truth or source authenticity; browser extensions can still observe page data |
| Local filesystem → CLI/core | T/I/D | A hostile, oversized or mislabeled file exhausts parsers, injects active HTML or exposes a local path/content in shared output | Explicit single-file CLI surface; canonical file URI; 20 MiB input and 8 MiB output limits; signature/extension dispatch; UTF-8 validation; HTML drops executable text; JSON validates; no HTTP route or persistence | `ingest.rs`; `amatl-cli/tests/cli.rs`; `docs/ingestion-local.md` | HTML/JSON parsing has no independent CPU budget; `file:` provenance and JSON output are sensitive; TOCTOU replacement remains detectable only through the content hash |
| Core → local PDF process | T/E/D | Malicious PDF exploits, blocks or induces disclosure through `pdftotext` | Fixed executable/arguments; PDF bytes only over stdin; bounded stdout, 8 s timeout, kill-on-drop, stderr discarded; process denied before spawn when egress is denied/isolated; the child starts from a scrubbed environment, so it cannot read the bearer token or a provider credential | `ingest.rs`; `child_process.rs`; ingestion unit tests | No OS sandbox, executable hash pinning or proof that the child lacks network access; it cannot read the service credentials from its environment (`ingest.rs` canary test), but isolated deployments should still retain host firewall/sandbox and cannot ingest PDF through this path |
| Core → Trafilatura | T/E | Hostile HTML exploits or controls extractor process | Exact executable and fixed arguments; the executable must be an absolute path or a bare name, never a relative path resolved against the working directory (`config/deep_config.rs`); HTML only through stdin; stdout byte cap, timeout, kill-on-drop, stderr discarded; failure is typed and optional; the child starts from a scrubbed environment | `extract.rs:43-150`; `extract.rs:173-185`; `tests/deep_phase5.rs:271-285`; `child_process.rs`; `config/deep_config.rs` | No OS sandbox, seccomp, uid separation, network denial, or pinned executable hash; a compromised child no longer inherits the bearer token or the provider credentials, but it still runs with the service account's filesystem and network access |
| Core → Chromium | E/D | Remote JavaScript escapes browser, accesses host network or consumes resources | Core remains fail-closed; separate harness uses bubblewrap user/mount/PID/IPC/UTS/network namespaces, read-only runtime, private profile and systemd memory/task/runtime limits; the sandboxed browser starts from an empty environment (`bwrap --clearenv`), so only the harness's explicit `--setenv` entries reach it | `render.rs`; `packaging/amatl-chromium-sandbox`; `.github/workflows/chromium-isolation.yml`; `docs/security/chromium-isolation.md`; `child_process.rs` | Isolation is verified but not wired into core; a CDP bridge and review are still required before activation |
| Core → SQLite | T/I/D | Corruption, contention, or cache poisoning changes correctness | SQLite is optional; failures degrade; WAL, NORMAL sync, 5 s busy/acquire timeout, pool 4, header/quick-check quarantine, versioned keys and TTL/LRU quotas | `service.rs:101-112`; `storage.rs:58-118`; `cache.rs:39-99`; `document_cache.rs:24-85`; `storage.rs:581-670` | Database is not encrypted or authenticated; local users with file access can read or modify it |
| Deep → Internet | E/D | Deep fetch becomes a general network proxy | Fetch routes through the service policy and, when governed, uses SafeFetcher with the limits `ExecutionLimits::for_surface` derives (time, bytes, redirects) instead of hardcoded ones; `isolated` denies egress before DNS | `service.rs`; `fetch.rs`; `amatl-server/src/tests.rs` | Under `standard`, an authorized caller can use Deep as a bounded public-network proxy; `isolated` denies it |
| All → logs/errors | I/R | Secrets or attacker-controlled text leaks or forges logs | HTTP errors expose fixed codes; provider transport errors are generic; non-TTY logs are structured JSON | `amatl-server/src/lib.rs:3326-3355 (service_error, service_error_code, api_error)`; `providers/http.rs:140-148`; `amatl-cli/src/main.rs:19-98` | Security rejections are also persisted to `security_events` when SQLite is enabled and readable through `/security-events` (admin scope); under saturation events are dropped and counted. There is still no automated end-to-end secret-redaction test, and without persistence logs remain the only trail |

## Accepted risks and review triggers

- The shared bearer token is suitable for local/small trusted deployments, not
  multi-tenant authorization.
- The `admin` scope is equivalent to code execution as the service account:
  `PATCH /deep/extractor` and `PATCH /deep/renderer` choose the program the
  service executes, and that program inherits the account's filesystem and
  network access. Validation bounds the *shape* of the path (an absolute path
  or a bare name, never a relative one — `config/deep_config.rs`), not what a
  chosen program does, so an admin token needs the protection of shell access.
- `/health`, `/ready` and `/metrics` are public for `GET`/`HEAD` on whatever
  interface the operator binds, and `/metrics` publishes request counters,
  latency quantiles and aggregate source counts. Exposing the port is the
  operator's decision rather than an authentication failure; serving the probes
  from a second, loopback-only listener is not implemented.
- SQLite confidentiality depends on host filesystem controls.
- Provider result integrity is heuristic, not cryptographic.
- Deep is the only outbound network boundary beyond providers; under `standard`
  its fetch is a bounded public-web proxy by design, and `isolated` disables it.
- Trafilatura lacks OS-level isolation. Its environment — and that of
  `pdftotext` and the Chromium harness — is scrubbed of the service's secrets
  (`child_process`), so a compromised child does not also receive the bearer
  token or the provider credentials; it keeps the service account's filesystem
  and network access, and no OS sandbox, seccomp or uid separation is applied.
- Chromium is excluded from the active core capability surface; its isolation
  harness is independently tested.
- `isolated` is an application fail-closed control, not proof of host-wide
  containment; confidential deployments also require a local client/inference
  runtime and an OS/network sandbox.

Review this model before enabling Chromium, adding a provider or outbound
protocol or inference backend, changing authentication, trusting proxy headers,
storing document content by default, changing how an external process is spawned
or what environment it inherits, or changing any Budget/HTTP limit.

References: [OWASP ASVS 5.0.0](https://github.com/OWASP/ASVS/tree/v5.0.0/5.0),
[OWASP Top 10 A04:2021 Insecure Design](https://owasp.org/Top10/A04_2021-Insecure_Design/),
and [OWASP SAMM](https://owaspsamm.org/).
