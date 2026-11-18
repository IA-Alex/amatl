# Auditoría conector-fullstack — AMATL

Continuación de `audit/auditoria-conector-fullstack-2026-11-14.md`
(cierre `VERIFICADO-E2E` en `6b032a41`) y de su re-auditoría incremental
(`audit/reauditoria-conector-fullstack-incremental-2026-11-14.md`, delta hasta
`6170d7ef`). Esta corrida audita el tramo `6170d7ef..HEAD`, cuyo cambio de
producto dominante es la **retirada de SearXNG** (`bb23a9fc`) y sus
consecuencias en la UI (`app.js`, `i18n.js`, `index.html`, `styles.css`).

Es una auditoría de **correspondencia y trazabilidad entre capas**, no de estilo
ni de seguridad. Regla aplicada: **sin evidencia no hay hallazgo**.

- Fuente: `/home/panda/amatl`. HEAD al analizar: `d34d2cd6`
  (`fix(browser-e2e): drive the pinned Chrome, not the runner's preinstalled one`).
- Delta auditado: `6170d7ef..d34d2cd6` (22 commits). En producto: retirada de
  SearXNG, endurecimiento de clasificación de errores en la UI, contrato de
  tools MCP al crear clientes, validación de config y varias pruebas.
- Modo: **auditoría completa** (fases 0–5), con ejecución real (integración +
  E2E en navegador Chromium vía CDP).
- **No se modificó** ningún fichero de producto. Este informe se escribe en
  `audit/` por convención del repo.
- Transportes cubiertos: **HTTP/REST** UI↔servidor (conector principal) y el
  **borde cliente→proveedor externo** (`amatl-core::providers`), que la retirada
  de SearXNG pone en primer plano.

---

## 1. Alcance y contexto

- **Objetivo**: verificar que cada operación que invoca el conector tiene
  contraparte real en el proveedor y produce de verdad el resultado declarado,
  **tras la retirada de SearXNG**, y que esa retirada no dejó referencias
  huérfanas ni rutas/consultas muertas en el cliente.

- **Stack/topología/transportes**:
  - `amatl-ui` — conector **HTML+JS vainilla embebido** (`include_bytes!`),
    servido como assets. Único transporte: **HTTP/REST sobre `fetch`**,
    centralizado en `client.js` (`globalThis.api`). Sin base-URL configurable:
    las rutas son literales relativos (18 llamadas, todas a mano).
  - `amatl-server` — proveedor axum. Tabla canónica de rutas en
    `crates/amatl-server/src/routes.rs` (`define_public_routes!`), **sin cambios
    en este delta** (`git diff 6170d7ef..HEAD -- routes.rs` vacío), por lo que la
    matriz de rutas del informe previo sigue vigente. `/mcp` se anida aparte
    (`lib.rs:827`).
  - **Borde proveedor externo**: `amatl-core/src/providers/registry.rs`
    (`ProviderRegistry::builtin()` registra **una** factoría: Marginalia),
    `config/providers.rs::builtin_provider_records()` (un solo expediente:
    Marginalia). `select_providers()` (`service.rs:2053`) es la frontera que
    exige registro + declaración + aprobación.

- **Flujos incluidos**:
  - UI→servidor: los 18 endpoints que `app.js` invoca (búsqueda, deep, answer,
    providers, status, historial, guardados, clientes de servidor, toggles,
    credencial).
  - Operador→producto: carga y validación de `amatl.toml` (documentado como el
    archivo de trabajo por defecto: `README.md:69`, `start.sh:14`,
    `docs/operacion.md:11`).
  - Cliente→proveedor externo: selección de fuentes y elegibilidad tras la
    retirada.

- **Evidencia disponible**: estática en ambos lados; contrato comparado
  (`client.js` ↔ `lib.rs` ↔ `openapi.yaml`); ejecución local del binario release
  en `127.0.0.1` (`--mock` y sin `--mock`); y **E2E en navegador real**
  (Chromium headless vía CDP, sin WebDriver).

- **Revisado en**: `d34d2cd6` (worktree: **sucio** — solo artefactos de
  auditoría sin seguimiento en `audit/` y `analiza/`; ficheros de producto
  limpios, `git status --porcelain -- crates/` vacío).

### Nota de fidelidad del entorno

- El servidor corrió **local y efímero** (puertos 8951/8952), nunca red
  compartida ni producción. `--mock` no alcanza la red: valida correspondencia de
  contratos y cableado, no la calidad del ranking sobre fuentes reales.
