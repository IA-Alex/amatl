# Refactor de `app.js` con IIFEs completos — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Encerrar cada dominio de `crates/amatl-ui/assets/app.js` en un IIFE real que encapsula su estado y **retorna** los símbolos que otros dominios y el arranque consumen, bajando la superficie global de ~86 funciones a un núcleo pequeño y explícito.

**Architecture:** El archivo pasa de "se ejecuta de arriba a abajo" a **núcleo primero, dominios después, arranque al final**. El núcleo transversal (`t`, utilidades, barra de estado, constantes) se declara arriba y los dominios lo leen. Cada dominio es un IIFE `const { … } = (function () { … return { … }; })();` que expone solo lo que cruza su frontera. El bloque final coordina: `applyTheme(storedTheme())`, `refreshPanels()`, los listeners y `DOMContentLoaded`.

**Tech Stack:** JavaScript vainilla, sin build, sin dependencias, CSP `'self'`. Un solo archivo.

**Spec:** `docs/superpowers/specs/2026-11-18-refactor-app-js-design.md` (incluye la **Enmienda 2026-11-18** que autoriza el cambio de mecanismo de arranque).

## Global Constraints

- **Un solo archivo**: `crates/amatl-ui/assets/app.js`. `asset()` (`crates/amatl-ui/src/lib.rs:25`) es allowlist de rutas exactas: no se crean archivos.
- **Sin módulos ES**, sin bundler, sin CDN. `index.html:18` no cambia (`<script defer>`).
- **Los símbolos verificados conservan nombre y texto exactos**: **85 aserciones** leen `text("/app.js")` y buscan substrings.
- **`globalThis.authHeaders` permanece global**: lo lee `client.js:8`.
- **`function reportFailure(error) {` permanece literal**: el test lo recorta con `split_once` (`lib.rs:527`).
- **`MCP_TOOL_NAMES` conserva su texto**: comparado con `MCP_TOOLS`.
- **`countLabel(entry.total_results, "resultsCount")` conserva el espacio tras la coma**.
- **Núcleo de solo lectura**: `t`, `pluralKey`, `countLabel`, `boundedText`, `safeHttpUrl`, `validHash`, `setStatusKey` (+ `statusNode`/`statusState`/`statusRenderer`/`rerenderStatus`), `announce`, `refreshPanels`, `PAGE_SIZE`/`LIST_LIMIT`/`MAX_*`.
- **Los 85 nodos DOM siguen de nivel superior** y de solo lectura: re-consultarlos dentro de un dominio cambiaría el momento de captura tras un re-render.
- **Comportamiento visible idéntico**: misma UI, mismos flujos, mismos textos. El *mecanismo* de arranque cambia (autorizado en la Enmienda).

## Review Focus

Las cinco clases de fallo que **ningún test atrapa** (las aserciones son `contains` de strings y no ejecutan JS, `lib.rs:533`), y que con IIFEs completos pasan de improbables a probables:

1. **TDZ en el núcleo**: un dominio lee un símbolo del núcleo declarado *después* de él. Tarea 5 (núcleo) + test de Tarea 9.
2. **Retorno incompleto**: un dominio no retorna un símbolo que otro consume, y el consumidor recibe `undefined`. Cada tarea 3-8 lo prueba con el probe.
3. **Listener duplicado o ausente**: al reubicar los `addEventListener` al bloque final, uno queda fuera o se registra dos veces. Tarea 9 lo prueba.
4. **Orden de arranque roto**: `applyTheme(storedTheme())` corre antes que los dominios, o `restoreFromLocation()` antes que `applyTranslations()`. Tareas 3 y 9.
5. **Símbolo re-declarado**: un dominio declara su propio `t`/`boundedText`, desviando a los demás a una copia huérfana. Tarea 9 lo prueba.

---

### Task 1: Baseline y plan versionado

**Files:**
- Modify: ninguno de producto.

- [ ] **Step 1: Registrar baseline**

Run: `cargo test -p amatl-ui --locked`
Expected: `test result: ok. 29 passed`.

- [ ] **Step 2: Verificar la rama**

Run: `git branch --show-current`
Expected: `refactor/app-js`.

- [ ] **Step 3: Commit del spec enmendado y este plan**

```bash
git add docs/superpowers/specs/2026-11-18-refactor-app-js-design.md docs/superpowers/plans/2026-11-18-refactor-app-js.md
git commit -m "docs(ui): amend the app.js design for full IIFEs and record the plan"
```

---

### Task 2: Probe de navegador (la única red real)

**Files:**
- Create: `/tmp/user/1000/opencode/amatl-refactor/probe.py` (efímero, fuera del repo).

**Interfaces:**
- Produces: `python3 …/probe.py [port]` → `EXCEPTIONS: N`, `CONSOLE_ERRORS: N`, `FLOWS: {...}`. Exit 0 solo con ambos en 0. **Cada tarea 3-8 lo re-ejecuta; ninguna se cierra sin `EXCEPTIONS: 0`.**

