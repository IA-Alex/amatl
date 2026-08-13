# OWASP ASVS 5.0.0 checklist

Scope: AMATL's current local HTTP/API/MCP service. Target: prioritized L1–L2
requirements, not a claim of full ASVS certification. ASVS 5.0 reorganized the
chapters: authentication is V6, authorization V8, API/Web Service V4, data
protection V14, secure communication V12, configuration V13, logging V16, and
architecture/dependency controls V15. IDs are pinned as `v5.0.0-*`.

Status meanings: **Pass** is traceable to implementation and a test; **Partial**
has an implementation or documentation gap; **N/A** explains why the feature is
absent.

| Requirement | Level | Applies/status | Control | Code/document evidence | Test/evidence |
|---|---:|---|---|---|---|
| v5.0.0-1.2.2 safe URL protocols | 1 | Pass | Structured `Url`; only HTTP(S); embedded credentials rejected | `security.rs:4-24` | `security.rs:83-89` |
| v5.0.0-1.3.6 SSRF | 2 | Partial | Pre-connect, post-DNS, redirect validation, address pinning and secret-safe rejection audit; no domain/port allowlist | `security.rs`; `fetch.rs` | Unit/redirect tests plus correlated MCP rejection; live rebinding fixture absent |
| v5.0.0-2.1.1 validation rules documented | 1 | Pass | Query, URL and HTTP rules are explicit | `fase_a_contratos.md`; `docs/security/ssrf-controls.md`; `docs/security/http-hardening.md` | Query/security/server contract tests |
| v5.0.0-2.1.3 business limits documented | 2 | Pass | Budget and per-surface limits documented | `docs/configuracion.md`; `docs/api/mcp.md` | `budget.rs:209-245`; `service.rs:416-437` |
| v5.0.0-2.2.1 server-side validation | 1 | Pass | API/MCP validates query; config validates ranges; fetch validates URL/header | `amatl-server/src/lib.rs:214-278,397-399`; `config.rs:522-674`; `fetch.rs:96-143,221-230` | `amatl-server/src/tests.rs:49-107`; `config.rs:682-755`; `fetch.rs:301-333` |
| v5.0.0-2.4.1 anti-automation/resource abuse | 2 | Pass | Rate, concurrency, timeout and Budget bounds | `amatl-server/src/lib.rs:148-151,309-371,461-490`; `budget.rs` | `amatl-server/src/tests.rs:153-199`; `budget.rs:209-245` |
| v5.0.0-3.4.1 HSTS | 1 | Pass when HTTPS | One-year HSTS with subdomains, omitted on HTTP | `amatl-ui/src/lib.rs:39-53` | `amatl-ui/src/lib.rs:79-94` |
| v5.0.0-3.4.2 restrictive CORS | 1 | Pass | Exact allowlist; no wildcard | `config.rs:661-672`; `amatl-server/src/lib.rs:493-515` | `amatl-server/src/tests.rs:109-151` |
| v5.0.0-3.4.3 CSP | 2 | Pass | Self-only CSP; objects/frames denied; no unsafe directives | `amatl-ui/src/lib.rs:7,39-53` | `amatl-ui/src/lib.rs:79-106` |
| v5.0.0-3.4.4 `nosniff` | 2 | Pass | Header on all middleware responses | `amatl-ui/src/lib.rs:39-53`; `amatl-server/src/lib.rs:374-395` | `amatl-server/src/tests.rs:31-46` |
| v5.0.0-3.4.5 referrer policy | 2 | Pass | `no-referrer` | `amatl-ui/src/lib.rs:8,39-53` | `amatl-ui/src/lib.rs:79-94` |
| v5.0.0-3.4.6 anti-framing | 2 | Pass | CSP `frame-ancestors 'none'`, plus legacy DENY | `amatl-ui/src/lib.rs:7,39-53` | `amatl-ui/src/lib.rs:79-94` |
| v5.0.0-3.5.2 CORS/preflight defense | 1 | Pass | Exact Origin validation applies with or without preflight to public and protected routes | `amatl-server/src/lib.rs:342-485` | `host_and_origin_are_explicitly_validated` covers rejected/accepted simple cross-origin GETs |
| v5.0.0-4.1.1 response content types | 1 | Pass | Axum JSON typing and explicit static-asset MIME types | `amatl-server/src/lib.rs:214-307`; `amatl-ui/src/lib.rs:18-37` | `amatl-ui/src/lib.rs:64-77`; API JSON tests |
| v5.0.0-4.2.1 HTTP message boundaries | 2 | Pass for the owned listener | Hyper/Axum parsers, header/body limits, HTTP/1 keep-alive disabled | `amatl-server/src/lib.rs:148-205,309-418` | Oversize streamed body and real TCP conflicting-Content-Length regression tests |
| v5.0.0-4.3.2 GraphQL introspection | 2 | N/A | AMATL implements no GraphQL endpoint | `amatl-server/src/lib.rs:140-151` | Route inventory proves absence |
| v5.0.0-5.1.1 upload rules | 2 | N/A | AMATL has no upload feature | `amatl-server/src/lib.rs:140-151` | Route inventory proves absence |
| v5.0.0-6.1.1 authentication anti-automation docs | 1 | Partial | Shared bearer and rate limit are documented; no account/password login exists | `docs/security/http-hardening.md`; `amatl-server/src/lib.rs:461-490` | `amatl-server/src/tests.rs:153-170`; distributed-abuse test absent |
| v5.0.0-6.3.1 brute-force controls | 1 | Partial | Pre-authentication rate limiting by socket IP covers public and protected routes | `amatl-server/src/lib.rs:316-528` | Invalid-token rotation and real socket-IP tests; no multi-process coordination |
| v5.0.0-8.1.1 authorization rules documented | 1 | Pass for current model | `/health` and assets public; API/MCP protected; no roles/data tenancy | `docs/security/http-hardening.md`; `amatl-server/src/lib.rs:401-403` | `amatl-server/src/tests.rs:31-76` |
| v5.0.0-8.2.1 function-level authorization | 1 | Pass for current model | Same bearer gates every protected function | `amatl-server/src/lib.rs:348-364,401-417` | `amatl-server/src/tests.rs:49-76,201-314` |
| v5.0.0-12.1.1 current TLS versions | 1 | Pass | rustls safe server defaults permit TLS 1.2/1.3; outbound clients explicitly require TLS 1.2+ | `Cargo.toml`; `amatl-server/src/lib.rs:168-190`; `providers/http.rs`; `fetch.rs` | Real trusted rustls handshake and untrusted-certificate rejection |
| v5.0.0-12.2.1 TLS for external service | 1 | Pass by config rule | Non-loopback bind requires local TLS pair and authentication | `config.rs:616-650` | `config.rs:743-754` |
| v5.0.0-12.3.2 outbound certificate validation | 2 | Pass | reqwest/rustls validation, TLS 1.2 minimum and no insecure verifier | `Cargo.toml`; `providers/http.rs`; `fetch.rs` | Provider transport rejects an untrusted live certificate without leaking its credential |
| v5.0.0-13.1.1 communication inventory | 2 | Pass | Provider, arbitrary Deep, Trafilatura and disabled Chromium boundaries documented | `docs/security/threat-model.md`; `docs/arquitectura.md` | Architecture review |
| v5.0.0-13.2.1 centralized outbound policy | 2 | Pass at application boundary | `data_policy` closes provider, Deep, MCP and canary egress; isolated configuration fails closed and remote inference is denied | `config.rs`; `service.rs`; `amatl-server/src/mcp.rs` | Config/service/MCP isolated-policy regressions |
| v5.0.0-13.2.4 outbound allowlist | 2 | Partial | Protocol and non-public-address denylist, not a domain/path/port allowlist | `security.rs:10-76`; `fetch.rs:96-143` | SSRF tests; residual explicitly recorded |
| v5.0.0-13.4.1 browser process isolation | 2 | Pass for harness; core disabled | New user/process/filesystem/network namespaces, read-only runtime, private profile, cgroup memory/task/runtime limits and bounded DOM | `packaging/amatl-chromium-sandbox`; `docs/security/chromium-isolation.md` | Real Chromium JavaScript render plus loopback denial in dedicated workflow |
| v5.0.0-13.3.1 secret management | 2 | Partial | Secrets excluded from source/config and read from environment; no vault integration | `service.rs:384-390`; `docs/security/secrets.md` | Provider tests verify no token in mapped/sanitized request output |
| v5.0.0-14.1.2 data protection/retention docs | 2 | Pass for stored classes | Inventory, TTL, retention, access assumptions and purge limits documented | `docs/security/data-retention.md`; migrations | Cache/storage/telemetry tests |
| v5.0.0-14.2.1 secrets absent from URLs | 1 | Pass for server token; Partial for providers | UI uses POST JSON, token has no form name and is sent in Authorization; provider adapters may use query API keys but sanitization exists | `amatl-ui/assets/index.html`; `amatl-ui/assets/app.js`; `providers/http.rs:12-29` | UI asset contract; `providers/http.rs:109-119`; `providers/mojeek.rs:390-408` |
| v5.0.0-14.3.2 anti-caching | 2 | Pass | Default `Cache-Control: no-store`; only immutable static CSS/JS cache for one hour | `amatl-server/src/lib.rs:389-393`; `amatl-ui/src/lib.rs:18-34` | `amatl-server/src/tests.rs:31-46` |
| v5.0.0-15.1.1 dependency remediation times | 1 | Pass | Audit gate plus severity-based acknowledgement and remediation SLA owned by `@IA-Alex` | `.github/workflows/ci.yml`; `SECURITY.md`; `docs/security/supply-chain.md` | CI configuration and published policy |
| v5.0.0-15.1.2 SBOM/trusted sources | 2 | Pass for private CI; release pending | Cargo.lock, deny trusted registry, CycloneDX artifacts with explicit retention | `Cargo.lock`; `deny.toml`; `.github/workflows/ci.yml`; `.github/workflows/release.yml` | `cargo deny check`; `cargo cyclonedx`; musl archive includes four SBOMs and SHA-256 |
| v5.0.0-15.1.3 expensive functions documented | 2 | Pass | Search, Deep, MCP fetch and extraction limits catalogued | `docs/configuracion.md`; `docs/api/mcp.md` | Budget, Deep and server tests |
| v5.0.0-15.2.2 resource-demand defenses | 2 | Pass | Budget, deadlines, byte/depth/subquery/concurrency limits | `budget.rs`; `service.rs:33-58,123-275`; `config.rs:522-650` | `budget.rs:209-245`; Deep/API tests; aggregate header and handler-timeout regressions |
| v5.0.0-15.3.1 minimum returned fields | 1 | Pass | Search excludes `final_url` and internal ranking details | `model.rs:474-500`; `plan_amatl.md:763-798` | `amatl-server/src/tests.rs:69-76`; `tests/search_contract.rs` |
| v5.0.0-15.3.2 backend redirects | 2 | Pass | Provider redirects disabled; SafeFetcher follows only intended, bounded, revalidated redirects | `providers/http.rs:49-54`; `fetch.rs:122-130,168-174` | `fetch.rs:326-333` |
| v5.0.0-16.1.1 logging inventory | 2 | Partial | Format/fields and routing events documented; retention/access depend on stderr consumer | `plan_amatl.md:710-722`; `amatl-cli/src/main.rs:19-98`; `docs/operacion.md` | Formatter compiled/tested indirectly; external sink pending operator |
| v5.0.0-16.2.1 investigation metadata | 2 | Pass for HTTP operations | JSON events include timestamp, level, target, message, context and span chain; server-generated request ID crosses HTTP, routing and SSRF | `amatl-cli/src/main.rs`; `amatl-server/src/lib.rs` | Unique response IDs and correlated MCP SSRF event contract test |
| v5.0.0-16.2.5 sensitive-log protection | 2 | Partial with regression coverage | Generic errors, URL/field redaction, value-free audit events and a strict own-target tracing filter | `providers/http.rs`; `amatl-cli/src/main.rs`; `amatl-server/src/lib.rs` | Formatter, authenticated rejection, TLS and MCP URL-token canaries; free-form future AMATL fields remain a review risk |
| v5.0.0-16.3.3 security-event logging | 2 | Pass for current HTTP/SSRF controls | Headers/body/Host/Origin/rate/auth/timeout and SSRF rejections emit stable secret-safe events | `execution.rs`; `security.rs`; `fetch.rs`; `amatl-server/src/lib.rs` | HTTP rejection and end-to-end correlated MCP SSRF audit tests |
| v5.0.0-16.4.1 log injection | 2 | Pass for machine output; TTY library-managed | Non-TTY output is one JSON object per event and newlines are escaped | `amatl-cli/src/main.rs` | `json_logs_escape_newlines_and_redact_sensitive_fields` uses a hostile newline payload |

## Priority gaps

Before claiming ASVS L2 alignment: decide whether deployment requires a domain/port egress
allowlist; define any multi-process rate-limit requirement; and document trusted
reverse-proxy handling before supporting one.

Normative reference: [OWASP ASVS 5.0.0](https://github.com/OWASP/ASVS/tree/v5.0.0/5.0).
