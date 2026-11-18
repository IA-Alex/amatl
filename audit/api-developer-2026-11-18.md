# api-developer — Revisión de contrato HTTP + MCP (2026-11-18)

Alcance: workspace completo (decisión del usuario); foco en la superficie pública
`amatl-server` (REST/JSON + MCP Streamable HTTP). Modo: **revisión de contrato con
propuesta de ajustes**. Solo lectura; no se ha modificado código.

Fuente de verdad: `crates/amatl-server/src/routes.rs` (inventario canónico) +
`crates/amatl-server/openapi.yaml` (contrato) + `docs/api/mcp.md` y `mcp.rs`.
Verificación dinámica propia: `cargo test --workspace --locked` en verde
(25 suites, 0 fallos; detalle en `audit/baseline-test-2026-11-18.txt`).

═══════════════════════════════════════════════════════════════
API REVIEW — AMATL HTTP + MCP
═══════════════════════════════════════════════════════════════

Resources:       Search, Deep, Answer, Providers, History/Saved, Admin/Operación
Base Path:       / (sin prefijo de versión en URI)
Style:           REST/JSON + MCP Streamable HTTP (JSON-RPC) en /mcp
Versionado:      SemVer del binario + `schema_version` ("1") en los contratos de datos
                 (decisión documentada: la negociación es por ambas, no por path)
Auth:            Bearer token (digest, expiración por credencial, rotación en caliente);
                 múltiples clientes con `token_env` + `Scope` propio

───────────────────────────────────────────────────────────────
ENDPOINTS (36 operaciones, inventario == router por test-guard bidireccional)
───────────────────────────────────────────────────────────────

| Method | Path | Purpose | Auth/Scope | Idempotent | Cacheable |
|--------|------|---------|-----------|------------|-----------|
| GET | /health | Liveness (router arriba) | pública | Sí | No |
| GET | /ready | Readiness (fuentes disponibles/usables) | pública | Sí | No |
| GET | /search | Búsqueda (compat, **deprecated**: preferir POST) | bearer/search | Sí | No |
| POST | /search | Búsqueda con body JSON | bearer/search | Sí | No |
| GET | /search/batch | Multi-consulta (1..=5 `q` repetidos), por ítem independiente | bearer/search | Sí | No |
| POST | /deep | Deep acotado (body o targets de Search) | bearer/deep | Sí | No |
| POST | /answer | Respuesta citada sobre resultados Search | bearer/answer | Sí | No |
| PATCH | /answer | Update parcial de `[answer]` | bearer/admin | No | No |
| POST | /answer/enabled | Toggle de la síntesis (drop-in live) | bearer/admin | Sí* | No |
| POST | /strategist/enabled | Toggle del strategist | bearer/admin | Sí* | No |
| GET | /providers | Resúmenes y capacidades | bearer | Sí | No |
| GET | /providers/{name} | Ficha del provider | bearer | Sí | No |
| PATCH | /providers/{name} | Update de la ficha (gobernanza) | bearer/admin | No | No |
| POST | /providers/{name}/enabled | Toggle del provider | bearer/admin | Sí* | No |
| POST | /providers/{name}/credential | Fijar credencial (nombre de env var) | bearer/admin | No | No |
| DELETE | /providers/{name}/credential | Retirar credencial | bearer/admin | Sí | No |
| GET | /status | Estado agregado (sin internals de ranking) | bearer | Sí | No |
| POST | /reload | Recarga de config en caliente | bearer/admin | Sí* | No |
| GET | /security-events | Eventos de seguridad (+dropped por saturación) | bearer/admin | Sí | No |
| GET | /history | Historial (requiere persistence.enabled) | bearer | Sí | No |
| DELETE | /history | Purga de historial | bearer/admin | Sí | No |
| DELETE | /history/{id} | Borrar entrada | bearer/admin | Sí | No |
| GET | /saved | Documentos guardados | bearer | Sí | No |
| POST | /saved | Guardar documento (IdempotencyKey) | bearer | No | No |
| DELETE | /saved/{id} | Borrar guardado | bearer/admin | Sí | No |
| GET | /metrics | Prometheus text (v0.0.4) | bearer/admin | Sí | No |
| GET | /policies | Políticas (ranking, diversity, search, ranking_v2, gaps) | bearer | Sí | No |
| PATCH | /policies/{name} | Update de política | bearer/admin | No | No |
| GET | /persistence | Config de persistencia | bearer | Sí | No |
| PATCH | /persistence | Update de persistencia | bearer/admin | No | No |
| GET | /persistence/backups | Listado de backups | bearer/admin | Sí | No |
| POST | /persistence/backup | Crear backup | bearer/admin | No | No |
| GET | /circuits | Estado de circuitos | bearer | Sí | No |
| POST | /circuits/reset | Reset de circuitos | bearer/admin | Sí* | No |
| GET | /telemetry | Config de telemetría | bearer | Sí | No |
| PATCH | /telemetry | Update de telemetría | bearer/admin | No | No |
| GET | /deep/limits | Config de Deep | bearer | Sí | No |
| PATCH | /deep/limits | Update de Deep | bearer/admin | No | No |
| PATCH | /deep/extractor | Update de extractor | bearer/admin | No | No |
| PATCH | /deep/renderer | Update de renderer | bearer/admin | No | No |
| GET | /inference | Patch de inferencia remota | bearer/admin | No | No |
| GET/POST | /server/clients | Listar/crear clientes del servidor | bearer/admin | — | No |
| PATCH/DELETE | /server/clients/{id} | Update/borrar cliente | bearer/admin | — | No |
| POST | /server/clients/{id}/rotate | Rotar token en caliente | bearer/admin | No | No |
| GET/PATCH | /server/pending-config | Config pendiente | bearer/admin | — | No |
| POST | /data-policy | Cambiar perfil/egress (fail-closed) | bearer/admin | No | No |

