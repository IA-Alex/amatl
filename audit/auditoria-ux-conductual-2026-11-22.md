# Auditoría UX conductual — AMATL (UI web) — 2026-11-22

Pipeline adversarial `Hunter → Skeptic → Referee`, con política *fail-closed*.
Reproducción **empírica en navegador real** (Playwright MCP, Chromium 1252) contra
el binario release en `./target/release/amatl serve --mock` en
`127.0.0.1:8080` (`no_auth = true`, `persistence.enabled = false`).

- Fuente: `/home/alonso/amatl`, HEAD `8558a922`, árbol de trabajo limpio.
- Vistas auditadas: `crates/amatl-ui/assets/{index.html,app.js,styles.css,client.js,store.js}`.
- Método: por cada hallazgo, secuencia *Trigger → State → Feedback* verificada en
  vivo; los `static` se citan con archivo:línea; los `behavioral` se reprodujeron
  cuando fue posible.
- Read-only: no se modificó código.

> Alcance: esta skill cubre **ergonomía, flujo y modelo mental**. Accesibilidad
> WCAG, rendimiento/CWV y seguridad de cliente se delegan a `frontend-audit`.

---

## 1. Resumen ejecutivo y cobertura

- **Alcance:** 1 vista (`/`), 5 `<dialog>`/`<template>`, formulario de búsqueda,
  flujo Search, flujo Deep (evidencia), panel de operación (fuente/servicio/historial/guardados/clientes).
- **Modo de evidencia predominante:** mixto — `static` (código) + `behavioral`
  (reproducción en vivo).
- **Totales:** `CONFIRMED` = 2 (1 ALTA, 1 MEDIA) · `HARDENING` = 2 · `MANUAL_REVIEW` = 2.
- **Puntos ciegos:** `persistence.enabled = false` en este entorno → historial y
  guardados no se pudieron ejercitar en vivo (quedan como `MANUAL_REVIEW` con la
  evidencia estática disponible). Verificación de contraste/tap-target proporcional
  (se mide en `frontend-audit`).

El conjunto ya fue auditado y refinado antes (`audit/auditoria-ux-*.md`,
`cierre-hallazgos-2026-11-18.md`). Esta pasada reproduce el *happy path* (Search,
Deep, i18n, tema, URL restore, doble submit) **sin regresiones** y concentra los
hallazgos nuevos en la superficie de **clientes/tokens**, que creció después de
aquellas auditorías.

---

## 2. Tabla de hallazgos

| ID | Veredicto | Severidad | Modo | Eje | Ubicación | Resumen (1 línea) |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `UX-001` | CONFIRMED | ALTA | static+behavioral | state_flow | `app.js:1607-1610` + `index.html:293-302` | Cerrar el diálogo del token (Escape/backdrop) descarta un secreto de un solo uso sin confirmación ni recuperación |
| `UX-002` | CONFIRMED | MEDIA | static | state_flow | `app.js:1529-1569` | `Rotar token` dentro de la fila no ofrece ver el token nuevo si el diálogo se cierra sin copiar; el aviso de irreversibilidad vive solo en el diálogo, no en el botón |
| `UX-003` | HARDENING | BAJA | static+behavioral | edge_case_ux | `app.js:658-659`, `311-336` | La selección de objetivos Deep se descarta en silencio al refinar y volver a buscar |
| `UX-004` | HARDENING | BAJA | static+behavioral | state_flow | `app.js:534-536`, `1443-1491` | El botón `Guardar` sigue habilitado aunque `/status` ya sabe que el almacenamiento está desactivado |
| `UX-005` | MANUAL_REVIEW | MEDIA | static | feedback_latency | `app.js:1302-1352` | El resultado de `history-reuse`/`history-delete`/`purge` no se pudo reproducir (storage off); falta confirmar el feedback en vivo |
| `UX-006` | MANUAL_REVIEW | BAJA | behavioral | cognitive_load | `index.html:137-204` | Densidad del panel de operación con historial+guardados+clientes activos no ejercitable sin storage |

---

## 3. Evidencia técnica y conductual detallada

### [UX-001] Cerrar el diálogo del token destruye un secreto irrecuperable sin confirmación

