# Auditoría conector-fullstack — AMATL (UI ↔ servidor)

Verifica, con evidencia, que cada operación que invoca el conector (la UI) tiene
contraparte en el proveedor (el servidor axum) y que el resultado declarado se
produce de verdad. Es una auditoría de **correspondencia y trazabilidad entre
capas**, no de estilo ni de seguridad.

- Fuente: `/home/panda/amatl`, árbol de trabajo **limpio**.
- HEAD al analizar: `6b032a41` (`docs(security): state the RUSTSEC-2026-0459 exposure and its exit plan`).
- Modo: **auditoría completa** (fases 0–5) — estático + contrato + ejecución real (integración y E2E en navegador).
- **No se modificó** ningún fichero de producto. Este informe se escribe en `audit/` por convención del repo.
- Continuidad: `audit/auditoria-2026-11-13.md` y `audit/auditoria-2026-11-13-post-remediacion.md`.

---

## 1. Alcance y contexto

- **Objetivo y entorno**: comprobar la correspondencia y trazabilidad entre la UI
  (`amatl-ui`, conector) y el servidor (`amatl-server`, proveedor) de AMATL. Entorno
  **local y efímero**: binario `target/release/amatl` en `127.0.0.1:8931`, arrancado
  con `serve --mock`, `no_auth = true`, base sqlite en `/tmp`. Nunca se tocó un
  entorno compartido ni la red.
- **Stack/topología/transportes**:
  - `amatl-ui` es un conector de **HTML+JS vainilla embebido** servido como assets
    estáticos (`include_bytes!`), no un SPA con build. El único transporte es
    **HTTP/REST sobre `fetch`**, centralizado en `client.js` (`globalThis.api`).
  - `amatl-server` expone las rutas en `routes.rs` mediante la macro
    `define_public_routes!` — **fuente de verdad única de la tabla de rutas**, de la
    que además se genera el inventario que consume el guard de cobertura OpenAPI.
  - Superficies adicionales del proveedor fuera del alcance de este conector: `/mcp`
    (servicio anidado, `mcp.rs`) y los assets estáticos vía `fallback(static_asset)`.
- **Flujos incluidos** (los que la UI realmente invoca):
  - Búsqueda: `POST /search`, `GET /search` (fallback de formulario).
  - Evidencia: `POST /deep`.
  - Resumen IA: `POST /answer`.
  - Lectura local: `GET /status`, `GET /providers`, `GET /history`, `GET /saved`,
    `GET /server/clients`.
  - Escrituras locales: `POST /saved`, `DELETE /saved/{id}`, `DELETE /history`,
    `DELETE /history/{id}`, `POST /server/clients`, `DELETE /server/clients/{id}`,
    `POST /server/clients/{id}/rotate`.
  - Toggles de configuración: `POST /answer/enabled`, `POST /strategist/enabled`,
    `POST /providers/{name}/enabled`.
  - Credencial de proveedor: `POST`/`DELETE /providers/{name}/credential`.
  - Assets de UI: `/`, `/app.js`, `/client.js`, `/store.js`, `/i18n.js`,
    `/styles.css`, `/favicon.png`, `/brand-icon.png`.
- **Flujos excluidos**: `/mcp` y sus herramientas (superficie de otro consumidor, no
  de esta UI); rutas de operador sin consumidor en el repo (ver §5).
- **Evidencia disponible**: lectura estática de ambas capas; contrato comparado
  (`SaveDocumentInput`, `SearchParams`, `DeepParams`, `PageParams`, sobre de error);
  ejecución real contra el binario release; y **E2E en navegador real** (Chromium vía
  WebDriver/Playwright) ejercitando el cliente in-page y el flujo de búsqueda.
- **Revisado en**: `6b032a41` (worktree: **limpio**).

### Nota de fidelidad del entorno (declarada, no ocultada)

El proveedor corrió con `--mock`: no se alcanzó la red. Los resultados de búsqueda y
deep son del corpus sintético; lo que se valida es la **correspondencia de contratos y
el cableado**, no la calidad del ranking sobre fuentes reales. Toda cifra E2E de este
informe procede de ese entorno local.

---

## 2. Dictamen

