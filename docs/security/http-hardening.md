# HTTP security baseline

The Axum server hosts UI, REST-like JSON endpoints, and MCP Streamable HTTP on
one listener. Defaults below come from `ServerConfig::default`
(`config.rs:487-503`) and are validated before router construction.

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
`amatl-server/src/lib.rs:374-395`).

## Host, Origin, CORS, and methods

- Default hosts: `127.0.0.1`, `localhost`, `[::1]`; values must be exact HTTP
  authorities without wildcards (`config.rs:494,652-659`).
- An incoming `Host` is mandatory, structurally parsed, and matched exactly by
  authority or host (`amatl-server/src/lib.rs:430-442`).
- Explicit Origins must be root HTTP(S) origins without query/fragment. When
  none are configured, origins are derived from scheme + allowed host + port
  (`config.rs:661-672`, `amatl-server/src/lib.rs:493-504`).
- CORS allows only configured origins, GET/POST/OPTIONS, and Authorization,
  Content-Type, and Accept headers (`amatl-server/src/lib.rs:506-515`).

## Authentication and exposure matrix

Protected: `/search`, `/deep`, `/providers`, `/mcp` and descendants. Public:
`/health` and static UI assets. Tokens are read from `server.token_env`, must be
at least 32 bytes, and are compared in constant time
(`amatl-server/src/lib.rs:93-108,401-427`).

| Bind | `no_auth` | TLS pair | Valid configuration | Meaning |
|---|---:|---:|---:|---|
| Loopback | false | absent or complete | Yes | Bearer required; HTTP is allowed only locally |
| Loopback | true | absent or complete | Yes | Explicit development mode; no bearer |
| Non-loopback | false | complete | Yes | Mandatory bearer and TLS |
| Non-loopback | false | absent/partial | No | Rejected by validation |
| Non-loopback | true | any | No | Rejected by validation |

Certificate and key must both be set or both absent. Remote exposure does not
infer proxy TLS termination; AMATL itself requires the configured pair
(`config.rs:616-650`). rustls uses its safe protocol defaults (TLS 1.2 and 1.3),
and every outbound reqwest client also declares TLS 1.2 as its minimum
(`amatl-server/src/lib.rs:168-190`, `providers/http.rs:49-55`,
`fetch.rs:168-177`).

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
(`amatl-server/src/lib.rs:500-528`).

Rejected headers/bodies, Host/Origin failures, rate limiting, failed
authentication and request timeouts emit structured `amatl::security` events.
They include only a stable event code, normalized path and socket IP; Host,
Origin and credential values are deliberately omitted
(`amatl-server/src/lib.rs:316-418`).

## `doctor` versus `/health`

`amatl doctor` is an operator diagnostic: it loads/validates configuration,
reports every provider, checks SQLite health and migrations, reports telemetry
state, and shows whether token/TLS prerequisites are ready
(`amatl-cli/src/main.rs:201-207,240-260,403-443`). `/health` is a public,
lightweight process/router availability response only:
`{"schema_version":"1","status":"ok"}`. It does not validate providers,
SQLite, token readiness, or outbound network (`amatl-server/src/lib.rs:291-293`).

Tests cover hardened public health, bearer enforcement, the simple and
protected Host/Origin/CORS matrix, rate and body limits, secret-safe security
events, real socket-IP separation, TCP, conflicting HTTP message boundaries,
trusted/untrusted TLS certificates and MCP (`amatl-server/src/tests.rs`). The
provider transport also rejects an untrusted certificate without exposing its
credential (`providers/http.rs`). Remaining environmental gaps are connection
saturation, distributed/multi-process rate limiting and reverse-proxy
deployments; forwarded headers remain intentionally unsupported.