- La reproducción clave del hallazgo H-01 **no depende de la red**: el fallo
  ocurre en `select_providers()` antes de conectar (y sin `--mock`), y se
  reprodujo con el CLI en proceso (`amatl search`).
- Chrome disponible en el host: 158.0.8127.36. ChromeDriver instalado: 157
  (desajuste; por eso esta corrida usa **CDP directo**, no WebDriver).

---

## 2. Dictamen

- **Estado: FALLA-CONFIRMADA.**
- **Resumen**:
  - **Lo que funciona**: la retirada de SearXNG es **correcta y coherente** en el
    producto versionado. La tabla de rutas no cambió (matriz del informe previo
    aún válida); `amatl.example.toml` ya lista solo Marginalia; `openapi.yaml` no
    menciona SearXNG; la UI cerró bien la superficie retirada en casi todo su
    perímetro; el contrato de `tools` MCP al crear clientes (`app.js:1787`
    `MCP_TOOL_NAMES`) coincide **exactamente** con `config::server::MCP_TOOLS`
    (`crates/amatl-core/src/config/server.rs:555-562`) y con los handlers de
    `mcp.rs` — de hecho el delta **corrige** el antiguo `tools: []`, que negaba
    todo tool; y la nueva clasificación de errores de la UI (`error.status`/
    `error.code`) es **contractual**: `client.js:27-28` sí los setea.
  - **Lo que falla**:
    1. **H-01 (alto)** — `app.js:1981-1984` conserva una consulta a
       `.provider-selector-option input[value="all"]`, un elemento **eliminado**
       con el selector de fuentes. `document.querySelector(...)` devuelve `null`
       y `.closest("label")` lanza `TypeError` **en cada carga de página**,
       abortando `applyTranslations()` antes de `updateFiltersSummary()`,
       `updateAccessSummary()` y `updateThemeToggle()`.
    2. **H-02 (crítico)** — `amatl.toml` del repo (no versionado, pero el archivo
       de trabajo por defecto) mantiene `enabled = ["searxng"]` y un expediente
       `[providers.searxng]`, mientras el producto ya no registra esa fuente. Sin
       `--mock`, **toda búsqueda falla**: `provider_not_registered`.
- **Severidad**: crítico **1** · alto **1** · medio 0 · bajo 2.
- **Cobertura**: identificados **52** · trazados **52** · contratos **9** ·
  E2E **5** · fallos **2** · pendientes **2**.

---

## 3. Matriz de trazabilidad

Origen = `crates/amatl-ui/assets/*` (conector) y `amatl.toml` (operador).
Proveedor = `crates/amatl-server/src/routes.rs` (tabla canónica),
`crates/amatl-core/src/{service.rs,config/*,providers/*}`. Evidencia E2E =
Chromium headless por CDP sobre el binario release reconstruido.

### 3.1 Borde UI ↔ servidor (endpoints; sin cambios de rutas en el delta)

