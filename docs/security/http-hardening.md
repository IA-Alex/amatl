# HTTP security baseline

The Axum server hosts UI and REST-like JSON endpoints on
one listener. Defaults below come from `ServerConfig::default`
(`config/server.rs:121-141`) and are validated before router construction.

The embedded UI submits Search as `POST /search` and Deep as `POST /deep`, both
with an `application/json` body. Its password-style token control has no form
`name`, is not persisted by application code, is cleared on page exit, and is
used only to construct the `Authorization: Bearer` header. A script failure can
therefore cause a safe failed form POST, but cannot serialize the token or query
into the URL. GET remains only as API compatibility and should not carry
sensitive text. Deep content is rendered only as text and external links are
restricted to credential-free HTTP(S) URLs.

## Content Security Policy and headers

Exact CSP (`amatl-ui/src/lib.rs:7`):

```text
default-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; form-action 'self'
```

`default-src`, scripts, styles, fonts, and connections remain same-origin;
objects are disabled; base URL cannot be redirected; frames are denied; images
permit same-origin and embedded data; forms stay same-origin. No `unsafe-inline`,
`unsafe-eval`, or wildcard is used. Every response also receives
`X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
`Permissions-Policy: camera=(), microphone=(), geolocation=()`,
`X-Frame-Options: DENY`, and `Cache-Control: no-store` unless an asset declares
its own cache policy. HTTPS adds `Strict-Transport-Security:
max-age=31536000; includeSubDomains` (`amatl-ui/src/lib.rs:39-53`,
`amatl-server/src/lib.rs:3096-3118 (secured)`).

## Host, Origin, CORS, and methods

- Default hosts: `127.0.0.1`, `localhost`, `[::1]`; values must be exact HTTP
  authorities without wildcards (`config/server.rs:129,456-464`).
- An incoming `Host` is mandatory, structurally parsed, and matched exactly by
  authority or host (`amatl-server/src/lib.rs:3217-3236 (valid_host)`).
- Explicit Origins must be root HTTP(S) origins without query/fragment. When
  none are configured, origins are derived from scheme + allowed host + port
  (`config/server.rs:465-475`, `amatl-server/src/lib.rs:3292-3303 (effective_origins)`).
- CORS allows only configured origins, GET/POST/PATCH/DELETE/OPTIONS, and
  Authorization, Content-Type, and Accept headers
  (`amatl-server/src/lib.rs:3305-3323 (cors_layer)`).

## Authentication and exposure matrix

Protected: `/search`, `/deep`, `/providers` and descendants. Public and
read-only: `/health`, `/ready`, `/metrics` and the static UI assets. The
exemption is declared per path *and* method in `required_scope`
(`amatl-server/src/lib.rs:557-642`), so a mutating method on one of those paths
does not inherit it and lands on the same fail-closed default as an undeclared
route. Tokens are read from `server.token_env`, must be at least 32 bytes, and
are compared in constant time (`amatl-server/src/lib.rs:53,460-528 (resolve_clients),3144-3204 (authenticate),3206-3215 (constant_time_eq)`).

`/metrics` is deliberately coarse for the same reason `/ready` is: it publishes
request counters, latency quantiles, cache/storage gauges and *aggregate* source
counts, never source names or per-source values. Which sources a deployment
declares is deployment-internal data and stays behind `/status`, which requires
the `read` scope (`amatl-server/src/lib.rs:2613-2682 (metrics),2438-2442 (status)`).

| Bind | `no_auth` | TLS pair | Valid configuration | Meaning |
|---|---:|---:|---:|---|
| Loopback | false | absent or complete | Yes | Bearer required; HTTP is allowed only locally |
| Loopback | true | absent or complete | Yes | Explicit development mode; no bearer |
| Non-loopback | false | complete | Yes | Mandatory bearer and TLS |
| Non-loopback | false | absent/partial | No | Rejected by validation |
| Non-loopback | true | any | No | Rejected by validation |

Certificate and key must both be set or both absent. Remote exposure does not
infer proxy TLS termination; AMATL itself requires the configured pair
(`config/server.rs:446-455`). rustls uses its safe protocol defaults (TLS 1.2 and
1.3), and every outbound reqwest client also declares TLS 1.2 as its minimum
(`amatl-server/src/lib.rs:865-868`, `providers/http.rs:49-55`,
`fetch.rs:168-177`).

The matrix is evaluated against the address the process is actually listening
on, not the `bind` the document declares. `--bind`/`--port` override the file at
startup and are never written back to it, so a document may legitimately name an
address this process is not serving. Every configuration accepted into a running
process — a `POST /reload`, a `SIGHUP`, or a `PATCH /server/pending-config` — is
therefore re-checked against the live listener and refused when it would leave
that listener without authentication or TLS (`config/mod.rs:525-541`;
`amatl-server/src/lib.rs:245-253 (validate_server_for_listener),2381-2387 (update_server_pending_config)`). A file declaring `127.0.0.1` cannot
switch `no_auth` on for a process bound to `0.0.0.0`. Moving the listener still
needs a restart, and a reload whose document declares a different address logs
`reload_listener_address_ignored` instead of pretending the listener moved.

## Resource limits

| Control | Default | Validation/runtime |
|---|---:|---|
| Body | 65,536 bytes | positive, at most 1 MiB; Content-Length precheck plus Axum body layer |
| Headers | 16,384 bytes aggregate | positive, at most 64 KiB |
| Request timeout | 30,000 ms | positive; wraps the handler |
| Idle/header timeout | 30,000 ms | positive; HTTP/1 keep-alive disabled, HTTP/2 keep-alive bounded |
| Rate | 60 protected requests/minute | positive; fixed 60 s in-memory window |
| Connections | 64 | 1–10,000; Tower concurrency limit |
| Query | 2,048 bytes | non-empty after trimming |

Rate keys use the remote IP obtained from the socket and never retain the
Authorization header. The limit applies before authentication and also covers
public routes; invalid credentials cannot create new buckets. Expired windows
are purged periodically rather than scanning on every request. Limits are per
process and do not trust forwarding headers
(`amatl-server/src/lib.rs:3255-3291 (within_rate_limit)`).

Forwarded headers are never consulted, and the cost of that choice is stated
rather than hidden: behind a reverse proxy every client shares the proxy's
address, so they share one rate-limit bucket and the audit trail names the proxy
instead of the caller. Trusting `X-Forwarded-For` would be worse — any client
could forge its own identity, and a forged key would also poison the audit
trail — so a deployment that needs per-client attribution must put an
authenticated proxy in front and let it enforce its own limits.

Rejected headers/bodies, Host/Origin failures, rate limiting, failed
authentication and request timeouts emit structured `amatl::security` events.
They include only a stable event code, normalized path and socket IP; Host,
Origin and credential values are deliberately omitted
(`amatl-server/src/lib.rs:2805-2971 (security_middleware),3144-3204 (authenticate)`).

Every application response receives a fresh `X-Request-ID` made from a
process-local sequence and timestamp. Incoming values are not reused. The ID is
also recorded on early security events and on an `http_request` tracing span so
downstream routing and SSRF events can be correlated. CORS exposes this response
header to explicitly allowed origins. HTTP parser failures that occur before
Axum middleware can run are outside this guarantee.

## `doctor` versus `/health`

`amatl doctor` is an operator diagnostic: it loads/validates configuration,
reports every provider, checks SQLite health and migrations, reports telemetry
state, and shows whether token/TLS prerequisites are ready
(`amatl-cli/src/main.rs:201-207,240-260,403-443`). `/health` is a public,
lightweight process/router availability response only:
`{"schema_version":"1","status":"ok"}`. It does not validate providers,
SQLite, token readiness, or outbound network (`amatl-server/src/lib.rs:2527-2530 (health)`).

Tests cover hardened public health, bearer enforcement, the simple and
protected Host/Origin/CORS matrix, rate and body limits, secret-safe security
events, real socket-IP separation, TCP, conflicting HTTP message boundaries,
trusted/untrusted TLS certificates, aggregate header rejection, handler timeout
cancellation and unique request IDs
(`amatl-server/src/tests.rs`). The
provider transport also rejects an untrusted certificate without exposing its
credential (`providers/http.rs`). Remaining environmental gaps are connection
saturation, distributed/multi-process rate limiting and reverse-proxy
deployments; forwarded headers remain intentionally unsupported.