- **Eje / Modo / Ubicación:** `state_flow` · `static` + `behavioral` ·
  `crates/amatl-ui/assets/app.js:1602-1610` (listener `close`) y
  `index.html:293-302` (`<dialog id="client-token-dialog">`).
- **Secuencia de fallo (*Trigger → State → Feedback*):**
  1. *Trigger*: el operador crea un cliente (`#client-create-form`) o pulsa
     `Rotar token`. El diálogo se abre con el token en claro (64 hex), `readonly`.
  2. *State*: el `<dialog>` es modal nativo. Pulsar **Escape** (o clic en el
     backdrop) lo cierra; el listener `close` ejecuta
     `clientTokenValue.value = ""` (línea 1608).
  3. *Feedback*: ninguna confirmación, ningún aviso. El token queda en blanco y
     **no existe ningún control para volver a mostrarlo** — solo `Rotar token`
     (que invalida el primero) o `Eliminar`.
- **Impacto conductual:** pérdida silenciosa e irreversible de un secreto de un
  solo uso. Un Escape accidental (tecla contigua, costumbre de cerrar diálogos)
  obliga a rotar el token, y si el cliente ya se distribuyó, a re-distribuirlo.
  Contraste directo: las acciones destructivas *vecinas* (`Rotar token`,
  `Eliminar`) **sí** piden `window.confirm(...)` con la consecuencia explícita
  (`app.js:1693-1694`, `1706-1707`) — probado en vivo: `"Generate a new token?
  The old one stops working immediately."`. Cerrar el token es igual de
  destructivo y carece de esa red.
- **Nota del Skeptic / Referee:** *no refutado*. El Skeptic consideró "es
  intencional no persistir el secreto" — cierto para el *transporte* (nunca va a
  `localStorage`/red), pero no exime de confirmar una acción destructiva. La
  reproducción empírica (`Escape` → `{open:false, tokenValue:"", confirmCalls:[]}`)
  confirma el fallo. Sobrevive como `CONFIRMED`/ALTA.
- **Variantes detectadas:** `Rotar token` reutiliza el mismo diálogo y hereda el
  defecto (`app.js:1706-1722`).

### [UX-002] El aviso de irreversibilidad vive solo dentro del diálogo, no junto al disparador

- **Eje / Modo / Ubicación:** `state_flow` · `static` ·
  `app.js:1556-1566` (botón `Rotate`) y `index.html:295` (copy del aviso).
- **Secuencia:** *Trigger*: el operador pulsa `Rotar token` con la expectativa de
  "ver el token actual o uno nuevo". *State*: se genera y muestra un token nuevo
  en el diálogo; el anterior ya es inválido. *Feedback*: el único texto que
  advierte "no volverá a mostrarse" está **dentro** del diálogo, después del
  `confirm`, no en la fila que contiene el botón.
- **Impacto conductual:** el modelo mental "rotar = puedo volver a verlo" choca
  con la realidad "solo tengo esta ventana". Es la causa raíz de UX-001 agravada:
  el operador aprende la irreversibilidad después, no antes.
- **Nota del Skeptic / Referee:** sobrevive a la refutación (el copy existe, pero
  está mal ubicado para el momento de decisión). `CONFIRMED`/MEDIA — es un
  problema de *placement* de información, no de ausencia.
- **Variantes:** `#marginalia-key-button` ("Insert Marginalia key") no rota, pero
  comparte el patrón de secreto; no aplica.

### [UX-003] La selección de objetivos Deep se descarta al re-buscar (fricción repetitiva)

- **Eje / Modo / Ubicación:** `edge_case_ux` · `static` + `behavioral` ·
  `app.js:658-659` (`if (mode === "search") store.update({ selectedTargets: [] })`).
- **Secuencia:** *Trigger*: marcar `.deep-target-checkbox` de uno o varios
  resultados y luego refinar la consulta y volver a `Buscar`. *State*:
  `selectedTargets` se vacía. *Feedback*: ninguna marca de que la selección previa
  se perdió; los checkboxes reaparecen sin marcar.
- **Impacto conductual:** fricción en uso repetitivo: quien selecciona evidencia y
  ajusta términos debe re-seleccionar. Reproducido: selección 1 → re-search →
  `selectedAfterResearch: 0`.
