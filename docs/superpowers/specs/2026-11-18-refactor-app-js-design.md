# Diseño — Refactor interno de `app.js` (approach B)

Fecha: 2026-11-18
Estado: aprobado por el operador (approach B), pendiente de plan de implementación.
Hallazgo de origen: ninguno; es trabajo de mantenibilidad, no una reparación.

## Intención y el porqué

`crates/amatl-ui/assets/app.js` tiene **1992 líneas y 86 funciones top-level** en un
único scope de módulo (script clásico, no módulo ES). Conviven responsabilidades
distintas — i18n, tema, llamadas HTTP, render, búsqueda/deep, proveedores,
historial, guardados, clientes MCP, ubicación/URL — sin fronteras: cualquier
función puede leer y mutar cualquier `let`/nodo global.

El dolor real es de **mantenibilidad y superficie de error**: un cambio en un
dominio no tiene forma de saber si toca otro, porque todo comparte scope. No es
un problema de aspecto ni de rendimiento.

## Objetivo

Reorganizar `app.js` en **dominios delimitados dentro del mismo archivo**,
encapsulando el estado y reduciendo la superficie global, **sin cambiar el
comportamiento observable** (mismo DOM, mismo HTML, mismos strings, mismos
endpoints).

## No-objetivos

- No introducir módulos ES, bundler, ni `<script type="module">`.
- No tocar `asset()` (`crates/amatl-ui/src/lib.rs`), `index.html`, `styles.css`,
  `client.js`, `store.js`, `i18n.js`.
- No adoptar librerías de UI (Open Props, VanJS, Oat, etc.).
- No cambiar nombres de funciones/constantes que los tests verifican por string.
- No rediseñar la interfaz.

## Anti-objetivos (harían fracasar esto aunque "funcione")

- Romper alguna de las **85 aserciones** que leen `text("/app.js")` y buscan
  substrings literales.
- Alterar el orden de ejecución observable (carga, `DOMContentLoaded`,
  `restoreFromLocation`).
- Introducir estado compartido nuevo entre dominios.

## Restricciones (verificadas)

| Restricción | Evidencia | Efecto en el diseño |
|---|---|---|
| CSP `script-src 'self'`, sin inline | `lib.rs:7` | Sin bundles inline; `app.js` sigue siendo un `<script defer>` clásico |
| `asset()` es allowlist de rutas exactas | `lib.rs:25-65` | **No se añaden archivos**; todo vive en `app.js` |
| Test `!html.contains("<script>")` | `lib.rs:183` | No se añade ningún `<script>` al HTML |
| 85 aserciones leen strings de `app.js` | `lib.rs` (`text("/app.js")`) | Los símbolos verificados **permanecen en `app.js` con el mismo texto** |
| i18n con paridad es/en | `i18n.js` + tests | No se añaden claves nuevas; no se renombran las existentes |

## Approach elegido (B) y alternativas descartadas

**Elegido: B — refactor interno.**
Delimitar dominios en `app.js` con secciones e IIFEs, encapsulando estado, sin
mover símbolos a otros archivos.

**Descartado: A — módulos ES reales.**
Mover funciones a archivos de módulo (`search`, `deep`, …) requeriría: añadir
~6 rutas a `asset()`, cambiar `index.html` a `type="module"` y **reescribir las
85 aserciones** para que lean del archivo correcto. Coste y superficie de riesgo
altos para un beneficio que B ya cubre en gran parte.

**Descartado: C — híbrido (solo lo puro).**
Extraer i18n/URL-safety/validación a 1-2 módulos rompe las aserciones de esos
símbolos igual que A; menos grave, pero paga el mismo tipo de precio sin abordar
el grueso de la estructura.

## Cómo queda B

### Inventario de estado global actual (lo que hay que encapsular)

- **Nodos DOM**: 85 `const x = document.querySelector(...)` top-level.
- **Listeners**: 23 `addEventListener` top-level.
- **Constantes de política**: `PAGE_SIZE`, `MAX_DEEP_DOCUMENTS`, `MAX_FRAGMENTS`,
  `MAX_FRAGMENT_BYTES`, `MAX_DOCUMENT_BYTES`, `MAX_SAVED_PAYLOAD_BYTES`,
  `LIST_LIMIT`.