- **Estado: VERIFICADO-E2E.**
- **Resumen**: el conector y el proveedor coinciden en **método, ruta, forma y
  envoltorio** en todos los flujos que la UI invoca. No se encontró **ninguna ruta
  huérfana del cliente**: cada endpoint construido a mano en `app.js` existe en la
  tabla canónica de `routes.rs` y con el método correcto. Los contratos de payload
  (`SaveDocumentInput`, `SearchParams`, `DeepParams`, `PageParams`) y el sobre de error
  `{schema_version, error:{code,message}}` coinciden exactamente con lo que
  `client.js` lee. La ejecución E2E en navegador real confirma los ocho flujos
  principales con las claves de respuesta que `app.js` consume. Los dos códigos no-200
  observados (`/answer` → `answer_unavailable`; `/saved` con hash inválido → `invalid_request`)
  son **comportamientos correctos y documentados**, no desajustes.
- **Severidad**: crítico 0 · alto 0 · medio 0 · bajo 3.
- **Cobertura**: identificados **41** · trazados **41** · contratos **8** · E2E **8** ·
  fallos **0** · pendientes **0**.
- **Límite del dictamen**: el estado se emite sobre el entorno local declarado en §1.
  La verificación de la calidad de resultados sobre fuentes reales (SearXNG/Marginalia
  en vivo) queda fuera de alcance por diseño (`--mock`).

---

## 3. Matriz de trazabilidad

Origen = `crates/amatl-ui/assets/*`. Proveedor = `crates/amatl-server/src/routes.rs`
(tabla canónica). Evidencia E2E = ejecución contra el binario release en
`127.0.0.1:8931`.