| ID | Origen | Transporte/operación | Proveedor | Resultado | Evidencia | Estado |
|---|---|---|---|---|---|---|
| U-01 | `app.js:719` | `POST /search` | `routes.rs:66` post(`search_post`) | 200 `{schema_version,status,results,…}` | ESTÁTICO (informe previo) + E2E (`--mock`) | OK |
| U-02 | `index.html:55` | `GET /search` (fallback) | `routes.rs:66` get(`search`) | 200 | ESTÁTICO + INTEGRACIÓN | OK |
| U-03 | `app.js:719` | `POST /deep` | `routes.rs:68` post(`deep_post`) | 200 | ESTÁTICO (informe previo) | OK |
| U-04 | `app.js:719` | `POST /answer` | `routes.rs:69` post(`answer_post`) | 500 `answer_unavailable` (documentado) | ESTÁTICO (informe previo) | OK |
| U-05 | `app.js:993` | `GET /providers` | `routes.rs:70` get(`providers`) | 200 `{schema_version,providers}` | ESTÁTICO + E2E (H-02) | OK |
| U-06 | `app.js:1100` | `GET /status` | `routes.rs:71` get(`status`) | 200 `{…,storage,answer,strategist,…}` | ESTÁTICO + E2E (informe previo) | OK |
| U-07 | `app.js:1457` | `GET /history?limit=` | `routes.rs:72` get(`history`) | 200 `{schema_version,entries}` | ESTÁTICO | OK |
| U-08 | `app.js:1499` | `DELETE /history` | `routes.rs:72` delete(`purge_history`) | 200 `{deleted}` | ESTÁTICO | OK |
| U-09 | `app.js:1489` | `DELETE /history/{id}` | `routes.rs:73` delete(`delete_history_entry`) | 200/404 | ESTÁTICO | OK |
| U-10 | `app.js:1516` | `GET /saved?limit=` | `routes.rs:74` get(`saved_documents`) | 200 `{schema_version,documents}` | ESTÁTICO | OK |
| U-11 | `app.js:1620` | `POST /saved` | `routes.rs:74` post(`save_document`) | 200 `{id,schema_version}` | ESTÁTICO | OK |
| U-12 | `app.js:1546` | `DELETE /saved/{id}` | `routes.rs:75` delete(`delete_saved_document`) | 200/404 | ESTÁTICO | OK |
| U-13 | `app.js:1278` | `POST /answer/enabled` | `routes.rs:77` post(`answer_toggle`) | 200 | ESTÁTICO | OK |
| U-14 | `app.js:1295` | `POST /strategist/enabled` | `routes.rs:78` post(`strategist_toggle`) | 200 | ESTÁTICO | OK |
| U-15 | `app.js:981` | `POST /providers/{name}/enabled` | `routes.rs:79` post(`provider_toggle`) | 200 | ESTÁTICO | OK |
| U-16 | `app.js:1391` | `POST /providers/marginalia/credential` | `routes.rs:80` post(`provider_credential_put`) | 200 `{…,set,available}` | ESTÁTICO | OK |
| U-17 | `app.js:1425` | `DELETE /providers/marginalia/credential` | `routes.rs:80` delete(`provider_credential_delete`) | 200 | ESTÁTICO | OK |
| U-18 | `app.js:1726` | `GET /server/clients` | `routes.rs:83` get(`list_server_clients`) | 200 `{schema_version,clients}` | ESTÁTICO | OK |
| U-19 | `app.js:1809` | `POST /server/clients` | `routes.rs:83` post(`create_server_client`) | 200 `{…,token}` / 409 | ESTÁTICO + E2E (C-20) | OK |
| U-20 | `app.js:1850` | `DELETE /server/clients/{id}` | `routes.rs:84` delete(`delete_server_client`) | 200/404 | ESTÁTICO | OK |
| U-21 | `app.js:1863` | `POST /server/clients/{id}/rotate` | `routes.rs:85` post(`rotate_server_client_token`) | 200 `{id,token}` | ESTÁTICO | OK |
| U-22 | `index.html:15-18` | `GET` assets (`/`,`*.js`,`*.css`,`*.png`) | `amatl-ui::asset()` vía `fallback` | 200 | E2E (carga de página en Chromium) | OK |

### 3.2 Contratos

| ID | Origen | Proveedor | Resultado | Evidencia | Estado |
|---|---|---|---|---|---|
| C-01 | `client.js:24-28` sobre de error `{error:{code,message}}` + `error.status`/`error.code` | `lib.rs:3541-3550` `api_error_with_status` | Gobierno del delta: la UI clasifica con `status`/`code`, no con dígitos en el mensaje | ESTÁTICO (ambos lados) | OK |
| C-02 | `app.js:1787` `MCP_TOOL_NAMES` = `["search","deep_search","fetch","providers","status","answer"]` | `config/server.rs:555-562` `MCP_TOOLS` (6) | Coinciden **uno a uno** (orden incluido) | ESTÁTICO + `cargo test -p amatl-core` (`unshipped_providers_are_absent_and_refused_by_validate` ok) | OK |
| C-03 | `app.js:1806` `tools = scopes.includes("mcp") ? names : []` | `config/server.rs:541-546` (`tools` no vacío exige scope `mcp`) | El delta cierra el antiguo `tools: []`, que negaba todo tool a clientes MCP | ESTÁTICO | OK |
| C-04 | `app.js:1620-1628` payload `POST /saved` (8 campos) | `service.rs` `SaveDocumentInput` | coincide | ESTÁTICO (informe previo) | OK |
| C-05 | `app.js:707-716` payloads `search`/`deep` | `lib.rs` `SearchParams`/`DeepParams` | coincide | ESTÁTICO (informe previo) | OK |
| C-06 | `app.js:283,715,1574` `schema_version === "1"` | `model.rs` `SCHEMA_VERSION="1"` | coincide | ESTÁTICO | OK |
| C-07 | `openapi.yaml` vs `routes.rs` | guard de cobertura OpenAPI | sin mención a SearXNG en el contrato | ESTÁTICO (`grep -i searxng openapi.yaml` vacío) | OK |
| C-08 | `amatl.example.toml:15-37` | `config/providers.rs::builtin_provider_records()` | el ejemplo ya solo lista Marginalia | ESTÁTICO | OK |
| C-09 | `amatl.toml:17-18` `enabled=["searxng"]` + `[providers.searxng]` | `registry.rs::builtin()` (solo Marginalia) + `service.rs:2081-2082` | `validate()` **pasa** (el nombre está declarado en el propio archivo) pero `select_providers()` **falla** | EJECUCIÓN (CLI, sin mock) | **DESAJUSTE** → H-02 |

