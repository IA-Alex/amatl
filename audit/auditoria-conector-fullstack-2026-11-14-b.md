# Auditoría conector-fullstack — AMATL

## 1. Alcance y contexto
- **Objetivo:** verificar que cada operación que invocan los conectores (UI web,
  MCP, CLI) tiene contraparte real en el proveedor (servidor Axum sobre
  `amatl-core`) y que el resultado declarado se produce de verdad.
- **Entorno:** Linux local, commit `8dfa6a06`, worktree sucio (`output/`
  sin trackear; los 5 ficheros de la reparación previa ya están commiteados).
- **Stack/topología/transportes:**
  - UI (JS vanilla) → **HTTP/JSON** → Axum (`crates/amatl-server`, un solo
    listener para API + UI + MCP).
  - Cliente MCP → **MCP Streamable HTTP** (`rmcp` 3.10.0) sobre el mismo listener.
  - CLI `amatl` → **en proceso** (llama a `amatl-core` directamente; **no** es un
    conector HTTP). No genera filas de trazabilidad HTTP.
  - Proveedor externo: adapter SearXNG → instancia `searxng/searxng:latest` local.
- **Flujos incluidos:** Search (POST/GET), Search batch, Deep, Answer, providers,
  status, history/saved (con y sin storage), toggles, métricas, MCP
  (initialize/list/call).
- **Flujos excluidos:** `browser_e2e` (requiere WebDriver), soak, canarios
  Marginalia (requieren key real), TUI.
- **Evidencia disponible:** `crates/amatl-server/openapi.yaml` (contrato formal),
  guardas de cobertura OpenAPI en `tests.rs`, servidor real en `127.0.0.1:8891`
  + instancia SearXNG real en `127.0.0.1:8888`.
- **Revisado en:** `8dfa6a06` (worktree: sucio).

## 2. Dictamen
- **Estado: FUNCIONAL-PARCIAL**
- **Resumen:** el núcleo conector↔proveedor funciona y está verificado E2E
  (Search/Deep/batch/providers/status/MCP contra instancia real). No hay rutas
  huérfanas ni desajustes de método. Hay **dos desincronizaciones de contrato
  confirmadas** (el esquema OpenAPI no describe su propia respuesta `/status`, y
  la doc de testing dice 5 herramientas MCP cuando hay 6) y varios cuerpos de
  request/respuesta "sin contrato tipado" en OpenAPI (usan el genérico
  `AdminPatch`/`AdminSuccess`), que es deuda de contrato, no rotura de flujo.
- **Severidad:** crítico 0 · alto 0 · medio 1 · bajo 3
- **Cobertura:** identificados 26 rutas HTTP + 6 tools MCP + N llamadas UI ·
  trazados 22 · contratos 8 · E2E 9 · fallos 4 (0 bloqueantes) · pendientes 4

## 3. Matriz de trazabilidad