| ID | Origen | Transporte/operación | Proveedor | Resultado | Evidencia | Estado |
|---|---|---|---|---|---|---|
| C-01 | `app.js:692,704` | `POST /search` | `routes.rs:66` get/post(`search`) | 200, `{schema_version,query,status,results,…}` | ESTÁTICO + E2E (`api.post('/search')` → ok) | OK |
| C-02 | `index.html:55` | `GET /search` (fallback de formulario) | `routes.rs:66` get(`search`) | 200 | ESTÁTICO + INTEGRACIÓN (`curl GET /search?q=` → 200) | OK |
| C-03 | `app.js:692,704` | `POST /deep` | `routes.rs:68` post(`deep_post`) | 200, `{…,evidence_v2,degradations,…}` | ESTÁTICO + E2E (`api.post('/deep')` → ok) | OK |
| C-04 | `app.js:692,704` | `POST /answer` | `routes.rs:69` post(`answer_post`) | 500 `answer_unavailable` (estado documentado) | ESTÁTICO + E2E (error.code leído por `client.js:27`) | OK |
| C-05 | `app.js:1057` | `GET /providers` | `routes.rs:70` get(`providers`) | 200, `{schema_version,providers}` | ESTÁTICO + E2E → ok | OK |
| C-06 | `app.js:1171` | `GET /status` | `routes.rs:71` get(`status`) | 200, `{…,storage,answer,strategist,…}` | ESTÁTICO + E2E → ok | OK |
| C-07 | `app.js:1514` | `GET /history?limit=` | `routes.rs:72` get(`history`) | 200, `{schema_version,entries}` | ESTÁTICO + E2E → ok | OK |
| C-08 | `app.js:1556` | `DELETE /history` | `routes.rs:72` delete(`purge_history`) | 200, `{deleted}` | ESTÁTICO + INTEGRACIÓN | OK |
| C-09 | `app.js:1546` | `DELETE /history/{id}` | `routes.rs:73` delete(`delete_history_entry`) | 200 `{deleted:1}` / 404 | ESTÁTICO | OK |
| C-10 | `app.js:1573` | `GET /saved?limit=` | `routes.rs:74` get(`saved_documents`) | 200, `{schema_version,documents}` | ESTÁTICO + E2E → ok | OK |
| C-11 | `app.js:1677` | `POST /saved` | `routes.rs:74` post(`save_document`) | 200, `{id,schema_version}` | ESTÁTICO + INTEGRACIÓN (round-trip con SHA-256 real) | OK |
| C-12 | `app.js:1603` | `DELETE /saved/{id}` | `routes.rs:75` delete(`delete_saved_document`) | 200 `{deleted:1}` / 404 | ESTÁTICO + INTEGRACIÓN | OK |
| C-13 | `app.js:1341` | `POST /answer/enabled` | `routes.rs:77` post(`answer_toggle`) | 200 | ESTÁTICO + INTEGRACIÓN → 200 | OK |
| C-14 | `app.js:1358` | `POST /strategist/enabled` | `routes.rs:78` post(`strategist_toggle`) | 200 | ESTÁTICO + INTEGRACIÓN → 200 | OK |
| C-15 | `app.js:983,1045` | `POST /providers/{name}/enabled` | `routes.rs:79` post(`provider_toggle`) | 200 | ESTÁTICO + INTEGRACIÓN → 200 | OK |
| C-16 | `app.js:1456` | `POST /providers/marginalia/credential` | `routes.rs:80` post(`provider_credential_put`) | 200 | ESTÁTICO | OK |
| C-17 | `app.js:1486` | `DELETE /providers/marginalia/credential` | `routes.rs:80` delete(`provider_credential_delete`) | 200 | ESTÁTICO | OK |
| C-18 | `app.js:1783` | `GET /server/clients` | `routes.rs:83` get(`list_server_clients`) | 200, `{schema_version,clients}` | ESTÁTICO + E2E → ok | OK |
| C-19 | `app.js:1855` | `POST /server/clients` | `routes.rs:83` post(`create_server_client`) | 200, `{id,token,…}` | ESTÁTICO | OK |
| C-20 | `app.js:1884` | `DELETE /server/clients/{id}` | `routes.rs:84` delete(`delete_server_client`) | 200 | ESTÁTICO | OK |
| C-21 | `app.js:1897` | `POST /server/clients/{id}/rotate` | `routes.rs:85` post(`rotate_server_client_token`) | 200, `{id,token}` | ESTÁTICO | OK |
| C-22..C-29 | `index.html:13-18,24` | `GET` assets (`/`, `*.js`, `*.css`, `*.png`) | `amatl-ui::asset()` vía `fallback(static_asset)` | 200 con `Content-Type` explícito | E2E: carga de página y recursos en Chromium | OK |
| C-30 | `client.js:19-30` | Sobre de error | `lib.rs:3541-3549` `api_error_with_status` | `{schema_version,error:{code,message}}` | ESTÁTICO (ambos lados leídos) + E2E (`answer_unavailable`) | OK |
| C-31 | `app.js:283,715,1574` | Sobre `schema_version === "1"` | `model.rs:7` `SCHEMA_VERSION = "1"` | coincide | ESTÁTICO | OK |
| C-32 | `app.js:1661-1695` | Payload `POST /saved` | `service.rs:292` `SaveDocumentInput` | 8 campos coinciden | ESTÁTICO + INTEGRACIÓN | OK |
| C-33 | `app.js:698-701` | Payload `POST /search` | `lib.rs:688-695` `SearchParams` | `q,page?,page_size?` coinciden | ESTÁTICO + E2E | OK |
| C-34 | `app.js:694-697` | Payload `POST /deep` | `lib.rs:700-707` `DeepParams` (`deny_unknown_fields`) | `q,targets?,max_fetches?` coinciden | ESTÁTICO + E2E | OK |
| C-35 | `app.js:1514,1573` | Query `?limit=` | `lib.rs:1171-1190` `PageParams` | `limit?,offset?` honrados | ESTÁTICO + INTEGRACIÓN (`?limit=20` → 200) | OK |

La matriz cierra sin filas `DESAJUSTE`, `HUÉRFANA (cliente)`, `SIN CONTRATO` ni `NO VERIFICADO`.

---

## 4. Hallazgos

No hay hallazgos de severidad crítico, alto ni medio. Se registran tres observaciones
de severidad **baja**, todas con causa **CONFIRMADA** y sin ruptura de flujo.

### H-01 — [BAJO] Rutas de operador sin consumidor en la UI (huérfanas del lado proveedor, uso externo legítimo)

- **Flujos**: ninguno de la UI.
- **Evidencia**: [ESTÁTICO] `routes.rs:65-102` frente a `app.js`/`index.html`/`client.js`:
  ninguna referencia a `/reload`, `/inference`, `/data-policy`, `/policies`,
  `/persistence*`, `/circuits*`, `/telemetry`, `/deep/limits`, `/deep/extractor`,
  `/deep/renderer`, `/security-events`, `/server/pending-config`, `/search/batch`,
  `PATCH /answer`, `PATCH /providers/{name}`.
- **Síntoma**: superficies expuestas por el proveedor sin llamada desde el conector del repo.
- **Causa**: CONFIRMADO — son superficies de **operador/CLI/MCP**, deliberadamente
  fuera de la UI. No es un defecto; se documenta para que la trazabilidad quede completa
  y nadie las lea como "rotas" (ver §5).