- **Nota del Skeptic / Referee:** refutado parcialmente como *defecto* — la
  selección referencia URLs concretas del conjunto anterior, que cambia con la
  nueva consulta; limpiarla evita apuntar a resultados que ya no están. Por eso es
  `HARDENING`/BAJA, no `CONFIRMED`: la mejora es *conservar los que coincidan por
  URL* o avisar de la limpieza, no eliminar la limpieza.
- **Variantes:** ninguna.

### [UX-004] `Guardar` habilitado con el almacenamiento ya reportado como desactivado

- **Eje / Modo / Ubicación:** `state_flow` · `static` + `behavioral` ·
  `app.js:534-536` (render del botón) y `1443-1491` (`saveDocument`).
- **Secuencia:** *Trigger*: pulsar `Guardar` en una tarjeta de evidencia cuando
  `#service-summary` ya dice "el almacenamiento está desactivado, así que no se
  pueden guardar documentos". *State*: `POST /saved` → 503. *Feedback*:
  `savedFailedStorage` inline y en `#status`, botón pasa a "Volver a guardar".
  Reproducido en vivo; el manejo del error es **excelente**.
- **Impacto conductual:** el control se ofrece como "listo" cuando la app ya sabe
  que no puede tener éxito; el operador hace un clic que sabe perdido de antemano.
- **Nota del Skeptic / Referee:** el Skeptic **refuta** el "fallo" (el comentario
  en `app.js:1481-1490` documenta que se deja habilitado a propósito, para que el
  operador pueda reintentar en cuanto vuelva el storage, sin recargar). Sobrevive
  solo como `HARDENING`/BAJA: deshabilitar con el mismo mensaje *ya visible*
  elimina el clic inútil sin perder la capacidad de reintento.
- **Variantes:** ninguna.

### [UX-005] Feedback del historial no reproducible (storage off) — `MANUAL_REVIEW`

- **Eje / Modo / Ubicación:** `feedback_latency` · `static` ·
  `app.js:1302-1352` (`fetchHistory`, `deleteHistoryEntry`, `purgeHistory`).
- **Por qué falta contexto:** `persistence.enabled = false` en el entorno; la UI
  oculta `.history-panel` (verificado: `hidden:true`). El flujo `history-reuse`
  **rellena** la consulta pero **no relanza** la búsqueda (`app.js:1317-1320`),
  mientras `audit/auditoria-ux-2026-11-14.md` reportaba "rellena y **relanza**".
  **Esa discrepancia debe confirmarse en vivo** con storage activo antes de
  emitir veredicto.
- **Nota del Skeptic / Referee:** `MANUAL_REVIEW` — se requiere
  `persistence.enabled = true` en un entorno de prueba, no asumible desde aquí.

### [UX-006] Densidad del panel de operación — `MANUAL_REVIEW`

- **Eje / Modo / Ubicación:** `cognitive_load` · `behavioral` ·
  `index.html:137-204`.
- **Por qué falta contexto:** con storage activo y varios clientes, el panel
  reúne fuente + servicio + historial + guardados + administración de clientes en
  un solo `<details>`. La carga cognitiva percibida es `behavioral` y no se puede
  confirmar sin el estado completo; además es herramienta técnica de alta densidad
  (Principio 5 de proporcionalidad): no se propone simplificar sin ganancia
  conductual demostrable.

---

## 4. Estado de reparación (2026-11-23)

Las cuatro reparaciones propuestas se aplicaron y verificaron en navegador real
(rebuild release + `serve --mock`), con la suite completa en verde
(`cargo test --workspace --locked` 0 fallos; `cargo clippy --all-targets -D
warnings` limpio).