### 3.3 Borde cliente→proveedor externo (elegibilidad tras la retirada)

| ID | Origen | Proveedor | Resultado | Evidencia | Estado |
|---|---|---|---|---|---|
| P-01 | `amatl.toml` `enabled=["searxng"]` | `registry.builtin()` sin factoría `searxng` | fuente inexistente | ESTÁTICO (`git show bb23a9fc --stat`: `searxng.rs` borrado, 654 líneas) | HUÉRFANA (cliente) → H-02 |
| P-02 | UI `#provider-list` (panel de fuentes) | `GET /providers` | con `amatl.toml` del repo: solo `marginalia`, `status:"unavailable"`, `code:"provider_disabled"` | E2E (curl al servidor con `--mock` y `repo.toml`) | **DESAJUSTE** → H-02 |
| P-03 | UI fuente-selección (`#provider-selector`) | — | retirada: no existe en `index.html`; test estático lo prohíbe | ESTÁTICO (`lib.rs:685-705`) | OK (retirada correcta) |
| P-04 | `applyTranslations()` `app.js:1981` | — (DOM ausente) | consulta a elemento retirado → `TypeError` | E2E (Chromium CDP) | **HUÉRFANA (cliente)** → H-01 |

**Filas sin coincidencia declarada**: U-01..U-22 cierran OK salvo C-09/P-01/P-02
(H-02) y P-04 (H-01). Las rutas de operador sin consumidor en la UI del informe
previo (§5) siguen igual: `routes.rs` no cambió en el delta.

---

## 4. Hallazgos

### H-01 — [ALTO] `applyTranslations()` consulta un elemento eliminado y lanza en cada carga de página

- **Flujos**: P-04; afecta a los flujos de UI que dependen de `applyTranslations()`
  y su continuación (resumen de filtros, resumen de acceso/token, tema).
- **Evidencia**:
  - [ESTÁTICO] `app.js:1981-1984`:
    ```js
    const sourceAllOption = document.querySelector(
      '.provider-selector-option input[value="all"]',
    ).closest("label");
    sourceAllOption.querySelector("span").textContent = t("providerSourceAll");
    ```
    `index.html` ya no contiene `provider-selector-option` ni `id="provider-selector"`
    (`grep` vacío). La limpieza retiró `#provider-selector-label` de
    `applyTranslations` pero **dejó estas tres líneas**.
  - [E2E] Chromium headless (CDP) sobre el binario release reconstruido, cargando
    `http://127.0.0.1:8951/`:
    - `Runtime.exceptionThrown`: `TypeError: Cannot read properties of null (reading 'closest')`
      con pila `at applyTranslations (app.js:1983:4)` ← `at HTMLDocument.<anonymous> (app.js:1993:3)`.
    - Invocación directa: `applyTranslations()` → `THROWS: TypeError: Cannot read properties of null (reading 'closest')`.
    - Estado del DOM: `document.querySelector('.provider-selector-option input[value="all"]')` → `null`.
  - [E2E] Consecuencia observada — `updateThemeToggle()` **nunca corre** al cargar:
    `#theme-toggle .icon-sun` conserva la clase `is-active` del markup mientras
    `resolvedTheme()` es `dark` (→ debería activarse `.icon-moon`); `icon-moon`
    queda sin `is-active`. Invocada a mano, `updateThemeToggle()` funciona
    (`aria-pressed="true"`, `icon-moon is-active`), probando que **solo el punto
    de llamada está muerto**.
  - [ESTÁTICO] Barrido de selectores del cliente contra `index.html`: **una sola**
    consulta colgante en todo `app.js` (esta).