- [ ] **Step 1: Escribir el probe**

Debe: abrir Chromium headless por CDP, suscribirse a `Runtime.exceptionThrown` + `Log.entryAdded` (error), cargar `/`, instalar un acumulador `window.onerror`/`unhandledrejection`, y recorrer: (1) buscar `rust` y contar `#results li`; (2) cambiar idioma `es→en→es` y leer `documentElement.lang`; (3) pulsar el toggle de tema y leer `data-theme`. Imprime y falla con exit≠0 si hay excepciones.

- [ ] **Step 2: Reconstruir y arrancar**

```bash
cargo build --release -p amatl-cli --locked
# servidor --mock en 127.0.0.1:8955, no_auth=true, enabled=[]
```

Los assets van con `include_bytes!`: sin reconstruir, el binario sirve el JS viejo (`audit/embedded-asset-staleness`).

- [ ] **Step 3: Confirmar el baseline limpio**

Run: `python3 /tmp/user/1000/opencode/amatl-refactor/probe.py 8955`
Expected: `EXCEPTIONS: 0`, `CONSOLE_ERRORS: 0`, `search_results >= 1`, `lang` = `es->en->es`. Si no, **parar**: el baseline está sucio y el probe no sirve de juez.

- [ ] **Step 4: Ledger, sin commit**

Registrar el resultado del baseline en el ledger. El probe no se versiona.

---

### Task 3: Spike del patrón sobre `theme` (el molde)

**Files:**
- Modify: `crates/amatl-ui/assets/app.js:5-38` (constantes + `storedTheme`/`applyTheme`), `:145-179` (`resolvedTheme`/`updateThemeToggle`, listener)

**Interfaces:**
- Consumes: `t` (núcleo, para `aria-label`), `themeToggle` (nodo global).
- Produces: nada que otros dominios consuman hoy; el arranque llama `applyTheme(storedTheme())`. `updateThemeToggle` la llama `applyTranslations` → **va al núcleo**, no al IIFE.

**Patrón que fijan las demás tareas:**
```javascript
const theme = (function () {
  const THEME_KEY = "amatl-theme";
  function storedTheme() { /* … */ }
  function applyTheme(value) { /* … */ }
  function resolvedTheme() { /* … */ }
  function updateThemeToggle() { /* … */ }
  return { storedTheme, applyTheme, resolvedTheme, updateThemeToggle };
})();
```
El núcleo expone `updateThemeToggle` al resto: `const updateThemeToggle = theme.updateThemeToggle;` **antes** de `applyTranslations`.

- [ ] **Step 1: Aplicar el patrón a `theme`**

`THEME_KEY`/`storedTheme`/`applyTheme`/`resolvedTheme`/`updateThemeToggle` dentro del IIFE; el IIFE **retorna los cuatro**. La llamada `applyTheme(storedTheme())` **sale** de la línea 29 y va al bloque de arranque (Tarea 9). El listener de click del tema también se reubica al arranque.

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed` (ninguna aserción toca símbolos de tema, pero el archivo debe seguir compilando como string).

- [ ] **Step 3: Probe**

Run: reconstruir + `python3 …/probe.py 8955`
Expected: `EXCEPTIONS: 0`; `theme` = `None->dark` en un click.

- [ ] **Step 4: Ruling y ledger**

Si el patrón funciona, anotar en el ledger: `Ruling: el molde IIFE-retorna-símbolos es viable; las tareas 4-8 lo replican`. Si falla, **parar y reconsiderar** (el molde no sirve y (b) está mal planteado).

- [ ] **Step 5: Commit**

```bash
git commit -am "refactor(ui): encapsulate the theme domain in an IIFE that returns its surface"
```

---

### Task 4: Núcleo transversal explícito

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — agrupar en la cabecera, en este orden, sin cambiar cuerpos: `CATALOG`/`FALLBACK`/`LANGUAGE_KEY`/`LANG`/`t`/`pluralKey`/`countLabel`, `boundedText`/`safeHttpUrl`/`validHash`, `statusRenderer`/`statusState`/`setStatusKey`/`rerenderStatus`/`announce`, `PAGE_SIZE`/`LIST_LIMIT`/`MAX_*`.

**Interfaces:**
- Produces: **todo** símbolo del núcleo disponible antes de cualquier dominio. Los dominios de las tareas 5-8 lo consumen sin declararlo.

- [ ] **Step 1: Reagrupar el núcleo arriba**

Mover las declaraciones a un bloque "NÚCLEO" al inicio, **conservando su texto exacto** y sin IIFE (son transversales). El DOM (85 nodos) va justo después.

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`.

- [ ] **Step 3: Probe**