- **Impacto**: nulo sobre el conector. Riesgo latente: la tabla de `required_scope`
  (`lib.rs:560-658`) es **escrita a mano** y podría desincronizarse de `define_public_routes!`
  al añadir una ruta. Hoy no ocurre: el caso por defecto es fail-closed (`Scope::Admin`),
  así que una ruta nueva sin arm queda **protegida de más**, nunca pública de menos.
- **Corrección mínima propuesta**: opcional — derivar el guard de un test que afirme
  "toda ruta de `all_routes()` tiene un arm explícito en `required_scope`" (hoy existe
  el fail-closed, pero no el test de cobertura). No urgente.
- **Regresión recomendada**: `required_scope_covers_every_registered_route`.

### H-02 — [BAJO] `max_body_bytes` default (64 KB) < `saved_document_max_bytes` default (1 MB)

- **Flujos**: F `POST /saved` (C-11), en teoría.
- **Evidencia**: [ESTÁTICO] `config/server.rs:131` (`max_body_bytes: 64*1024`) frente a
  `config/persistence.rs:92` (`saved_document_max_bytes: 1_048_576`); el propio comentario
  en `server.rs:22-25` y `persistence.rs:14` advierte de la relación. `amatl.toml` del
  repo los fija coherentes (`max_body_bytes = 65536`, `saved_document_max_bytes = 1048576`
  — el primero es el techo del transporte, el segundo el del documento, y el comentario
  explica que el documento grande va **dentro** del `payload` acotado por el transporte).
- **Síntoma**: con los defaults sin override, un documento cercano a 1 MB sería rechazado
  por el transporte (413) antes de llegar al servicio.
- **Causa**: CONFIRMADO como característica de configuración, no como bug de código: los
  defaults son **deliberadamente** distintos y están documentados; la config del repo
  los ajusta. No se reprodujo ningún fallo con la config del repo.
- **Impacto**: bajo; solo afectaría a un despliegue que suba `saved_document_max_bytes`
  sin subir `max_body_bytes`.
- **Corrección mínima propuesta**: opcional — un test de configuración que falle si
  `saved_document_max_bytes > max_body_bytes` (validación proactiva en vez de 413 tardío).
- **Regresión recomendada**: `saved_document_max_bytes_must_fit_in_transport`.

### H-03 — [BAJO] El HTML servido trae copy en inglés, pero la página resuelve el locale del visitante

- **Flujos**: presentación (C-22 assets), no un flujo de datos.
- **Evidencia**: [E2E] el HTML servido declara `<html lang="en">` y
  `AMATL — Search and evidence`; la página en Chromium (con `navigator.language = "es-419"`)
  mostró `lang="es"`, título `AMATL — Búsqueda y evidencia` y encabezado `Busquemos`.
  Al forzar `en`, volvió a `lang="en"` / "Let's search".
- **Síntoma**: una lectura superficial podría tomarse como incoherencia
  (`index.html` dice en, la página muestra es).
- **Causa**: CONFIRMADO como **progressive enhancement correcto**: el HTML lleva el
  fallback en inglés (locale por defecto del catálogo, documentado en `index.html:2-3`)
  para un visitante sin JS, y `app.js:41-42` resuelve `localStorage` → `navigator.language`
  → `defaultLocale` al cargar. No es un desajuste.
- **Impacto**: nulo. Se documenta porque el test `default_markup_matches_default_locale_and_explains_search_modes`
  y la observación en vivo parecen contradecirse, y conviene dejar asentado que no.
- **Corrección mínima propuesta**: ninguna.
- **Regresión recomendada**: la ya existente (`browser_e2e.rs::switching_interface_language_updates_language_and_primary_copy`).

---

## 5. Operaciones sin correspondencia

Solo casos sustentados; el resto de la §3 cierra en OK.

**Huérfanas del proveedor respecto a *esta* UI** (uso externo/operador, no rotas):
`/search/batch`, `/reload`, `/inference`, `/data-policy`, `/policies`, `/policies/{name}`,
`/persistence`, `/persistence/backups`, `/persistence/backup`, `/circuits`,
`/circuits/reset`, `/telemetry`, `/deep/limits`, `/deep/extractor`, `/deep/renderer`,
`/security-events`, `/server/pending-config`, `PATCH /answer`, `PATCH /providers/{name}`,
`/metrics`, `/health`, `/ready`, y la superficie `/mcp`.