| ID | Origen | Transporte/operación | Proveedor | Resultado | Evidencia | Estado |
|---|---|---|---|---|---|---|
| T-01 | `app.js:719` | POST `/search` | `routes.rs:66` → `lib.rs:995` | 200 `SearchResponse` | **E2E**: 5 resultados, `total_results=31`, `degradations=[]` | OK |
| T-02 | `app.js:719` | POST `/deep` | `routes.rs:68` → `lib.rs:1075` | 200 `DeepResponse` | **E2E**: 10 documentos, 10 `evidence_v2` | OK |
| T-03 | `app.js:719` | POST `/answer` | `routes.rs:69` → `lib.rs:1124` | 200 `AnswerResult` | Estático + contrato; backend deshabilitado en la corrida | OK |
| T-04 | — (cliente API) | GET `/search/batch` | `routes.rs:67` → `lib.rs:1044` | 200 `{count,batches}` | **E2E**: `count=2`, ambos `partial_success` | OK |
| T-05 | `app.js:1002` | POST `/providers/{name}/enabled` | `routes.rs:79` → `lib.rs:1310` | 200 `ReloadReport` | Estático (guard Admin `lib.rs:610`) | OK |
| T-06 | `app.js:1064` | POST `/providers/{name}/enabled` | idem T-05 | 200 | Estático | OK |
| T-07 | `app.js:1368` | POST `/answer/enabled` | `routes.rs:77` → `lib.rs:1218` | 200 `ReloadReport` | Estático (guard Admin `lib.rs:604`) | OK |
| T-08 | `app.js:1385` | POST `/strategist/enabled` | `routes.rs:78` → `lib.rs:1266` | 200 `ReloadReport` | Estático + test `tests.rs:2739` | OK |
| T-09 | `app.js:1076` | GET `/providers` | `routes.rs:70` → `lib.rs:1158` | 200 `{providers[]}` | **E2E**: `marginalia/unavailable`, `searxng/available` | OK |
| T-10 | `app.js:1190` | GET `/status` | `routes.rs:71` → `lib.rs:2613` | 200 `ServiceStatus` | **E2E** + **H-01** | DESAJUSTE |
| T-11 | `app.js:1483` | POST `/providers/marginalia/credential` | `routes.rs:80` → `lib.rs:1395` | 200 `ProviderCredentialReport` | Estático (Admin `lib.rs:613`) | OK |
| T-12 | `app.js:1513` | DELETE `/providers/{name}/credential` | `routes.rs:80` → `lib.rs:1468` | 200, idempotente | Estático | OK |
| T-13 | `app.js:1545` | GET `/history?limit=20` | `routes.rs:72` → handler | 200 `{entries[]}` | **E2E**: 503 sin storage | OK (fail-closed) |
| T-14 | `app.js:1577` | DELETE `/history/{id}` | `routes.rs:73` | 200/404 | Estático | OK |
| T-15 | `app.js:1587` | DELETE `/history` | `routes.rs:72` | 200 | Estático | OK |
| T-16 | `app.js:1604` | GET `/saved?limit=20` | `routes.rs:74` | 200 `{documents[]}` | **E2E**: 503 sin storage | OK (fail-closed) |
| T-17 | `app.js:1634` | DELETE `/saved/{id}` | `routes.rs:75` | 200 | Estático | OK |
| T-18 | `app.js:1708` | POST `/saved` | `routes.rs:74` | 200 | **E2E**: 503 sin storage | OK (fail-closed) |
| T-19 | `app.js:1814` | GET `/server/clients` | `routes.rs:83` → `lib.rs:1670` | 200 `{clients[]}` | Estático + **H-03** (schema OpenAPI genérico) | OK (contrato débil) |
| T-20 | `app.js:1886` | POST `/server/clients` | `routes.rs:83` → `lib.rs:1691` | 200 `{...summary,token}` | Estático + **H-03** | OK (contrato débil) |
| T-21 | `app.js:1915` | DELETE `/server/clients/{id}` | `routes.rs:84` | 200 | Estático | OK |
| T-22 | `app.js:1928` | POST `/server/clients/{id}/rotate` | `routes.rs:85` → `lib.rs:1908` | 200 `{...summary,token}` | Estático + **H-03** | OK (contrato débil) |
| T-23 | cliente MCP | POST `/mcp` initialize | `mcp.rs` | 200, `serverInfo.name=amatl` | **E2E**: fallback `2025-11-25` | OK |
| T-24 | cliente MCP | POST `/mcp` tools/list | `mcp.rs:120` | 6 tools | **E2E**: `answer,deep_search,fetch,providers,search,status` | OK |
| T-25 | cliente MCP | POST `/mcp` tools/call search | `mcp.rs:122` | structured `SearchResponse` | **E2E**: `status=partial_success`, 3 resultados | OK |
| T-26 | `lib.rs:958` | GET `/search?q=` | `routes.rs:66` | 400 `invalid_query` | **E2E**: `{}` → `invalid_request` | OK |

## 4. Hallazgos