Run: `python3 …/probe.py 8955`
Expected: `EXCEPTIONS: 0`.

- [ ] **Step 4: Commit**

```bash
git commit -am "refactor(ui): hoist the transversal core above the domains"
```

---

### Task 5: Dominio `net`

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `globalThis.authHeaders` (:95), `ERROR_MESSAGE_KEYS`, `reportFailure` (:650), `reportValidationError`, `clearValidationError`

**Interfaces:**
- Produces: `reportFailure`, `reportValidationError`, `clearValidationError` (los consumen `search-deep` y `render`). `globalThis.authHeaders` **queda fuera del IIFE**, como asignación de nivel superior (contrato con `client.js`).

- [ ] **Step 1: Aplicar el patrón**

IIFE que retorna `{ reportFailure, reportValidationError, clearValidationError }`. El literal `function reportFailure(error) {` **no cambia**.

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`, incl. `report_failure_reads_only_error_properties_never_a_bare_message`.

- [ ] **Step 3: Probe**

Expected: `EXCEPTIONS: 0`.

- [ ] **Step 4: Commit**

```bash
git commit -am "refactor(ui): encapsulate the net domain behind an IIFE"
```

---

### Task 6: Dominio `render`

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `renderStatus` (:235), `renderSearchResult` (:324), `renderEvidenceFragment` (:444), `renderDeepDocument` (:484), `renderAnswerCard` (:565), `render` (:576), `deepItems`, `documentStatus`, `acquisitionMethod`, `signalLabel`, `appendSafeUrl`, `verifyFragment`, `setVerification`

**Interfaces:**
- Consumes: `t`, `boundedText`, `safeHttpUrl`, `setStatusKey` (núcleo).
- Produces: `render` (lo llama `store.subscribe` en el arranque), `deepItems` (lo usa `validatePayload` de `search-deep`).

- [ ] **Step 1: Aplicar el patrón**

IIFE que retorna `{ render, deepItems }` (y lo que `search-deep` consuma; comprobar con grep antes de cerrar el retorno).

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`.

- [ ] **Step 3: Probe**

Expected: `EXCEPTIONS: 0`; `search_results >= 1` (el render pinta).

- [ ] **Step 4: Commit**

```bash
git commit -am "refactor(ui): encapsulate the render domain behind an IIFE"
```

---

### Task 7: Dominio `search-deep` (con `location`)

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `setBusy`, `queryText`, `validatePayload`, `focusTokenField`, `run`, `execute`, `requestedModeIsUnavailable`, `searchLocation`, `syncLocation`, `restoreFromLocation`

**Interfaces:**
- Consumes: `globalThis.api`, `store`, `t`, `boundedText`, `validHash`, `reportFailure`, `render`/`deepItems`, `syncLocation`.
- Produces: `execute` (submit del formulario), `restoreFromLocation` (arranque), `syncLocation`.

Ruling del plan: `location` se **funde** con `search-deep` — `run()` es su único llamador. Se salta una tarea.

- [ ] **Step 1: Aplicar el patrón**

Un IIFE que retorna `{ execute, restoreFromLocation, syncLocation, queryText, setBusy }` según lo que el arranque y `render` consuman (grep antes de cerrar).

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed` (`search_and_deep_use_post_contracts_and_safe_dom_apis`).

- [ ] **Step 3: Probe**

Expected: `EXCEPTIONS: 0`; buscar pinta, y `/?q=rust&mode=search` restaura con la UI **traducida**.

- [ ] **Step 4: Commit**

```bash
git commit -am "refactor(ui): encapsulate the search-deep and location domains behind one IIFE"
```

---

### Task 8: Dominios `providers`, `storage-panels` y `clients`

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` — `providerReason`…`saveMarginaliaKey`; `historyLabel`…`saveDocument`; `scopeLabel`…`rotateServerClient`

**Interfaces:**
- Produces: los **7** símbolos que cruzan: `fetchProviderStatus`, `fetchServiceState`, `fetchHistory`, `fetchSavedDocuments`, `fetchServerClients` (los cinco de `refreshPanels`), `saveDocument` (usado dentro de `renderDomain`) y `scopeLabel` (usado en `applyTranslations`).

**Complejidad medida:** dentro del rango 1004-1933 hay **48 declaraciones de nodos DOM**
(`const x = document.querySelector(...)`) intercaladas. La regla D2 las quiere globales de
solo lectura, así que el bloque no es un rango limpio: el script debe **clasificar línea a
línea** y dejar los nodos fuera del IIFE. Verificado: ningún nodo se usa en carga antes de
declararse, así que consolidarlos es seguro.

- [ ] **Step 1: Consolidar los 85 nodos DOM**

Mover todas las líneas `const <nodo> = document.querySelector(...)` (y los no-`querySelector`,
p. ej. `document.querySelector("details.filters")` con comillas dobles) a un bloque único
"DOM" justo después del núcleo, conservando su texto exacto. Verificar con `order_check.py`
y `node --check`.