- **Estado mutable**: `searchAvailable`, `statusRenderer`, `statusState`, `LANG`.
- **Mapas**: `PROVIDER_STATUS_CLASSES`, `ERROR_MESSAGE_KEYS`.

### Dominios objetivo (delimitados, no extraídos a archivos)

Núcleo transversal (solo lectura, ver D1): `i18n` (`t`, `pluralKey`,
`countLabel`), utilidades (`boundedText`, `safeHttpUrl`, `validHash`),
barra de estado (`setStatusKey` + estado asociado), `announce`,
`refreshPanels`, constantes de política. Captura de nodos DOM (ver D2).

Dominios:

1. `theme` — `storedTheme`, `applyTheme`, `resolvedTheme`, `updateThemeToggle`.
2. `net` — envoltura de `globalThis.api` y clasificación de errores.
3. `render` — `render`, `renderStatus`, plantillas, tarjetas.
4. `search-deep` — `run`, `execute`, paginación, validación de payload.
5. `providers` — estado de proveedores, toggles, credencial Marginalia.
6. `storage-panels` — historial y guardados.
7. `clients` — clientes de servidor y su vocabulario MCP.
8. `location` — `searchLocation`, `syncLocation`, `restoreFromLocation`
   (candidato a agruparse con `search-deep`, que es quien lo llama).

### Garantías de la refactorización

- **Comportamiento idéntico**: mismo DOM, mismos strings verificados, mismo
  orden de inicialización.
- **Símbolos verificados intactos**: los nombres y el texto que las 85
  aserciones buscan no cambian.
- **Sin archivos nuevos, sin rutas nuevas en `asset()`, sin tocar el HTML.**
- **Estado encapsulado**: lo que hoy es global pasa a ser local del dominio
  cuando ningún otro dominio lo necesita; lo compartido se declara explícito.

### Decisión D1 — Símbolos compartidos (núcleo transversal)

La afirmación "no se mueven símbolos" no aplica a un conjunto pequeño de
símbolos **transversales** que casi todos los dominios usan. Éstos **no se
encapsulan**: permanecen en un núcleo de nivel superior, declarado primero, y
los dominios solo los **leen**.

Núcleo transversal (evidencia: recuento de usos en `app.js`):

| Símbolo | Usos | Naturaleza |
|---|---|---|
| `t` | ~174 | catálogo i18n |
| `boundedText` | 31 | utilidad de acotado |
| `setStatusKey` + `statusNode`/`statusState`/`statusRenderer`/`rerenderStatus` | ~20 | barra de estado compartida |
| `announce` | 12–16 | accesibilidad (aria-live) |
| `validHash` | 8 | validación |
| `safeHttpUrl` | 6 | allowlist de URLs |
| `countLabel`/`pluralKey` | 6–7 | i18n de conteo |
| `refreshPanels` | 5 dominios | orquestador de refresco |
| `PAGE_SIZE`, `LIST_LIMIT`, `MAX_*` | — | constantes de política |

Regla: **solo lectura desde los dominios**; ninguno de estos símbolos se
re-declara ni se sombrea dentro de un dominio. Los dominios encapsulan lo suyo
(estado propio, nodos propios, lógica propia) y leen el núcleo.

### Decisión D2 — Nodos DOM: globales, no por dominio

Los 85 `querySelector` de nivel superior (hoy intercalados: `app.js:93-129`,
`942-944`, `1065-1068`, `1311-1321`, `1435-1439`, …) **permanecen globales de
solo lectura**. No se re-consultan por dominio ni se mueven a IIFEs.

Motivo: re-consultar cambiaría el **momento de captura** (hoy, antes del
primer `render`) y el DOM se re-renderiza; capturar tarde devolvería nodos
distintos. Se agrupan visualmente en un bloque "DOM" al inicio, pero siguen
siendo `const` de nivel superior.