- **Síntoma**: un error de JS no capturado en cada `DOMContentLoaded`; tras él no
  se inicializan `updateFiltersSummary()`, `updateAccessSummary()` ni
  `updateThemeToggle()`; en un cambio de idioma, `setLocale()` llama a
  `applyTranslations()` y aborta antes de `render(store.get())`,
  `refreshPanels()` y `rerenderStatus()` (etiquetas posteriores a la línea 1984
  no se re-traducen en esa pasada).
- **Causa**: **CONFIRMADO** — referencia residual a un control retirado por
  `bb23a9fc`. El elemento existía antes; la retirada borró el markup pero no la
  consulta.
- **Impacto**: alto sobre la robustez del conector: cualquier estado derivado se
  retrasa o no ocurre en carga; el toggle de tema muestra el icono contrario hasta
  el primer clic; el cambio de idioma queda parcial. No rompe la búsqueda, pero sí
  el arranque limpio de la página y la coherencia de tema/idioma.
- **Por qué el test existente no lo atrapó**: `browser_e2e.rs:330`
  (`switching_interface_language_updates_language_and_primary_copy`) afirma
  `#search-heading` = "Busquemos", pero ese `textContent` se fija en `app.js:1929`,
  **antes** del `throw` en 1983: el test valida elementos previos al fallo y no
  observa la excepción no capturada.
- **Corrección mínima propuesta** (no aplicada):
  ```js
  // app.js — eliminar las tres líneas huérfanas (1981-1984)
  -  const sourceAllOption = document.querySelector(
  -    '.provider-selector-option input[value="all"]',
  -  ).closest("label");
  -  sourceAllOption.querySelector("span").textContent = t("providerSourceAll");
  ```
  El `#provider-selector-label` ya se retiró en el mismo bloque; estas tres son el
  resto. `providerSourceAll` puede quedar en el catálogo sin uso (inofensivo), pero
  retirarlo también es coherente con `lib.rs:702` que ya lo prohíbe.
- **Regresión recomendada**:
  1. En `browser_e2e.rs`, suscribirse a `Runtime.exceptionThrown`/`window.onerror`
     durante la carga y afirmar **cero excepciones no capturadas** (no solo el
     `textContent` resultante). Este es el guard que faltaba.
  2. Ampliar `retired_source_selector_is_gone_and_cannot_return`
     (`crates/amatl-ui/src/lib.rs:685`) para afirmar además que `app.js` **no
     consulta** `provider-selector-option` ni ninguna clase del selector retirado.

### H-02 — [CRÍTICO] `amatl.toml` habilita una fuente que ya no existe: toda búsqueda sin `--mock` falla

- **Flujos**: P-01/P-02, C-09; y todo flujo de producto que parte de la config de
  trabajo (`search`, `deep`, `answer`, `provider-canary` real).
- **Evidencia**:
  - [ESTÁTICO] `amatl.toml:17-18`: `[providers] enabled = ["searxng"]`, más un
    expediente `[providers.searxng]` (`amatl.toml:24-44`).
  - [ESTÁTICO] `crates/amatl-core/src/providers/registry.rs`:
    `builtin()` → `Self::empty().with(Arc::new(MarginaliaFactory))` (una sola
    factoría). `bb23a9fc` borró `providers/searxng.rs` (-654 líneas).
  - [ESTÁTICO] `config/providers.rs::builtin_provider_records()`: un solo
    expediente, `marginalia`.
  - [ESTÁTICO] `service.rs:2081-2082` (`select_providers`, rama **no mock**):
    ```rust
    let factory = self.registry.get(name)
        .ok_or_else(|| ServiceError::ProviderNotRegistered(name.clone()))?;
    ```
    Un nombre en `providers.enabled` sin factoría **no degrada: devuelve error**.
  - [EJECUCIÓN] CLI en proceso, sin `--mock`, con `amatl.toml` del repo:
    ```
    $ amatl --config-file <repo amatl.toml> search "rust"
    error_code=provider_not_registered message=declared provider has no registered implementation
    Error: declared provider has no registered implementation: searxng
    ```
  - [E2E] Servidor local con `amatl.toml` del repo (sin `--mock`): `GET /providers`
    devuelve **solo** `marginalia` con `status:"unavailable"`,
    `code:"provider_disabled"` — `searxng` no aparece, sin aviso en el log.
  - [ESTÁTICO] `config/mod.rs:472-479` `validate()` solo comprueba
    `is_declared(name)`; como el propio `amatl.toml` **declara**
    `[providers.searxng]`, la validación **pasa** y el desajuste no se detecta en
    carga. El ejemplo versionado (`amatl.example.toml`) ya está limpio, así que el
    drift es del archivo local no versionado (`.gitignore:2`).