Sí* = idempotente en efecto (toggle/reset con estado estable reportado).
MCP en `/mcp`: tools `search`, `deep_search`, `fetch`, `providers`, `status`,
`answer` (`MCP_TOOLS: [&str; 6]` en `config/server.rs:555`; sin tool de ingestión).

───────────────────────────────────────────────────────────────
REQUEST / RESPONSE — formas dominantes
───────────────────────────────────────────────────────────────

- `ErrorResponse`: `{ schema_version: "1", code: snake_case estable, message }`.
  El `code` enum == `ERROR_CATALOG` de `amatl-core` (guard de igualdad bidireccional
  en `tests.rs:openapi_error_catalog_matches_shared_catalog`).
- `SearchResponse`: `status: success | partial_success | failure`, `results[]`
  (sin `final_url` — Search no expone descargas), `providers_used/failed/partial`,
  `errors[]`, `degradations[]`, `diversity`, `budget_remaining`, paginación
  server-side (`page`, `page_size`, `total_results`), `X-Request-ID` siempre.
- `DeepInput`/`DeepResponse`: documentos parciales + `degradations` + `evidence_v2`
  (rangos de bytes sobre `Document.content`, hashes, provenance).
- Cuerpos administrativos: patch structs snake_case `additionalProperties: false`,
  campo omitido = conservar; string vacío = limpiar (convención `lib.rs`).
- Todos los `components/responses` llevan `X-Request-ID`; el entrante nunca se
  refleja ni se confía en él.

───────────────────────────────────────────────────────────────
ERROR CATALOG (transporte, común a todas las operaciones)
───────────────────────────────────────────────────────────────