Consecuencia: **`dom` no es un dominio encapsulado**; es el bloque de captura
del núcleo. Desaparece como IIFE.

### Decisión D3 — Regla de orden y verificación real (TDZ)

Riesgo que **la suite no cubre**: las 85 aserciones hacen `contains` de strings
y **no ejecutan JS** (el propio repo lo dice: `lib.rs:533-534`, "assert string
presence rather than JS semantics and the CI pipeline ships no JS linter"). Un
`ReferenceError` por zona muerta temporal (TDZ), una IIFE que atrapa mal una
referencia o un doble `addEventListener` **pasarían `cargo test` igual**.

Reglas obligatorias del refactor:

1. **Todo dominio se auto-contiene**: una IIFE solo puede leer símbolos
   **declarados antes** que ella. Nada de capturar `const` declarados después.
2. **Los `addEventListener` viven en un único bloque de arranque**, al final
   del archivo (donde hoy están los últimos, `app.js:1899-1901`). No se
   registran listeners dentro de las IIFEs de dominio.
3. **El orden de los bloques preserva el actual**: núcleo/i18n → DOM →
   constantes → dominios → arranque (`refreshPanels()` + `DOMContentLoaded`).
4. **`refreshPanels` y `refreshPanels`-style orquestadores** se declaran en el
   núcleo o en un bloque explícito del que todos puedan leerlos.

Verificación **real** (obligatoria, además de la suite):

- **Consola limpia en navegador real**: cargar `/` con el binario release
  reconstruido y confirmar **cero** `Runtime.exceptionThrown` / errores de
  consola (CDP directo, ya usado en la auditoría del 2026-11-18).
- **`window.onerror` / `unhandledrejection` vacíos** durante la carga.
- **Recorrido de los flujos principales** (búsqueda, deep, historial,
  guardados, providers, clientes, cambio de idioma, tema) con criterio
  observable: mismo DOM final que antes del refactor y consola limpia.
- **Sin listeners duplicados**: comprobar que un cambio de idioma se aplica
  una sola vez y que los toggles responden una sola vez.

### Cómo se verifica (además de lo anterior)

- `cargo test -p amatl-ui --locked`: los 27 tests con las 85 aserciones deben
  seguir verdes (necesario, **no suficiente**).
- `cargo test --workspace --locked` como red de seguridad.
- Reconstruir el binario antes de la verificación en navegador (riesgo de
  assets embebidos obsoletos, memoria `audit/embedded-asset-staleness`).

## Qué queda a criterio del implementador

- El número exacto de IIFEs/bloques y su orden en el archivo.
- Si algún dominio pequeño se agrupa con otro mayor (p. ej. `location` con
  `search-deep`).
- El estilo de los comentarios de sección (respetando que los comentarios de
  intención van en español, según las convenciones del repo).
- Si conviene un objeto de estado único por dominio o varios `let` locales al
  bloque.

## Riesgo y mitigación

- **Riesgo principal (D3)**: romper el orden de ejecución — TDZ, referencia
  atrapada en una IIFE, o listener duplicado — y que **la suite no lo detecte**
  porque solo compara strings. Mitigación: reglas D3 (dominios autocontenidos,
  listeners en el bloque de arranque) + verificación en navegador real con
  consola limpia; la suite es necesaria pero no suficiente.
- **Riesgo secundario (D1)**: que un dominio re-declare o sombree un símbolo
  transversal (`t`, `setStatusKey`, `boundedText`…). Mitigación: el núcleo se
  declara primero y solo se lee; ningún dominio lo re-declara.
- **Riesgo (D2)**: re-consultar nodos dentro de un dominio cambiaría el momento
  de captura y devolvería nodos distintos tras un re-render. Mitigación: los 85
  nodos permanecen globales de solo lectura.
- **Riesgo de alcance**: que "ya que estamos" se cuele un cambio de
  comportamiento. Mitigación: el plan se ejecuta en bloques pequeños, cada uno
  con la suite verde **y** la verificación en navegador antes de seguir.
