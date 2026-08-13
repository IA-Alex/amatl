# SSRF and outbound URL controls

AMATL applies the same `SafeFetcher` to Deep and the MCP `fetch` tool. Provider
API clients are separate adapters with fixed endpoints; this document describes
user/result-controlled outbound URLs.

Before URL validation, the central data policy selects the network
implementation. `profile = "isolated"` (valid only with `egress = "deny"`)
installs a denied fetcher/transport, so Deep, MCP fetch, providers and canaries
fail before DNS or connection. `SafeFetcher` is constructed only when effective
egress is governed.

## Validation sequence

1. **Before DNS/connect:** parse as `Url`; allow only `http` or `https`; reject
   embedded username/password, missing host, internal host suffixes, and
   non-public literal IPs (`security.rs:4-24`, `fetch.rs:96-108`).
2. **After DNS:** resolve the host, reject an empty answer and reject the entire
   answer if any address is non-public (`security.rs:26-33`,
   `fetch.rs:109-114`).
3. **At connect:** disable proxies and automatic redirects, then pin the already
   validated `SocketAddr` set with `resolve_to_addrs` (`fetch.rs:161-187`).
4. **Every redirect:** accept only 301, 302, 303, 307, or 308; structurally join
   `Location`; rerun URL validation; on the next loop rerun DNS validation and
   pinning (`fetch.rs:122-130,251-260`).

This ordering limits DNS rebinding between validation and connection. It does
not claim network-layer egress filtering.

## Block catalog

Hostnames are lowercased and stripped of a terminal dot. `localhost` and names
ending in `.localhost`, `.local`, `.localdomain`, `.internal`, `.intranet`,
`.lan`, or `.home` are blocked
(`security.rs:36-42`).

IPv4 blocks (`security.rs:51-64`):

- private: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`;
- loopback `127.0.0.0/8`, link-local `169.254.0.0/16`, unspecified and
  `0.0.0.0/8`, limited broadcast, multicast `224.0.0.0/4`, reserved
  `240.0.0.0/4`;
- shared address space `100.64.0.0/10`;
- protocol assignments `192.0.0.0/24`;
- benchmarking `198.18.0.0/15`;
- documentation `192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24`.

IPv6 blocks (`security.rs:66-77`): loopback `::1`, unspecified `::`, multicast
`ff00::/8`, unique-local `fc00::/7`, link-local `fe80::/10`, documentation
`2001:db8::/32`, and any IPv4-mapped address whose IPv4 value is blocked.

## Request and response limits

- Request headers are an allowlist of `accept`, `accept-language`,
  `if-modified-since`, `if-none-match`, and `user-agent`; authorization and
  cookies cannot be forwarded
  (`fetch.rs:221-230`).
- Response headers are reduced to content type/length, last-modified, ETag, and
  cache-control (`fetch.rs:233-249`).
- The body is streamed and rejected as soon as the configured byte limit would
  be exceeded (`fetch.rs:205-211`).
- A single absolute deadline covers DNS, redirects, and body transfer
  (`fetch.rs:98-120`).
- Deep defaults: 20 MiB global bytes, 10 fetches, five redirects, 10 crawl URLs,
  depth one, 20 s (`config.rs:414-429`). MCP `fetch`: 256 KiB, two redirects,
  three seconds (`amatl-server/src/mcp.rs:60-72`).

## Verification and gaps

Every rejected initial URL, redirect URL/location, or DNS answer emits an
`amatl::security` event containing only `security_event=ssrf_blocked`, stage and
stable reason. It deliberately omits URL, hostname, query and resolved
addresses. When the fetch originates in HTTP/MCP, the surrounding
`http_request` span supplies the same `request_id` returned as `X-Request-ID`.
Third-party tracing targets are excluded from CLI/server output to prevent an
upstream library from copying full MCP arguments into logs.

Tests prove rejection before DNS, rejection of mixed public/private DNS answers,
private ranges including IPv4-mapped IPv6, sensitive headers, and a redirect to
a private literal, and end-to-end MCP correlation without URL/token logging
(`security.rs`, `fetch.rs`, `amatl-server/src/tests.rs`). Property tests exercise
URL parsing/canonicalization (`tests/properties.rs`).

Residual gaps: there is no live rebinding integration fixture, no enforced
operator-level firewall, no public-domain allowlist, and no classification of a
public IP that an operator routes to internal infrastructure. The application
gate does not sandbox external processes. Deployment should add network egress
policy as defense in depth.

Reference: [OWASP SSRF Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html).
