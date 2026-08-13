# MCP surface

AMATL exposes MCP Streamable HTTP at `/mcp` through the same Axum listener and
`AmatlService` used by API and CLI. It supports protocol version `2026-07-28`,
uses stateless protocol metadata, JSON responses, no legacy session mode, and
requires the same bearer token, Host/Origin checks, body limits and rate limit
as protected HTTP endpoints (`amatl-server/src/lib.rs`). Every response also
receives a server-generated `X-Request-ID`; the same identifier is attached to
the request span and any SSRF rejection audit emitted while handling that MCP
call.

## Tools

| Tool | Input | Output | Specific bounds |
|---|---|---|---|
| `search` | `{ "query": string }` | `SearchResponse` in `structuredContent` | query 1–2048 bytes; MCP surface limits below |
| `deep_search` | `{ "query": string }` | `DeepResponse` | query 1–2048 bytes; MCP surface limits below |
| `fetch` | `{ "url": string }` | schema version, final URL, HTTP status, content type, UTF-8-lossy content, size, retrieval time | public HTTP(S), 3,000 ms, 262,144 bytes, 2 redirects, SafeFetcher SSRF rules |
| `providers` | no parameters | provider summaries and capabilities | no outbound provider call |

Tool errors are structured with `schema_version: "1"` and a stable code such as
`invalid_query`, `invalid_url`, `search_failed`, `deep_search_failed`,
`fetch_failed`, or `providers_failed`. Internal details and provider credentials
are not returned (`amatl-server/src/mcp.rs:36-126`).

Con `[data_policy] profile = "isolated"` y `egress = "deny"`, `fetch` usa el
mismo gate central que Deep y responde `egress_denied` sin resolver DNS ni
conectar. `providers` reporta ese mismo código y `deep_search` conserva el
resultado Search con degradaciones de fetch. MCP no construye un cliente HTTP
independiente.

## Limits compared with local CLI/API

MCP takes the configured value or the stricter cap:

| Resource | CLI/API default | MCP cap |
|---|---:|---:|
| Provider calls | 3 | 2 |
| Provider timeout | 3,000 ms | 2,500 ms |
| Search timeout | 8,000 ms | 5,000 ms |
| Deep fetches | 10 | 3 |
| Deep bytes | 20 MiB | 2 MiB |
| Deep timeout | 20,000 ms | 10,000 ms |
| Gap subqueries | 2 | 1 |
| Gap cost | 2 | 1 |

If operator configuration is already lower, MCP does not increase it
(`service.rs:33-58`). The standalone `fetch` tool has its own even smaller
limits shown above and does not consume the Search/Deep Budget. It is therefore
a bounded public-network proxy for authenticated clients; rate limiting and
deployment egress policy remain necessary bajo `standard`. En `isolated` deja
de ser proxy. La política de AMATL no puede demostrar que el cliente MCP que
envía la consulta sea local; para ejercicios confidenciales el cliente/modelo
también debe ejecutarse localmente y el host debe aplicar defensa en profundidad.

The contract test initializes MCP, lists exactly these four tools, and calls
`search` (`amatl-server/src/tests.rs:201-314`).