- **Síntoma**: con el archivo de trabajo del repo, el operador cree tener SearXNG
  activo; la realidad es que (a) ninguna búsqueda real funciona
  (`provider_not_registered`) y (b) cualquier lectura de `/providers` muestra una
  única fuente deshabilitada.
- **Causa**: **CONFIRMADO** — la retirada `bb23a9fc` actualizó producto, ejemplo y
  documentación, pero **no** el `amatl.toml` de trabajo presente en el repo, que
  sigue fijando la fuente retirada. El cambio se marcó `!` (breaking) precisamente
  por esto.
- **Impacto**: **crítico** para el operador de este checkout: el flujo central
  (`buscar`) no puede completarse sin editar la configuración. No afecta a un
  despliegue que parta de `amatl.example.toml`.
- **Corrección mínima propuesta** (no aplicada): en `amatl.toml`, poner
  `enabled = []` (o `["marginalia"]` con key real) y eliminar el bloque
  `[providers.searxng]`; alinear con `amatl.example.toml`. Alternativa de
  producto (más profunda): que `validate()` rechace un `enabled` que, estando
  declarado en el archivo, **no tenga factoría registrada** — hoy la declaración
  local “blanquea” el nombre y evita el error temprano.
- **Regresión recomendada**: test que cargue el `amatl.toml` de trabajo real (o un
  fixture equivalente) y afirme `validate()` OK **y** elegibilidad de cada nombre
  efectivo; o un test que afirme que todo nombre en `providers.enabled` tiene
  `registry.contains(name)`.

---

### Observaciones de severidad baja (causa CONFIRMADA, sin ruptura de flujo)

### H-03 — [BAJO] Test de retirada no cubre consultas JS al selector eliminado

- **Evidencia**: [ESTÁTICO] `crates/amatl-ui/src/lib.rs:685-705`
  (`retired_source_selector_is_gone_and_cannot_return`) verifica ausencia de
  cadenas concretas (`providerSelectorLabel`, `applyProviderSource`,
  `value="searxng"`, …) pero **no** la clase CSS `provider-selector-option`, que
  es la que quedó viva en `app.js:1982`.
- **Causa**: CONFIRMADO — el test enumera símbolos, no el contrato “no quedan
  consultas a elementos del selector retirado”.
- **Impacto**: bajo por sí mismo; es la causa raíz de que H-01 pasara la suite.
- **Corrección mínima propuesta**: añadir a ese test `assert!(!javascript.contains("provider-selector-option"))`.
- **Regresión recomendada**: la misma aserción.

### H-04 — [BAJO] Chrome 158 vs ChromeDriver 157 en el host; el E2E WebDriver no corre local

- **Evidencia**: [EJECUCIÓN] `google-chrome --version` → `158.0.8127.36`;
  `chromedriver --version` → `157.0.8103.78`. La suite oficial
  (`browser_e2e.rs`, `fantoccini`/WebDriver) falla cerrada con ese desajuste;
  esta corrida usó **CDP directo** para obtener evidencia real. El repo ya
  aborda el pin en CI (`2bfff4e3`, `d34d2cd6`).
- **Causa**: CONFIRMADO — desajuste de versiones en el host de desarrollo.
- **Impacto**: bajo (entorno, no producto); impide reproducir el E2E oficial
  fuera de CI.
- **Corrección mínima propuesta**: instalar el driver que empareje con Chrome 158
  (o fijar Chrome 157 en el host), como ya hace el workflow.
- **Regresión recomendada**: ninguna de producto; ya cubierto por CI.

---

## 5. Operaciones sin correspondencia

- **Cliente → proveedor externo, huérfana (H-02)**: `amatl.toml`
  (`enabled=["searxng"]`) invoca una fuente que el registro ya no implementa
  (`registry.builtin()` = solo Marginalia). Es el único huérfano del borde
  cliente→proveedor de esta corrida.