| Status | Code | Meaning | Recovery |
|--------|------|---------|----------|
| 400 | invalid_host / bad_request | Host fuera de allowlist o payload inválido | Corregir Host/consulta/body |
| 401 | unauthorized | Bearer ausente, inválido o expirado | Renovar/rotar token |
| 403 | invalid_origin / scope_denied | Origin denegado o scope insuficiente | Ajustar Origin/scope |
| 404 | not_found | Entrada inexistente | Verificar id |
| 409 | conflict | Estado inconsistente (ya existe / no mutable) | Resolver conflicto |
| 413 | body_too_large | Body > 65536 bytes (default) | Reducir body |
| 422 | bad_request (misma forma) | Validación de dominio fallida | Corregir campos |
| 429 | rate_limited | Ventana por IP de socket (X-Forwarded-For ignorado a propósito) | Backoff; `Retry-After: 60` |
| 431 | headers_too_large | Cabeceras > 16384 bytes (default) | Reducir cabeceras |
| 500 | service_unavailable_code | Fallo de planificación/config de servicio | Revisar config/logs |
| 503 | storage_unavailable | Persistencia local deshabilitada/inusable | Activar/repair storage |
| 504 | request_timeout | Deadline > 30000 ms (default) | Reintentar |

Decisión registrada y respetada: **no** se usan problem documents RFC 7807; la
misma forma `ErrorResponse` sirve a CLI/API/MCP/UI con códigos estables
(documentado en la cabecera de `openapi.yaml`). Es una decisión coherente para un
ecosistema de superficies propias; no se propone cambiarla sin acuerdo.

───────────────────────────────────────────────────────────────
MCP — contrato (docs/api/mcp.md, verificado contra mcp.rs y tests)
───────────────────────────────────────────────────────────────

| Tool | Input | Output | Bounds específicos |
|------|-------|--------|--------------------|
| search | `{query, page?, page_size?}` | SearchResponse en structuredContent | query 1–2048 B; page_size clamp MCP |
| deep_search | `{query}` | DeepResponse + evidence_v2 | 8 fragmentos × 512 B/doc; progress + cancel |
| fetch | `{url}` | URL final, status, contenido UTF-8-lossy, tamaño | SSRF SafeFetcher; caps MCP: 3 s, 256 KiB, 2 redirects |
| providers | — | resúmenes | sin salida de red |
| status | — | estado + límites efectivos | sin salida de red |
| answer | `{query}` | respuesta citada | scope `answer`; si no hay backend → `inference_unavailable` |

- Revisión de protocolo: revisiones hasta `2026-07-28`; lifecycle por `_meta`
  (SEP-2567); `initialize` negocia a `2025-11-25`. Stateless, JSON, sin sesión legacy.
- Autorización por identidad autenticada (nunca headers): allowlist `tools` por
  credencial; fuera de lista → `scope_denied`. Antispoofing probado.
- Límites: única fuente `ExecutionLimits::for_surface` (service.rs); MCP toma el
  valor configurado o el cap estricto (tabla verificada en docs; sin magic numbers
  repartidos por los tools).

───────────────────────────────────────────────────────────────
GUARDS DEL CONTRATO (qué está automatizado hoy)
───────────────────────────────────────────────────────────────

| Guard (tests.rs) | Garantiza |
|---|---|
| `openapi_covers_every_router_operation` | cada ruta+método del router está en openapi.yaml |
| `openapi_operations_match_router_inventory` | igualdad exacta en ambos sentidos (y que el parser no sea vacío) |
| `openapi_error_catalog_matches_shared_catalog` | enum `ErrorResponse.code` == `ERROR_CATALOG` bidireccional |
| `openapi_documents_transport_level_rejections` | 400/403/429/431/504 en todas; 401 si protegida; 413 si requestBody |
| `openapi_component_refs_resolve` | sin `$ref` rotos ni componentes huérfanos (responses/parameters) |
| `status_response_matches_the_openapi_service_status_schema` y `admin_responses_match_their_openapi_schemas` | conformidad de payloads reales vs schemas |
| `required_scope_covers_every_registered_route` | cada ruta tiene scope asignado |
| `search_and_deep_use_post_contracts_and_safe_dom_apis` (ui tests) | la UI usa POST y APIs DOM seguras |

Veredicto de contrato: **EXCELENTE / APTO**. Un único punto de verdad por ruta
(macro `define_public_routes!`), guard bidireccional, catálogo compartido, y
rechazos de transporte documentados por operación para clientes generados.

───────────────────────────────────────────────────────────────
HALLAZGOS Y PROPUESTA DE AJUSTES
───────────────────────────────────────────────────────────────

