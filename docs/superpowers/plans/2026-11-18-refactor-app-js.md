# Refactor interno de `app.js` — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorganizar `crates/amatl-ui/assets/app.js` en dominios delimitados dentro del mismo archivo, encapsulando estado y reduciendo la superficie global, sin cambiar el comportamiento observable.

**Architecture:** Un solo archivo, un solo `<script defer>`. Se introduce un **núcleo transversal** de solo lectura (`t`, utilidades, barra de estado, `refreshPanels`, constantes) y luego **IIFEs de dominio** que encapsulan su propio estado y leen el núcleo. Los 85 nodos DOM permanecen globales de solo lectura. El bloque de arranque (`refreshPanels()`, listeners, `DOMContentLoaded`) queda al final, en su orden actual.

**Tech Stack:** JavaScript vainilla sin build, sin dependencias, CSP `'self'`. Verificación con `cargo test -p amatl-ui` (tests de contrato por substring) y Chromium por CDP para el comportamiento.

**Spec:** `docs/superpowers/specs/2026-11-18-refactor-app-js-design.md`

## Global Constraints

- **Un solo archivo**: `crates/amatl-ui/assets/app.js`. No se crean archivos nuevos; `asset()` en `crates/amatl-ui/src/lib.rs:25` es una allowlist de rutas exactas.
- **Sin módulos ES**: sigue siendo `<script defer>` clásico. `index.html:18` no cambia.
- **Sin dependencias, sin build, sin CDN**: CSP `lib.rs:7` es `script-src 'self'`.
- **Los símbolos verificados conservan su nombre y texto exactos**: **85 aserciones** en `crates/amatl-ui/src/lib.rs` leen `text("/app.js")` y buscan substrings.
- **El orden de ejecución observable no cambia**: `refreshPanels()` en la línea 1901 y `DOMContentLoaded` en la 1987 conservan su posición relativa.
- **Núcleo de solo lectura**: `t`, `pluralKey`, `countLabel`, `boundedText`, `safeHttpUrl`, `validHash`, `setStatusKey` + estado asociado, `announce`, `refreshPanels`, y las constantes `PAGE_SIZE`/`LIST_LIMIT`/`MAX_*`. Ningún dominio los re-declara ni los sombrea.
- **Los 85 nodos DOM siguen globales** (`const x = document.querySelector(...)` de nivel superior). No se re-consultan dentro de dominios.
- **Comentarios de intención en español**; identificadores en inglés (convención del repo).

## Review Focus

Las cinco clases de fallo que el spec implica y que **ningún test existente atrapa**, porque las aserciones son `contains` de strings y **no ejecutan JS** (`lib.rs:533`):

1. **TDZ por IIFE**: un dominio lee un `const` declarado *después* de su bloque, y lanza `ReferenceError` al cargar. Los tests pasan igual.
2. **Listener duplicado**: un `addEventListener` movido dentro de un dominio y ejecutado dos veces, de modo que un cambio de idioma o un toggle se aplica doble.
3. **Nodo re-consultado tarde**: un dominio hace su propio `querySelector` tras un re-render y captura un nodo distinto del que el resto usa.
4. **Símbolo transversal sombreado**: un dominio declara su propio `t`/`boundedText` y el resto de la página deja de ver el real.
5. **Orden de arranque alterado**: `restoreFromLocation()` corre antes que `applyTranslations()`, y la pantalla restaurada queda sin traducir.

Cada uno tiene su test o su verificación en la tarea que toca el código correspondiente (ver Tareas 2 y 8).

---

### Task 1: Andamiaje del núcleo y verificación previa

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` (solo inserción de cabecera de sección)

**Interfaces:**
- Consumes: nada.
- Produces: el comentario de estructura en la cabecera del archivo, que fija el orden de bloques para las tareas siguientes.

- [ ] **Step 1: Registrar el baseline**

Run:
```bash
cargo test -p amatl-ui --locked
```
Expected: `test result: ok. 29 passed`.

- [ ] **Step 2: Añadir el índice de bloques como comentario de cabecera**

Insertar tras la primera línea de comentario del archivo, sin tocar código:

```javascript
// ── Estructura del archivo ──────────────────────────────────────
// Núcleo transversal (SOLO LECTURA para los dominios, declarado primero):
//   i18n · utilidades · barra de estado · orquestador de refresco · constantes
// Captura de nodos DOM: 85 `const`, globales, de solo lectura.
// Dominios (IIFEs, autocontenidos, leen el núcleo, no lo re-declaran):
//   theme · net · render · search-deep · providers · storage-panels · clients
// Bloque de arranque (al final, en este orden):
//   refreshPanels() · listeners · DOMContentLoaded
```

- [ ] **Step 3: Verificar que nada cambió**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed` (un comentario no altera ninguna aserción).