- **Cliente → DOM, huérfana (H-01)**: `app.js:1981` consulta
  `.provider-selector-option`, retirado de `index.html`. Único punto colgante
  detectado por barrido estático de todos los `querySelector` de `app.js`.
- **Huérfanas del proveedor respecto a la UI**: **sin cambios** en este delta
  (`routes.rs` intacto). Sigue vigente la lista del informe previo (§5):
  `/search/batch`, `/reload`, `/inference`, `/data-policy`, `/policies*`,
  `/persistence*`, `/circuits*`, `/telemetry`, `/deep/limits|extractor|renderer`,
  `/security-events`, `/server/pending-config`, `PATCH /answer`,
  `PATCH /providers/{name}`, `PATCH`/`DELETE /server/clients/{id}`. Todas son
  superficies de **operador/CLI/MCP** con uso externo legítimo; no se han podido
  atribuir a un consumidor desconocido dentro del repo, así que se declaran como
  “sin consumidor en este repo”, no como rotas.
- **Borde externo distinto de la UI**: los `tools/*.py` siguen siendo clientes de
  **SearXNG** (`http://127.0.0.1:8888/search`), no de la API de AMATL. Son
  utilitarios de campañas históricas; con la retirada de SearXNG quedan sin
  proveedor operativo. Fuera del alcance de este conector, se anotan para no
  confundirlos con consumidores de la API.

---

## 6. No verificado y límites

- **H-01 en el E2E oficial (WebDriver)**: NO VERIFICADO por el desajuste
  ChromeDriver 157 / Chrome 158 del host (H-04). Sí verificado por **CDP directo**
  con evidencia de excepción y de estado del DOM. Procedimiento para resolverlo:
  instalar el driver emparejado y correr
  `AMATL_BROWSER_E2E=1 cargo test -p amatl-server --test browser_e2e`; se espera
  que el test de idioma **siga pasando** (afirma elementos previos al `throw`),
  por lo que la brecha de cobertura de H-03 es la que hay que cerrar, no el test
  actual.
- **Búsqueda contra fuentes reales**: excluida por diseño. `--mock` no alcanza la
  red; y la reproducción de H-02 es justamente *antes* de conectar, por lo que no
  requiere red ni credencial de Marginalia.
- **Endpoints de operador sin consumidor**: no se ejercieron; su no-uso se
  declaró por lectura estática (coherente con el informe previo).
- **`amatl.toml` no versionado**: por definición (`.gitignore:2`) es estado local
  del operador. El hallazgo H-02 se emite sobre el archivo **presente en este
  checkout** (el que `README`/`start.sh` usan por defecto), no sobre un artefacto
  versionado; un clon limpio que use `amatl.example.toml` no lo reproduce.
- **Calidad del ranking / relevancia**: fuera de alcance de una auditoría de
  correspondencia.

---

## 7. Plan de corrección

| Orden | Hallazgo | Dependencia | Cambio mínimo | Verificación de cierre |
|---|---|---|---|---|
| 1 | H-02 (crítico) | ninguna | Editar `amatl.toml`: `enabled = []` (o `["marginalia"]` con key) y borrar `[providers.searxng]`; opcional, endurecer `validate()` para rechazar `enabled` sin factoría | `amatl --config-file amatl.toml search "rust"` deja de devolver `provider_not_registered`; `GET /providers` no lista una fuente inexistente |
| 2 | H-01 (alto) | ninguna | Borrar `app.js:1981-1984` (las tres líneas huérfanas del selector retirado) | Carga en browser real sin `TypeError`; `updateThemeToggle/filtersSummary/accessSummary` corren en `DOMContentLoaded`; `cargo test -p amatl-ui` y la suite de navegador verdes |
| 3 | H-03 (bajo) | H-01 | Añadir `assert!(!javascript.contains("provider-selector-option"))` a `retired_source_selector_is_gone_and_cannot_return` | El test falla si la consulta reaparece (probable por mutación) |
| 4 | H-04 (bajo) | entorno | Instalar el ChromeDriver emparejado con Chrome 158 en el host | `AMATL_BROWSER_E2E=1 … browser_e2e` corre sin el fallo de versión |

**Recomendación de guard nuevo (transversal)**: dotar a la suite de navegador de
una aserción de **cero excepciones no capturadas** durante la carga y el cambio
de idioma. Es el control que habría detenido H-01 en origen, y cubre toda la clase
“consulta a elemento retirado”, no solo este caso.