- **[MEDIA · consistencia de diseño] `GET /search/batch` pone hasta 5 consultas
  en el request-target.** Evidencia: `openapi.yaml:85-104` (solo GET, `q`
  repetido 1..=5) frente a `openapi.yaml:38` que deprecata `GET /search` con la
  razón "Prefer POST so query text is not placed in the request target". La misma
  razón aplica al batch y con más fuerza (5 consultas → URLs largas, límites de
  request-target, fugas por logs/proxies/historial). Propuesta (aditiva, sin
  breaking): `POST /search/batch` con body `{queries: string[]}` (1..=5, mismos
  `SearchBatchResponse`), marcar `GET` como `deprecated: true` igual que en
  `/search`, más test de contrato. Alternativa barata: si GET se mantiene
  intencionadamente, documentar la excepción con su justificación.
- **[MEDIA · deriva contrato↔implementación] el 503 de `/ready` está
  infra-documentado.** Evidencia: `tests.rs:6096-6136` devuelve **503 + `status:
  "degraded"`** cuando una fuente `available` tiene el circuito abierto
  (persistencia sana), pero `openapi.yaml:954` → `StorageUnavailable`
  ("Local persistence is disabled or unusable", :1301-1302) es la única causa
  documentada. Un cliente generado no puede modelar la degradación por providers.
  Propuesta: ampliar la descripción de la respuesta 503 de `/ready` (causas:
  storage inusable **o** fuentes degradadas; el campo `sources_usable` es el
  predicado de automation, no `sources_available` — así lo fija el test) o
  renombrar el componente a `Unavailable`/`GatewayDegraded`.
- **[BAJA · docs] ruta del contrato mal referenciada en gobernanza interna.**
  Las memorias de trabajo (`conventions`, `task_completion`) citan
  `docs/api/openapi.yaml`; el contrato vive en `crates/amatl-server/openapi.yaml`
  (justificado en `tests.rs:5695-5698`: `include_str!` no puede escapar del
  paquete; C1 en `docs/security/hallazgos-2026-10-26.md`). Corrigiendo la memoria
  (y cualquier doc que repita la ruta viva) se evita que un cambio futuro toque
  el fichero equivocado.
- **[BAJA · naming] `'500' → responses/ServiceUnavailable`** y **`'422' →
  responses/BadRequest`**: dos status mapeados a componentes con nombre ajeno a
  su código. La semántica es correcta (descripción = "Planning or service
  configuration failure"; 422 comparte forma con 400 a propósito), pero un
  cliente generado expone nombres confusos. Renombrar componentes
  (`PlanningFailure`, `ValidationError`) es breaking solo para el texto del
  contrato, no para el wire format.
- **[MEJORA · tooling] los guards parsean YAML con slicing de strings.**
  Son meticulosos y autovalidados (`openapi_operations_match_router_inventory`
  prueba que el parser no pierde operaciones), pero son sensibles a
  reformateo (forma compacta vs bloque ya se maneja a medias). Recomendación:
  añadir al CI un validador externo (Spectral/Redocly) como complemento — no
  como sustituto, ya que la igualdad con el router solo se puede comprobar en
  los tests.
- **[NOT_A_BUG] RFC 7807 ausente**: decisión explícita y documentada
  (`openapi.yaml:10-16`); mismo catálogo de códigos en superficies propias.
  Correcto tal cual está.

## Seguridad del contrato (lectura api-developer)

- Rate limit por peer address de socket; `X-Forwarded-For` ignorado por diseño
  (anti-evasión documentado en el propio contrato). Correcto tras proxy inverso
  con IP real conectada: la clave de rate es la IP de socket, no el cliente
  original — trade-off consciente, documentado.
- Secretos: el contrato solo habla del *nombre* de variable (`token_env`,
  `credential_env`); el token se compara por digest y rota en caliente. Coherente
  con el invariante "los secretos nunca en amatl.toml".
- `data_policy` `isolated` corta egress antes de conectar también en MCP `fetch`
  (`egress_denied`); no existe bypass por transporte. Coherente entre doc y código.

═══════════════════════════════════════════════════════════════