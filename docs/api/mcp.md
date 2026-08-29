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
| `search` | `{ "query": string, "page"?: integer, "page_size"?: integer }` | `SearchResponse` in `structuredContent`, with `total_results`, `page` and `page_size` when paginated | query 1–2048 bytes; pagination is server-side and `page_size` is clamped to the MCP cap |
| `deep_search` | `{ "query": string }` | `DeepResponse`, including additive `evidence_v2` fragments and provenance | query 1–2048 bytes; MCP surface limits below; at most 8 fragments of 512 bytes per document; honors cancellation and reports progress |
| `fetch` | `{ "url": string }` | schema version, final URL, HTTP status, content type, UTF-8-lossy content, size, retrieval time | public HTTP(S) with SafeFetcher SSRF rules; time, byte and redirect limits derived from configuration for the MCP surface |
| `providers` | no parameters | provider summaries and capabilities | no outbound provider call |
| `status` | no parameters | service state (sources, storage, caches, inference backend) plus the limits in force for this surface | no outbound provider call; same aggregation as `GET /status` |

`deep_search` is the long operation. A client that sends
`notifications/cancelled` stops the wait immediately and receives
`request_cancelled`; a client that supplied `_meta.progressToken` also receives
`notifications/progress` at the start and end of the call. No tool holds work
after its caller has gone away.

Cada herramienta comprueba la lista `tools` de la credencial autenticada antes
de trabajar. La decisión usa la identidad que estableció el middleware HTTP, no
un encabezado del cliente: un `Mcp-Name` que no coincida con el cuerpo lo
rechaza el propio transporte, y la autorización no lo consulta en ningún caso.
Una herramienta fuera de la lista responde `scope_denied`. Así `fetch` —la más
sensible— puede negarse a un cliente concreto sin apagar el egress para todos.

Tool errors are structured with `schema_version: "1"` and a code from the shared
catalog in `amatl-core/src/errors.rs` — the same identifiers the HTTP surface
returns, for example `invalid_query`, `invalid_url`, `egress_denied`,
`fetch_failed`, `search_planning_failed`, `provider_not_registered`,
`inference_unavailable` or `configuration_invalid`. Internal details and provider
credentials are not returned (`amatl-server/src/mcp.rs`).

`evidence_v2` preserves the v1 evidence score and exposes exact byte ranges over
`Document.content`, source/extracted-content hashes and URL/fetch/extractor
provenance. It improves traceability but does not authenticate hostile Internet
content; MCP consumers must not treat fragment text as trusted instructions.

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
| Page size | 100 | 25 |
| `fetch` timeout | 8,000 ms | 3,000 ms |
| `fetch` bytes | 2 MiB | 256 KiB |
| `fetch` redirects | 5 | 2 |

Every row is computed by `ExecutionLimits::for_surface`, including the `fetch`
ones: no tool hardcodes a ceiling, so lowering a value in configuration lowers
it for MCP too. The `status` tool returns the effective numbers, which is the
authoritative answer for a client that needs to size its own requests.

If operator configuration is already lower, MCP does not increase it
(`service.rs:33-58`). The standalone `fetch` tool has its own even smaller
limits shown above and does not consume the Search/Deep Budget. It is therefore
a bounded public-network proxy for authenticated clients; rate limiting and
deployment egress policy remain necessary bajo `standard`. En `isolated` deja
de ser proxy. La política de AMATL no puede demostrar que el cliente MCP que
envía la consulta sea local; para ejercicios confidenciales el cliente/modelo
también debe ejecutarse localmente y el host debe aplicar defensa en profundidad.

The contract test initializes MCP, lists exactly these five tools, asserts that
no ingestion tool exists, and calls `search` and `status`
(`amatl-server/src/tests.rs`).

La ingestión local no es una herramienta MCP ni un endpoint HTTP. Sólo
la CLI acepta rutas del filesystem; esta separación evita que un cliente remoto
con bearer convierta el servidor en lector de archivos locales.