- [ ] **Step 4: Commit**

```bash
git add crates/amatl-ui/assets/app.js
git commit -m "refactor(ui): index the app.js block structure in a header comment"
```

---

### Task 2: Verificación en navegador (la red que la suite no da)

**Files:**
- Create: `/tmp/user/1000/opencode/amatl-refactor/probe.py` (efímero, fuera del repo)

**Interfaces:**
- Consumes: el binario `target/release/amatl` y el servidor `--mock`.
- Produces: un script reutilizable que, dada una URL, afirma **cero excepciones no capturadas** y recorre los flujos principales. Cada tarea posterior lo re-ejecuta.

- [ ] **Step 1: Escribir el probe**

Debe, con Chromium headless por CDP:
1. Suscribirse a `Runtime.exceptionThrown` y `Runtime.consoleAPICalled` (nivel error).
2. Cargar la página y esperar 3 s.
3. Evaluar `window.onerror`/`unhandledrejection` acumulados.
4. Recorrer: escribir consulta, pulsar Buscar, cambiar idioma es→en→es, pulsar el toggle de tema.
5. Imprimir `EXCEPTIONS: N` y fallar con salida no-cero si N > 0.

El script de la auditoría del 2026-11-18 (`behav.py`, en `/tmp/user/1000/opencode/amatl-audit/`) sirve de base: ya abre Chrome, habla CDP y evalúa en la página.

- [ ] **Step 2: Reconstruir el binario y arrancar el servidor**

```bash
cargo build --release -p amatl-cli --locked
# servidor --mock en 127.0.0.1:8955, no_auth = true, enabled = []
```

Regla del repo (`audit/embedded-asset-staleness`): los assets van con `include_bytes!`, así que **hay que reconstruir** o el binario sirve el JS previo.

- [ ] **Step 3: Correr el probe y registrar el estado base**

Run: `python3 /tmp/user/1000/opencode/amatl-refactor/probe.py`
Expected: `EXCEPTIONS: 0` **antes** de cualquier refactor. Si no es 0, parar: el baseline está sucio.

- [ ] **Step 4: No commitear** (el probe es efímero, fuera del repo).

---

### Task 3: Dominio `theme`

**Files:**
- Modify: `crates/amatl-ui/assets/app.js:5-20` (`storedTheme`, `applyTheme`), `:136-165` (`resolvedTheme`, `updateThemeToggle`, listener)

**Interfaces:**
- Consumes: `themeToggle` (nodo global), `t` (núcleo).
- Produces: `applyTheme(theme)`, `resolvedTheme()`, `updateThemeToggle()` — mismas firmas, ahora dentro de una IIFE que expone solo esas tres al resto del archivo si alguna las llama de fuera. **Antes de encapsular, comprobar quién las llama fuera del bloque**; hoy `updateThemeToggle` se llama desde `applyTranslations` (:1983) y desde su propio listener.

- [ ] **Step 1: Comprobar los llamadores externos**

Run: `grep -n "updateThemeToggle\|resolvedTheme\|applyTheme" crates/amatl-ui/assets/app.js`
Expected: si `updateThemeToggle` se llama desde fuera del bloque de tema, la IIFE **debe** exponerla (p. ej. `globalThis.__theme = { updateThemeToggle }` **no**: rompería CSP-idiom). En su lugar, mantenerla en el núcleo o pasar una referencia. Decidir según lo que devuelva el grep y anotarlo.

- [ ] **Step 2: Envolver el bloque en una IIFE**

Sin cambiar cuerpos ni nombres. El estado si lo hubiera queda local.