### H-01 — [MEDIO] El esquema OpenAPI de `/status` no describe su propia respuesta
- **Flujos:** T-10
- **Evidencia:** **E2E** `GET /status` real (commit `8dfa6a06`) devuelve las claves
  `['answer','cache','data_policy','inference_backend','schema_version','sources',
  'status','storage','strategist']`, mientras `openapi.yaml` define `ServiceStatus`
  con `additionalProperties: false` y `required: [schema_version, status, sources,
  storage, cache, inference_backend, answer]` — **sin `strategist` ni
  `data_policy`** (`crates/amatl-server/openapi.yaml:1749-1763`). El servidor
  emite ambos campos (`crates/amatl-core/src/service.rs:332,337`) y un test lo
  confirma (`tests.rs:2773-2774` lee `status["strategist"]["enabled"]`).
- **Síntoma:** un cliente que valide `/status` contra el contrato declarado
  **rechaza una respuesta válida del propio servidor**.
- **Causa:** **CONFIRMADO** — el esquema quedó atrás al añadirse `strategist` y
  `data_policy` a `ServiceStatus`; la guarda `openapi_covers_every_router_operation`
  solo verifica rutas/métodos, no la forma del cuerpo de respuesta
  (`tests.rs:5167`, `tests.rs:5373`).
- **Impacto:** consumidores estrictos (generación de SDK, validación en CI de
  terceros) se rompen; la doc de la API miente por omisión.
- **Corrección mínima propuesta:** en `openapi.yaml:1749-1763`, añadir a
  `properties` `strategist: { $ref: '#/components/schemas/StrategistStatus' }` y
  `data_policy: { $ref: '#/components/schemas/DataPolicyStatus' }`, añadirlos a
  `required`, y declarar ambos esquemas a partir de `service.rs:341-400`. Añadir
  una guarda de conformidad de respuesta (o, si el coste es alto, un test que
  compare las claves de `ServiceStatus` serializado contra el `required`+`properties`).
- **Regresión recomendada:** test que serialice un `ServiceStatus` de ejemplo y
  verifique que cada clave está declarada en el esquema OpenAPI.

### H-02 — [BAJO] `docs/testing.md` declara cinco herramientas MCP; hay seis
- **Flujos:** T-24
- **Evidencia:** **E2E** `tools/list` → `answer, deep_search, fetch, providers,
  search, status` (6). El test `tests.rs:1232-1243` **sí** afirma las seis.
  `docs/testing.md:37` dice "Exactly five tools covered".
- **Síntoma:** documentación de cobertura desactualizada (la herramienta `answer`
  se añadió después).
- **Causa:** **CONFIRMADO** — texto de doc no actualizado al añadir `answer`.
- **Impacto:** un lector (o auditor externo) cree que `answer` no está cubierta
  por contrato. No afecta runtime.
- **Corrección mínima propuesta:** `docs/testing.md:37`: "Exactly five tools" →
  "Exactly six tools (`answer`, `deep_search`, `fetch`, `providers`, `search`,
  `status`)".
- **Regresión recomendada:** ninguna de código; opcionalmente un chequeo de doc.

### H-03 — [BAJO] Cuerpos de request/respuesta de admin sin esquema tipado en OpenAPI
- **Flujos:** T-19, T-20, T-22 (y `/providers/{name}/credential`, `/providers/{name}/enabled`)
- **Evidencia:** **ESTÁTICO** `openapi.yaml:1059-1075` (`AdminPatch`) y `:1069-1075`
  (`AdminSuccess`) se reutilizan como cuerpos genéricos, de modo que campos
  concretos (`enabled`, `key`, `{clients[],token}`) no se fijan. Los structs reales
  sí los definen (`lib.rs:1294-1297` `ProviderToggleParams.enabled`,
  `lib.rs:1349-1352` `ProviderCredentialParams.key`, `lib.rs:1673-1681`
  `CreateServerClientParams`).
- **Síntoma:** el contrato documentado no restringe esos cuerpos.
- **Causa:** **CONFIRMADO** — uso deliberado del genérico para no duplicar esquemas.
- **Impacto:** clientes generados desde OpenAPI no validan tipos en esos endpoints;
  riesgo de drift silencioso. No rompe el flujo (verificado E2E en T-09/T-11).