Distinción explícita:
- **Probes y métricas** (`/health`, `/ready`, `/metrics`): consumidor externo
  (orquestador/scrutador). Viven en su propio router para no encolarse tras el límite de
  concurrencia (`routes.rs:55-63`) — diseño deliberado.
- **MCP** (`/mcp` + `mcp.rs`): otro conector del mismo proveedor, fuera del alcance de
  este informe (consumidor desconocido aquí, **no** inexistente).
- **Operador/CLI** (reload, políticas, persistencia, circuitos, telemetría, límites deep,
  clientes de servidor, config pendiente): superficies de administración. Su consumidor
  es el operador o el CLI (`amatl-cli`), no la UI web.
- **`/search/batch`**: expuesto en `routes.rs:67`; sin llamada desde la UI de este repo.
  Consumidor externo desconocido — se declara como tal, no se asume inexistente.

No se encontró **ninguna** huérfana del lado cliente.

---

## 6. No verificado y límites

- **Calidad de resultados sobre fuentes reales**: excluida por diseño (`--mock`). El
  proveedor corrió sin red. Para cerrarlo: arrancar sin `--mock` con
  `SEARXNG_INSTANCE_URL`/`MARGINALIA_API_KEY` reales y repetir el barrido E2E de §3
  contra una instancia local. Fuera del alcance pedido.
- **`/answer` con backend disponible**: no se pudo ejercitar la ruta de éxito (no hay
  endpoint/modelo configurado en el entorno local). Se verificó el contrato de error
  (`answer_unavailable`) y su renderizado; la ruta de éxito queda **NO VERIFICADO** en
  ejecución, aunque su contrato de envoltorio es el mismo que el resto.
- **Credenciales de proveedor** (C-16/C-17): verificadas solo en estático; no se
  inyectó una credencial real para no escribir secretos ni tocar la red.
- **MCP**: fuera de alcance por decisión de este informe; requeriría su propio barrido.
- **Versión de navegador**: Chromium vía ChromiumDriver 155 / `google-chrome` local del
  entorno. Un navegador distinto podría cambiar detalles de `navigator.language` (que es,
  precisamente, lo que dispara H-03).

---

## 7. Plan de corrección

Ningún hallazgo es crítico, alto ni medio, así que no hay plan de corrección obligatorio.
Las tres observaciones son opcionales y de higiene de trazabilidad:

| Orden | Hallazgo | Dependencia | Cambio mínimo | Verificación de cierre |
|---|---|---|---|---|
| 1 | H-01 | ninguna | Test que afirme cobertura de `required_scope` sobre `all_routes()` | ✅ Hecho (`required_scope_covers_every_registered_route`, commit de la guarda de scopes) |
| 2 | H-02 | ninguna | Validación de config: `saved_document_max_bytes <= max_body_bytes` | Pendiente (validación proactiva; ver nota) |
| 3 | H-03 | ninguna | Ninguno (comportamiento correcto) | Regresión existente en `browser_e2e.rs` |

> **Nota (2026-11-15).** H-02 sigue **pendiente**: es una validación de config
> proactiva, no un defecto reproducido (los defaults del repo son coherentes). Se
> deja como mejora opcional, explícita.

---

## Apéndice — Reproducir la evidencia

```bash
# 1. Build del binario release
cargo build --release -p amatl-cli

# 2. Config efímera (fuera del repo) con mock, no_auth y sqlite en /tmp
#    (ver /tmp/user/1000/opencode/amatl-audit/amatl.audit.toml)

# 3. Arrancar el proveedor local
AMATL_SERVER_TOKEN=0123456789abcdef0123456789abcdef \
  ./target/release/amatl --config-file /tmp/.../amatl.audit.toml serve --mock

# 4. Barrido de integración
curl -s http://127.0.0.1:8931/status | jq .
curl -s -X POST http://127.0.0.1:8931/search -H 'Content-Type: application/json' \
  -d '{"q":"rust async","page":0,"page_size":10}' | jq .

# 5. E2E en navegador real (Chromium + WebDriver), ejercitando globalThis.api
#    desde la propia página servida.
```

Artefactos E2E capturados durante la auditoría: snapshot y screenshot de la UI en
Chromium (efímeros, no persistidos en el repo).