- [ ] **Step 3: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`. Las aserciones de `storedTheme`/`applyTheme` (`presentation_keeps_technical_internals_hidden`) siguen encontrando sus strings.

- [ ] **Step 4: Probe en navegador**

Run: reconstruir + `python3 …/probe.py`
Expected: `EXCEPTIONS: 0`, y el toggle de tema cambia el atributo `data-theme`.

- [ ] **Step 5: Commit**

```bash
git commit -am "refactor(ui): delimit the theme domain in an IIFE"
```

---

### Task 4: Dominio `net` (envoltura de `globalThis.api` y errores)

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `globalThis.authHeaders` (:86), `reportFailure` (:641), `ERROR_MESSAGE_KEYS` (:659), `reportValidationError` (:877), `clearValidationError` (:889)

**Interfaces:**
- Consumes: `globalThis.api` (de `client.js`), `statusNode`, `t`.
- Produces: `reportFailure(error)`, `reportValidationError(key)`, `clearValidationError()` con firmas idénticas. `authHeaders` **debe seguir en `globalThis`**: `client.js:8` lo lee por ese nombre.

- [ ] **Step 1: Test de contrato del contrato de error**

El test `report_failure_reads_only_error_properties_never_a_bare_message` (:527) recorta el cuerpo de `reportFailure` por `split_once("function reportFailure(error) {")`. Ese literal **debe permanecer** exacto. Verificarlo antes de tocar:

Run: `grep -n "function reportFailure(error) {" crates/amatl-ui/assets/app.js`
Expected: una coincidencia exacta.

- [ ] **Step 2: Encapsular el dominio**

`reportFailure` y sus vecinos dentro de la IIFE; `globalThis.authHeaders` permanece como asignación de nivel superior (su contrato es el nombre global).

- [ ] **Step 3: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`, en particular `report_failure_reads_only_error_properties_never_a_bare_message` y `mcp_clients_carry_the_tool_vocabulary_and_errors_classify_on_status`.

- [ ] **Step 4: Probe**

Run: `python3 …/probe.py` tras reconstruir.
Expected: `EXCEPTIONS: 0`.

- [ ] **Step 5: Commit**

```bash
git commit -am "refactor(ui): delimit the net/error-classification domain"
```

---

### Task 5: Dominio `render`

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `renderStatus` (:226), `setStatusKey` (:234), `rerenderStatus` (:239), `announce` (:247), `renderSearchResult` (:315), `renderEvidenceFragment` (:435), `renderDeepDocument` (:475), `renderAnswerCard` (:556), `render` (:567), `deepItems` (:593), `documentStatus`, `acquisitionMethod`, `signalLabel`

**Interfaces:**
- Consumes: nodos globales, `t`, `boundedText`, `store`.
- Produces: `render`, `renderStatus`, `setStatusKey`, `rerenderStatus`, `announce`. **`setStatusKey` y `announce` son transversales** (20 y 12 usos, desde otros dominios y desde el arranque): van al **núcleo**, no a esta IIFE.

- [ ] **Step 1: Separar lo transversal de lo local**

`setStatusKey`, `rerenderStatus`, `announce` y su estado (`statusRenderer`, `statusState`, `statusNode`) se quedan en el núcleo. El resto del render entra en la IIFE.

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed` (`presentation_keeps_technical_internals_hidden`, `deep_evidence_is_bounded_traceable_and_has_no_local_file_surface`).

- [ ] **Step 3: Probe**

Expected: `EXCEPTIONS: 0`; una búsqueda mock pinta resultados y el estado se reinicia.

- [ ] **Step 4: Commit**

```bash
git commit -am "refactor(ui): delimit the render domain, hoist the status bar to the core"
```

---

### Task 6: Dominio `providers`

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `providerReason` (:955), `toggleEnabled` (:960), `toggleProvider` (:979), `fetchProviderStatus` (:990), `configRow`, `percentage`, `marginaliaCredentialSource`, `updateMarginaliaUi`, `syncMarginaliaFromStatus`, `openMarginaliaDialog`, `saveMarginaliaKey`, `syncCredentialActions`

**Interfaces:**
- Consumes: `globalThis.api`, `t`, `boundedText`, `setStatusKey`.
- Produces: `fetchProviderStatus()` (la llama `refreshPanels` en el arranque), `syncCredentialActions(enabledList)`.

- [ ] **Step 1: Encapsular, dejando `fetchProviderStatus` accesible al orquestador**

`refreshPanels` la llama desde el bloque de arranque: la IIFE debe exponerla de forma que el núcleo la vea (mantenerla en el núcleo, o devolverla y asignarla a una referencia del núcleo — decidir y anotar).

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed` (`marginalia_surface_copy_is_catalog_driven`, `marginalia_credential_ui_reflects_the_credential_source`).

