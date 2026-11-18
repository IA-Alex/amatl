# Cierre de hallazgos — auditoría conector-fullstack 2026-11-18

Cierra los dos hallazgos de `audit/auditoria-conector-fullstack-2026-11-18.md`
(H-01 alto, H-02 crítico) y la observación H-03. Es un registro de reparación, no
una auditoría nueva: la evidencia se tomó en el estado reparado.

- Fuente: `/home/panda/amatl`.
- HEAD al cerrar: `5402104e` (`fix(ui): drop the dead selector query that threw
  on every page load`). El commit que cierra H-01 llegó a HEAD durante esta
  misma sesión de reparación.
- Método: skill `reparador-integral-es` (baseline → causa raíz → mapa de impacto
  → reparación → validación en capas), con Serena para la edición simbólica.

---

## Resumen

| Hallazgo | Severidad | Estado | Autor del arreglo |
|---|---|---|---|
| H-01 — `applyTranslations()` consulta un elemento eliminado | alto | **CERRADO** | commit `5402104e` (no esta sesión) |
| H-02 — `amatl.toml` habilita `searxng` retirado | crítico | **CERRADO** | esta sesión (archivo gitignored) |
| H-03 — test de retirada no cubría consultas JS al selector | bajo | **CERRADO** | commit `5402104e` |

---

## H-01 — [ALTO] `TypeError` en cada carga de página

- **Causa raíz**: `applyTranslations()` seguía consultando
  `.provider-selector-option input[value="all"]`, un elemento que el retiro del
  selector de fuentes (`bb23a9fc`) había eliminado del markup. `querySelector`
  devolvía `null` y el `.closest("label")` encadenado lanzaba en cada
  `DOMContentLoaded`, abortando el resto del manejador.
- **Evidencia del cierre**: [ESTÁTICO] `git show 5402104e --stat` →
  `app.js` (-4 líneas, las tres consultas huérfanas) y `src/lib.rs` (+8). El
  test `retired_source_selector_is_gone_and_cannot_return` ahora también prohíbe
  `provider-selector` / `providerSelector` / `provider-source` en `app.js`.
  [EJECUCIÓN] `cargo test --workspace --locked` → 706 tests, 0 fallos.
- **No duplicado**: esta sesión no modificó `app.js`; el arreglo ya estaba en
  HEAD al iniciar la reparación.

## H-02 — [CRÍTICO] `amatl.toml` habilita una fuente que ya no existe

- **Causa raíz**: el retiro de SearXNG (`bb23a9fc`) eliminó el adapter
  (`providers/searxng.rs`) y su factoría registrada, pero el `amatl.toml` de
  trabajo seguía con `enabled = ["searxng"]` y un expediente
  `[providers.searxng]`. Como el propio archivo **declaraba** la ficha,
  `validate()` pasaba (`config/mod.rs:472-479` solo comprueba `is_declared`) y el
  fallo aparecía solo en ejecución: `service.rs:2081-2082` devuelve
  `ProviderNotRegistered` para un nombre habilitado sin factoría, rompiendo
  **toda** búsqueda no-`--mock`.
- **Reparación** (solo en el archivo gitignored `amatl.toml`; ninguna línea de
  producto cambió):
  - `enabled = ["searxng"]` → `enabled = ["marginalia"]` (decisión del operador:
    su entorno ya tenía `MARGINALIA_API_KEY`).
  - Eliminado el bloque `[providers.searxng]` completo.
  - Corregidos 3 comentarios que aún listaban SearXNG como fuente.
- **Evidencia del cierre**:
  - [EJECUCIÓN] `amatl providers` → `marginalia  available` (antes: la fuente
    retirada no aparecía y Marginalia salía `provider_disabled`).
  - [EJECUCIÓN] `amatl search "rust" --mock` → `status: success`.
  - [EJECUCIÓN] `amatl config` → `providers.declared = ["marginalia"]`,
    `providers.enabled = ["marginalia"]`.
  - [EJECUCIÓN] `amatl search "rust"` (sin `--mock`): el error
    `provider_not_registered: searxng` **ya no ocurre**.

## H-03 — [BAJO] El test de retirada no cubría consultas JS

- **Causa raíz**: `retired_source_selector_is_gone_and_cannot_return`
  (`crates/amatl-ui/src/lib.rs`) enumeraba símbolos, no el contrato "no quedan
  consultas a elementos del selector retirado", que es como H-01 pasó la suite.
- **Evidencia del cierre**: el mismo commit `5402104e` añadió las tres
  aserciones que faltaban (`provider-selector`, `providerSelector`,
  `provider-source` ausentes de `app.js`).

---

## Validación en capas (estado reparado)

| Capa | Comando | Resultado |
|---|---|---|
| Enfocadas | `cargo test -p amatl-ui -p amatl-core --lib --locked` | 27 + 389 ok |
| Básicas | `cargo build --workspace --locked` | ok |
| Básicas | `cargo fmt --all -- --check` | ok (exit 0) |
| Amplia | `cargo test --workspace --locked` | **706 tests, 0 fallos** |
| Flujo | `amatl providers` | `marginalia available` |
| Flujo | `amatl search "rust" --mock` | `status: success` |

- **Baseline** previo a editar: `cargo test -p amatl-ui` → 27/27 ok. No apareció
  ningún fallo nuevo; no se atribuye nada al entorno.
- **No verificado**: la llamada real a `api2.marginalia-search.com` no se ejecutó
  (consume cuota de una key escasa emitida a mano). Queda como verificación
  opcional del operador con `amatl search "rust"` sin `--mock`.

---

## Alcance y límites

- La reparación de H-02 vive en `amatl.toml`, un archivo **gitignored**
  (`.gitignore:2`): es local a este checkout. Un clon nuevo parte de
  `amatl.example.toml`, que ya estaba limpio, así que no reproduce el hallazgo.
- **Endurecimiento pendiente (no aplicado)**: `validate()` podría rechazar un
  `enabled` cuyo nombre, aun declarado en el archivo, no tenga factoría en el
  `ProviderRegistry`. Hoy la declaración local "blanquea" el nombre y evita el
  error temprano; sería la defensa de producto que impide repetir H-02 con otra
  fuente retirada. Requiere decisión de diseño (cambia validación), fuera del
  alcance de esta reparación.
- **Caducidad a vigilar**: la ficha de Marginalia en `amatl.toml`
  (`reviewed_at = "2026-09-13"`) expira a los 90 días → ~**2026-12-12**. Pasada
  esa fecha la fuente queda `provider_not_approved` hasta renovar la ficha
  (`config/mod.rs:114-143`, `approved_on`).
- **Riesgo residual de secreto**: la key se lee de `MARGINALIA_API_KEY` en el
  entorno y nunca se escribió en `amatl.toml` ni en logs; no se imprimió su valor
  en ninguna verificación.