- [ ] **Step 2: `providers`**

IIFE que retorna `{ fetchProviderStatus, fetchServiceState, providerReason, toggleProvider, … }`
— el script decide el conjunto por los usos externos medidos.

- [ ] **Step 3: `storage-panels`**

IIFE que retorna `{ fetchHistory, fetchSavedDocuments, saveDocument, purgeHistory, … }`.
`historyLabel` conserva `countLabel(entry.total_results, "resultsCount")` exacto.

- [ ] **Step 4: `clients`**

IIFE que retorna `{ fetchServerClients, scopeLabel, … }`. `MCP_TOOL_NAMES` conserva su texto.

- [ ] **Step 5: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`.

- [ ] **Step 6: Probe**

Expected: `EXCEPTIONS: 0`; los paneles respetan el gating de almacenamiento.

- [ ] **Step 7: Commit**

```bash
git commit -am "refactor(ui): consolidate the DOM block and encapsulate the remaining domains"
```

---

### Task 9: Bloque de arranque, guard de símbolos y cierre

**Files:**
- Modify: `crates/amatl-ui/assets/app.js` (bloque final; `refreshPanels` en :1894), `crates/amatl-ui/src/lib.rs` (test nuevo)

**Interfaces:**
- Consumes: todo lo retornado por las tareas 3-8.

- [ ] **Step 1: Reescribir el arranque**

Al final del archivo, en este orden exacto:
1. Asignaciones de núcleo que dependen de dominios (`const updateThemeToggle = theme.updateThemeToggle;`).
2. `applyTheme(storedTheme())` (antes vivía en la línea 29).
3. `applyDeclarativeTranslations()` + `applyTranslations()`.
4. `refreshPanels()`; listeners de nivel superior (`historyPurgeButton`, `tokenInput`, `store.subscribe(render)`, toggles de tema/idioma).
5. `document.addEventListener("DOMContentLoaded", …)` con `uiLanguage.value = LANG` → `applyTranslations()` → `setStatusKey("initialStatus","")` → `restoreFromLocation()`.

- [ ] **Step 2: Test de contrato**

Run: `cargo test -p amatl-ui --locked`
Expected: `29 passed`.

- [ ] **Step 3: Test nuevo — símbolos del núcleo no sombreados ni ausentes**

Añadir a `crates/amatl-ui/src/lib.rs`:

```rust
#[test]
fn core_symbols_are_declared_once_and_not_shadowed() {
    let javascript = text("/app.js");
    assert_eq!(javascript.matches("function t(key)").count(), 1);
    assert_eq!(javascript.matches("function boundedText(").count(), 1);
    assert_eq!(javascript.matches("function setStatusKey(").count(), 1);
    assert_eq!(javascript.matches("function refreshPanels(").count(), 1);
    // El arranque corre al final: `refreshPanels()` no aparece antes del
    // último dominio declarado.
    let last_domain = javascript.rfind("})();").expect("domains are IIFEs");
    let startup = javascript.rfind("refreshPanels();").expect("startup calls refreshPanels");
    assert!(startup > last_domain, "startup must follow every domain");
}
```

- [ ] **Step 4: Correr el test nuevo**

Run: `cargo test -p amatl-ui --locked core_symbols_are_declared_once`
Expected: PASS.

- [ ] **Step 5: Probe completo**

Run: reconstruir + `python3 …/probe.py 8955`
Expected: `EXCEPTIONS: 0`, `CONSOLE_ERRORS: 0`, los cinco flujos responden **una sola vez**.

- [ ] **Step 6: Suite completa**

Run: `cargo test --workspace --locked`
Expected: verde, con 30 tests en `amatl-ui` (29 + 1 nuevo).

- [ ] **Step 7: Commit**

```bash
git add crates/amatl-ui/assets/app.js crates/amatl-ui/src/lib.rs
git commit -m "refactor(ui): coordinate the domains from a final startup block"
```

---

### Task 10: Cierre — comparación contra baseline

- [ ] **Step 1: Comparar con el baseline de la Tarea 2**

Mismos valores esperados: `EXCEPTIONS: 0`, `CONSOLE_ERRORS: 0`, `search_results` igual, `lang` = `es->en->es`, tema cambia en un click.

- [ ] **Step 2: Reducción de superficie global**

Run: `grep -cE "^(async )?function [a-zA-Z]" crates/amatl-ui/assets/app.js`
Expected: muchas menos de las 86 iniciales (las que entraron en IIFEs). Anotar el número exacto y la diferencia.

- [ ] **Step 3: Dictamen en pantalla**

Reportar: tareas completadas, excepciones en navegador, tests, reducción de superficie, y cualquier desviación del plan con su ruling.