- [ ] **Step 3: Probe**

Expected: `EXCEPTIONS: 0`; el panel de fuentes se pinta y el toggle responde una sola vez (cubre el fallo 2 del Review Focus).

- [ ] **Step 4: Commit**

```bash
git commit -am "refactor(ui): delimit the providers domain"
```

---

### Task 6b: Dominio `search-deep`

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `setBusy` (:204), `queryText` (:258), `validatePayload` (:615), `focusTokenField` (:636), `run` (:680), `execute` (:897), `requestedModeIsUnavailable` (:915)

**Interfaces:**
- Consumes: `globalThis.api`, `t`, `boundedText`, `validHash`, `setStatusKey`, `store`, `syncLocation` (Tarea 6c).
- Produces: `run(mode, page)`, `execute(event)`. `searchAvailable` (:78) es estado de este dominio.

- [ ] **Step 1: Encapsular `search-deep`**

`queryText` y `setBusy` los usan otros dominios (`render`): si el grep muestra llamadas externas, se quedan en el **núcleo** en vez de en la IIFE. Verificar antes de mover.

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed` (`search_and_deep_use_post_contracts_and_safe_dom_apis` busca `globalThis.api.post(endpoint`, `body.page = store.get().page`, `headers.Authorization`).

- [ ] **Step 3: Probe**

Expected: `EXCEPTIONS: 0`; buscar y "Analizar evidencia" pintan resultados.

- [ ] **Step 4: Commit**

```bash
git commit -am "refactor(ui): delimit the search-deep domain"
```

---

### Task 6c: Dominio `location`

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `searchLocation` (:815), `syncLocation` (:828), `restoreFromLocation` (:836)

**Interfaces:**
- Consumes: `store`, `queryInput` y demás nodos de filtros, `run` (Tarea 6b).
- Produces: `searchLocation(page)`, `syncLocation(page)`, `restoreFromLocation()`. El spec permite agruparlo con `search-deep` (es quien lo llama); **si se agrupa, esta tarea se funde con la 6b**.

- [ ] **Step 1: Decidir agrupación**

Si `search-deep` es la única llamante de las tres, fundirlas en una IIFE y saltar al Step 3. Anotar la decisión.

- [ ] **Step 2: Encapsular `location`**

- [ ] **Step 3: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`.

- [ ] **Step 4: Probe de restauración de URL**

Cargar `/?q=rust&mode=search` en el navegador y confirmar que la consulta y los resultados se restauran **y** la interfaz queda traducida (fallo 5 del Review Focus: `restoreFromLocation` después de `applyTranslations`).

Expected: `EXCEPTIONS: 0`, campo y resultados poblados, `h1` traducido.

- [ ] **Step 5: Commit**

```bash
git commit -am "refactor(ui): delimit the location domain"
```

---

### Task 7: Dominios `storage-panels` y `clients`

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `historyLabel` (:1442) … `deleteSavedDocument` (:1544); `scopeLabel` (:1677) … `rotateServerClientToken`

**Interfaces:**
- Consumes: `globalThis.api`, `t`, `countLabel`, nodos de historial/guardados/clientes.
- Produces: `fetchHistory`, `fetchSavedDocuments`, `fetchServerClients` (las llama `refreshPanels`).

- [ ] **Step 1: Encapsular `storage-panels`**

Mantener `historyLabel` (usa `countLabel`, pluralizado) intacto: el test `history_count_uses_the_plural_aware_helper` busca `countLabel(entry.total_results, "resultsCount")` (con el espacio tras la coma).

- [ ] **Step 2: Encapsular `clients`**

`MCP_TOOL_NAMES` (:1787) **debe conservar su texto exacto**: el test `mcp_clients_carry_the_tool_vocabulary_and_errors_classify_on_status` lo compara con `MCP_TOOLS`.

- [ ] **Step 3: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`.

- [ ] **Step 4: Probe**

Expected: `EXCEPTIONS: 0`; los paneles respetan el gating de almacenamiento (`storage_backed_panels_are_gated_on_storage_availability`).

- [ ] **Step 5: Commit**

```bash
git commit -am "refactor(ui): delimit the storage-panels and clients domains"
```

---

### Task 8: Bloque de arranque y verificación final

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `refreshPanels` (:1885), listeners (:1899-1900), `refreshPanels()` (:1901), `applyDeclarativeTranslations` (:1903), `applyTranslations` (:1924), `DOMContentLoaded` (:1987)

**Interfaces:**
- Consumes: todos los dominios de las tareas 3-7.
- Produces: el archivo en su forma final.

- [ ] **Step 1: Fijar el orden del arranque**

El bloque final conserva exactamente esta secuencia:
1. `applyDeclarativeTranslations()` y `applyTranslations()` (i18n, en el núcleo).
2. `refreshPanels()` (:1901).
3. Los listeners de nivel superior (`historyPurgeButton`, `tokenInput`).
4. `document.addEventListener("DOMContentLoaded", …)` con `uiLanguage.value = LANG` → `applyTranslations()` → `setStatusKey("initialStatus","")` → `restoreFromLocation()`.

`restoreFromLocation()` **después** de `applyTranslations()` (fallo 5 del Review Focus).

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`.

- [ ] **Step 3: Test de clase de fallo — símbolo transversal sombreado**

Añadir a `crates/amatl-ui/src/lib.rs`:

```rust
#[test]
fn domains_do_not_shadow_the_core_transversal_symbols() {
    let javascript = text("/app.js");
    // Un dominio que re-declara `t` o `boundedText` desvía el resto de la
    // página a una copia que nadie más usa.
    assert_eq!(javascript.matches("function t(key)").count(), 1);
    assert_eq!(javascript.matches("function boundedText(").count(), 1);
    assert_eq!(javascript.matches("function setStatusKey(").count(), 1);
}
```

- [ ] **Step 4: Correr el test nuevo**

Run: `cargo test -p amatl-ui --locked domains_do_not_shadow`
Expected: PASS.

- [ ] **Step 5: Probe completo**

Run: `python3 …/probe.py` sobre el binario reconstruido.
Expected: `EXCEPTIONS: 0`, y los cinco flujos (buscar, deep, historial, cambio de idioma, tema) responden una sola vez.

- [ ] **Step 6: Suite completa**

Run: `cargo test --workspace --locked`
Expected: todas verdes (el refactor no toca Rust de producto, así que el recuento no baja).

- [ ] **Step 7: Commit**

```bash
git add crates/amatl-ui/assets/app.js crates/amatl-ui/src/lib.rs
git commit -m "refactor(ui): fix the app.js startup order and guard the core symbols"
```

---

### Task 9: Cierre — comparación contra el baseline

**Files:**
- Modify: ninguno.

**Interfaces:**
- Consumes: el baseline de la Tarea 1 y los probes de cada tarea.
- Produces: la evidencia de que el refactor no cambió comportamiento.

- [ ] **Step 1: Diff de comportamiento**

Comparar la salida del probe final con la del baseline de la Tarea 2. Deben coincidir en: número de excepciones (0), textos principales (`h1`, botones), y el estado del DOM tras una búsqueda mock.

- [ ] **Step 2: Confirmar que ningún símbolo verificado se movió de archivo**

Run: `grep -c "" crates/amatl-ui/assets/app.js` y comparar el recuento de funciones top-level con el baseline (86, menos las que quedaron dentro de IIFEs — anotar la diferencia y que sea solo por encapsulamiento).

- [ ] **Step 3: Dictamen en pantalla**

Reportar: tareas completadas, excepciones en navegador (debe ser 0), tests (29 + 1 nuevo), y cualquier desviación del spec con su motivo.