| ID | Estado | Cambio | Verificación en vivo |
| :--- | :--- | :--- | :--- |
| `UX-001` | **CERRADO** | `app.js`: guardián de cierre con `tokenCopied`; `cancel` (Escape) y backdrop enrutan por `closeClientTokenDialog()`; clave `clientTokenCloseConfirm` en `i18n.js` (EN/ES) | Escape con token sin copiar → pide confirmación y **conserva** el token (`open:true, tokenLen:64, confirmCalls:1`); tras copiar → cierra **sin** confirmación (`confirmCalls:[]`); aceptar → cierra |
| `UX-002` | **CERRADO** | `app.js`: `rotate.title = t("clientRotateConfirm")` en cada fila | `rotateTitle` = "Generate a new token? The old one stops working immediately." |
| `UX-003` | **CERRADO** | `app.js`: la selección Deep se reconcilia con las URLs visibles al comprometer la búsqueda (en vez de vaciarse al inicio) | `alpha`→selección 1→`beta` (misma URL) → `selectedAfterResearch:1`, checkbox restaurado |
| `UX-004` | **CERRADO** | `app.js`: flag `storageReady` desde `/status`; `saveButton.disabled = !storageReady` + `title` | Con storage off → `disabled:true`, `title` explicativo |

`UX-005`/`UX-006` siguen en `MANUAL_REVIEW` (requieren `persistence.enabled=true`).

## 5. Contrato JSON para Remediación (Handoff)

```json
{
  "audit_status": "partial_context",
  "findings": [
    {
      "id": "UX-001",
      "verdict": "CONFIRMED",
      "severity": "CRITICAL",
      "evidence_mode": "static",
      "axis": "state_flow",
      "file": "crates/amatl-ui/assets/app.js",
      "lines": "1602-1610",
      "category": "silent_data_loss",
      "root_cause": "El listener `close` del diálogo del token borra el valor antes de cualquier confirmación, y Escape/backdrop cierran el <dialog> nativo sin preguntar, destruyendo un secreto de un solo uso irrecuperable.",
      "trigger": "Crear cliente o Rotar token -> pulsar Escape (o clic en backdrop) -> el token de 64 hex queda vacío y sin forma de re-mostrarlo",
      "variants": ["crates/amatl-ui/assets/app.js:1706-1722"]
    },
    {
      "id": "UX-002",
      "verdict": "CONFIRMED",
      "severity": "MEDIUM",
      "evidence_mode": "static",
      "axis": "state_flow",
      "file": "crates/amatl-ui/assets/index.html",
      "lines": "293-302",
      "category": "misplaced_disclosure",
      "root_cause": "El aviso de irreversibilidad del token solo aparece dentro del diálogo, después de que el usuario ya disparó la acción, no junto al botón que la inicia.",
      "trigger": "Pulsar `Rotar token` en una fila de cliente",
      "variants": []
    },
    {
      "id": "UX-003",
      "verdict": "HARDENING",
      "severity": "LOW",
      "evidence_mode": "static",
      "axis": "edge_case_ux",
      "file": "crates/amatl-ui/assets/app.js",
      "lines": "658-659",
      "category": "repetitive_friction",
      "root_cause": "Cada búsqueda vacía `selectedTargets` sin conservar los que coincidan por URL ni avisar al operador.",
      "trigger": "Marcar objetivo Deep -> cambiar la consulta -> Buscar",
      "variants": []
    },
    {
      "id": "UX-004",
      "verdict": "HARDENING",
      "severity": "LOW",
      "evidence_mode": "static",
      "axis": "state_flow",
      "file": "crates/amatl-ui/assets/app.js",
      "lines": "534-536",
      "category": "dead_affordance",
      "root_cause": "El botón Guardar se renderiza habilitado aunque `/status` ya reporta `storage.enabled=false`, ofreciendo una acción que el cliente sabe que fallará.",
      "trigger": "Servicio con almacenamiento desactivado -> Guardar en una tarjeta de evidencia",
      "variants": []
    }
  ],
  "manual_review_required": [
    {
      "id": "UX-005",
      "file": "crates/amatl-ui/assets/app.js:1317-1320",
      "missing_context": "Entorno con `persistence.enabled = true` para confirmar si `history-reuse` relanza la búsqueda (la auditoría de 2026-11-14 decía que sí; el código actual solo rellena la consulta)."
    },
    {
      "id": "UX-006",
      "file": "crates/amatl-ui/assets/index.html:137-204",
      "missing_context": "Estado completo del panel de operación (storage activo + varios clientes) para evaluar carga cognitiva percibida sin violar el Principio 5 (proporcionalidad de herramienta técnica)."
    }
  ]
}
```