- **Corrección mínima propuesta:** declarar esquemas concretos
  (`ProviderToggleInput`, `ProviderCredentialInput`, `ServerClientInput`,
  `ServerClientList`, `CreatedServerClient`) y referenciarlos.
- **Regresión recomendada:** extender la guarda OpenAPI para exigir esquema
  específico en cuerpos admin con campos conocidos.

### H-04 — [BAJO] `/status` no documenta `strategist` en la UI pero la UI lo lee (acoplamiento con H-01)
- **Flujos:** T-10
- **Evidencia:** **ESTÁTICO** `app.js:1282-1320` lee
  `payload.strategist.{configured,available,enabled}`; el esquema OpenAPI (H-01)
  no lo declara.
- **Síntoma:** contradicción entre lo que la UI consume y lo que el contrato
  publica.
- **Causa:** **CONFIRMADO** — misma causa raíz que H-01 (schema sin actualizar).
- **Impacto:** refuerza H-01; un tercero que valide el contrato pensaría que la
  UI lee un campo inexistente.
- **Corrección mínima propuesta:** cubierto por H-01.
- **Regresión recomendada:** la de H-01.

## 5. Operaciones sin correspondencia
- **Cliente huérfano:** ninguno. Las 22 operaciones HTTP de la UI (subagente de
  inventario, `app.js` + `client.js`) mapean a rutas declaradas en `routes.rs:66-101`.
- **Proveedor huérfano (sin consumidor en este repo):** varias rutas de
  configuración avanzada (`/reload`, `/data-policy`, `/policies`, `/persistence*`,
  `/circuits*`, `/telemetry`, `/deep/limits|extractor|renderer`, `/security-events`,
  `/inference`, `/server/pending-config`) **no** las invoca la UI. *No* son
  huérfanas reales: son superficie de operador (curl/CLI/automatización) y/o
  consumidas por pruebas. Se listan como *consumidor externo/local*, no como defecto.
- **Nota de método:** el CLI no es conector HTTP (llama a `amatl-core` en proceso),
  así que no aporta filas a esta matriz.

## 6. No verificado y límites
- **`/answer` POST con backend real:** en la corrida el backend estaba deshabilitado
  (`answer.available=false`). Verificado a nivel de contrato y estático; no se
  observó una síntesis real. Procedimiento: configurar `[answer]` con endpoint
  válido y repetir T-03.
- **`browser_e2e`:** requiere un WebDriver vivo; no ejecutado. Es la única
  evidencia que probaría la UI **renderizada** de punta a punta.
- **`/providers/{name}/credential` canary real:** el POST dispara una prueba de
  proveedor contra la red; no se ejecutó para Marginalia (key real). Procedimiento:
  usar una key vigente en un entorno aislado.
- **Soak/load:** `--ignored`, no ejecutado (fuera de alcance).
- **Marginalia:** `enabled=false`; sus flujos no se ejercieron en vivo.

## 7. Plan de corrección
| Orden | Hallazgo | Dependencia | Cambio mínimo | Verificación de cierre |
|---|---|---|---|---|
| 1 | H-01 | — | Añadir `strategist` + `data_policy` a `ServiceStatus` en `openapi.yaml` | Test de conformidad de respuesta de `/status` vs esquema |
| 2 | H-04 | H-01 | (cubierto por H-01) | idem H-01 |
| 3 | H-02 | — | Corregir `docs/testing.md:37` (cinco→seis) | Revisión de doc |
| 4 | H-03 | — | Esquemas concretos para cuerpos admin | Guarda OpenAPI extendida |

### Nota de entorno (no es hallazgo)
`cargo test -p amatl-server` falló al enlazar con
`undefined hidden symbol: <http::request::Parts as rmcp::model::extension::AnyClone>::into_any`.
Tras `cargo clean -p amatl-server` el enlace y `cargo test -p amatl-server`
pasaron (todas las suites `ok`). Es un **artefacto incremental obsoleto**, no un
defecto de fuente: `cargo test --workspace --locked` siempre pasó (717/0/2).
Clasificado como **ENVIRONMENT_FAILURE** (caché de build), reproducible solo con
el artefacto previo.
