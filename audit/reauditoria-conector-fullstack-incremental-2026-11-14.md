# Re-auditoría conector-fullstack incremental — AMATL (UI ↔ servidor)

Continuación de `audit/auditoria-conector-fullstack-2026-11-14.md` (cierre
`VERIFICADO-E2E` en `6b032a41`). Esta corrida audita **solo el delta**
`6b032a41..6170d7ef` sobre HEAD, con evidencia estática, de contrato y de
ejecución real en navegador. Es una auditoría de **correspondencia y trazabilidad
entre capas**, no de estilo ni de seguridad.

- Fuente: `/home/panda/amatl`. HEAD al analizar: `6170d7ef`
  (`fix(ui): hide the model toggle until the strategist is usable`).
- Delta auditado: 3 commits, 3 de producto y 1 documental.
- Modo: **re-auditoría incremental** (fases 0–5 acotadas al delta), con
  ejecución real.
- **No se modificó** ningún fichero de producto. Este informe se escribe en
  `audit/` por convención del repo.

---

## 1. Alcance y contexto

- **Objetivo**: verificar que los tres cambios de UI del delta mantienen la
  correspondencia con el proveedor y producen de verdad el resultado declarado.
- **Delta** (`git diff --stat 6b032a41..6170d7ef`):

  | Commit | Ficheros de producto | Naturaleza |
  |---|---|---|
  | `45f17b82` fix(ui): close the UX findings on the search surface | `app.js`, `i18n.js`, `index.html`, `styles.css`, `lib.rs` (tests) | Conector |
  | `64812bfb` docs(audit): record the connector-fullstack and UX reviews | `audit/*.md` | Documental |
  | `6170d7ef` fix(ui): hide the model toggle until the strategist is usable | `app.js`, `index.html`, `lib.rs` (tests) | Conector |

- **Proveedor (`amatl-server`) intacto en el delta**: `git diff --stat
  6b032a41..6170d7ef -- crates/amatl-server/` está **vacío**. La tabla canónica
  `define_public_routes!` en `routes.rs` es byte-idéntica a la ya auditada, por lo
  que la matriz de rutas de §3 del informe previo sigue vigente sin cambios.
- **Qué cambió en el conector** (los tres puntos verificados):
  1. **Gating por usabilidad** de `#model-options` (contenedor del toggle
     "Búsqueda con modelo") según el estado del strategist reportado por `/status`.
  2. **Regrupación del token** `#local-token`: sale del grid de filtros a un
     disclosure propio `details.access-filters` (`#access-summary`).
  3. **Pluralización del conteo de historial**: `historyLabel` pasa por
     `countLabel(..., "resultsCount")` en vez de `t("resultsCount")`.
- **Entorno de ejecución** (local y efímero, nunca red compartida):
  - Binario `target/release/amatl` **reconstruido** tras detectar staleness
    (ver §4, H-R01), en `127.0.0.1:8942` (`--mock`, `no_auth = true`) y
    `127.0.0.1:8943` (variante sin strategist configurado).
  - Navegador real: Chromium embebido de Playwright (build 1248), ejercitando el
    cliente in-page (`globalThis.api`) sobre la página servida.
- **Nota de fidelidad**: `--mock` no alcanza la red; se valida correspondencia de
  contratos, cableado y comportamiento de UI, no la calidad del ranking sobre
  fuentes reales.

---

## 2. Dictamen

- **Estado: VERIFICADO-E2E (delta).**
- **Resumen**: los tres cambios del delta coinciden entre conector y proveedor y
  producen en navegador real el resultado declarado. El gating de `#model-options`
  se confirmó en **ambos** sentidos (`configured+available` → visible;
  `!configured` → oculto con `display:none` y alto 0). La regrupación del token es
  un cambio de presentación puro, sin alteración de contrato (mismos ids, mismos
  endpoint, misma semántica de error). La pluralización produce `"1 resultado."`
  singular en el conteo 1, cerrando la divergencia con el encabezado. No se halló
  ninguna ruta huérfana nueva ni desajuste de contrato **introducido por el delta**.
- **Severidad del delta**: crítico 0 · alto 0 · medio 0 · bajo 0 (H-R01 es práctica
  de proceso; H-R02 quedó cerrada).
- **Cobertura**: flujos del delta identificados **3** · trazados **3** · contratos
  **3** · E2E **8** (suite oficial `browser_e2e` + dos configuraciones de servidor) ·
  fallos introducidos **0** · pendientes **0**.
- **Remediación**: se cerraron los dos huecos de verificación (§6): driver 157
  instalado (suite oficial 8/8 con evidencia real) y test E2E del gating añadido
  (validado por mutación). Único cambio de producto: `browser_e2e.rs` (+75 líneas,
  un test). No se tocó código de la UI ni del servidor.
- **Límite**: el dictamen cubre el delta sobre `6170d7ef`. Las observaciones de
  baja severidad del informe previo (H-01..H-03) **siguen abiertas** y no se
  re-evaluaron aquí salvo en lo que el delta toca.

---

## 3. Matriz de trazabilidad (delta)

Origen = `crates/amatl-ui/assets/*` y `crates/amatl-ui/src/lib.rs`. Proveedor =
`crates/amatl-core/src/service.rs` (`StrategistStatus`) y `/status` en
`crates/amatl-server/src/routes.rs` (intacto). Evidencia E2E = navegador real
sobre el binario reconstruido.

| ID | Origen | Operación | Proveedor | Resultado | Evidencia | Estado |
|---|---|---|---|---|---|---|
| D-01 | `index.html:70` (`#model-options hidden`), `app.js:1287,1296,1359` | Gating por estado del strategist leído de `GET /status` | `routes.rs:71` get(`status`) → `service.rs:1793` `StrategistStatus{enabled,configured,available,model,endpoint}` | Caso usable: `hidden:false`, `display:flex`. Caso no configurado: `hidden:true`, `display:none`, `offsetHeight:0` | ESTÁTICO + E2E (Chromium sobre :8942 y :8943) | OK |
| D-02 | `index.html:144-152` (`details.access-filters`, `#access-summary`, `#local-token`), `app.js:284-301,633,881-884` | Token fuera del grid de filtros; panel propio; error abre el panel correcto | Sin endpoint nuevo (mismo formulario y mismo `#local-token`) | Panel hermano del de filtros; contiene el token; fuera del `filter-grid`; `#filters-summary` = "Filtros" | ESTÁTICO + E2E (DOM real) | OK |
| D-03 | `app.js:1532` `countLabel(entry.total_results,"resultsCount")` | Conteo de historial pluralizado | Contrato `countLabel`/`pluralKey` existente (sin cambio de servidor) | `countLabel(1,"resultsCount")` = `"1 resultado."`; el viejo `t("resultsCount")` = `"resultados."` | ESTÁTICO + E2E (ejecución de ambos helpers en la página) | OK |
| D-04 | `lib.rs:649,687,730` (nuevos tests) | Guardas de regresión de D-01..D-03 | — | `cargo test -p amatl-ui` → 25/25 ok | ESTÁTICO + EJECUCIÓN | OK |

Sin filas `DESAJUSTE`, `HUÉRFANA (cliente)`, `SIN CONTRATO` ni `NO VERIFICADO`.

---

## 4. Hallazgos

No hay hallazgos crítico, alto ni medio introducidos por el delta. Se registra
una observación de severidad **baja**, de entorno (no de producto).

### H-R01 — [BAJO] Binario release servía assets embebidos obsoletos (staleness de build, no defecto de producto)

- **Flujos**: todos los que consumen assets de UI (D-01..D-03), durante la
  primera pasada de esta corrida.
- **Evidencia**: [EJECUCIÓN] el binario `target/release/amatl` (mtime `17:21:10`)
  era **anterior** a `index.html`/`app.js` (mtime `17:49`). El HTML servido en
  `127.0.0.1:8942` traía `<div class="model-options">` **sin `id` y sin `hidden`**,
  y el DOM real no encontraba `#model-options` (`getElementById` → `null`). Los
  assets se embeben con `include_bytes!`, así que el binario no recompilado sirve
  la versión previa al commit.
- **Síntoma**: la nueva superficie (gating del toggle, panel de token) **no existía**
  en la página servida, lo que podría leerse como un fallo del cambio `6170d7ef`.
- **Causa**: CONFIRMADO — **staleness del artefacto de build**, no un defecto del
  código. Tras `cargo build --release -p amatl-cli` (mtime `17:56`), el HTML servido
  pasó a traer `id="model-options" hidden`, `details.access-filters` y
  `panel-scroll`, y los tres puntos del delta quedaron verificables.
- **Impacto**: nulo sobre el producto. Riesgo de proceso: auditar un binario
  desactualizado produce **falsos negativos** en la capa de UI. Aplica a cualquier
  verificación E2E de assets embebidos.
- **Corrección mínima propuesta**: ninguna de código. De proceso: reconstruir el
  binario como primer paso de cualquier verificación E2E de la UI (o comprobar
  `mtime(binario) >= mtime(assets)` antes de arrancar). No urgente.
- **Regresión recomendada**: ninguna nueva; los tests de `amatl-ui` ya comparan
  contra los assets fuente directamente, de modo que no se ven afectados por
  el staleness del binario.

---

## 5. Operaciones sin correspondencia

Sin hallazgos nuevos. El delta **no añade ni retira endpoints** del conector: las
tres operaciones del §3 son de comportamiento/presentación sobre superficies ya
trazadas (D-02 no introduce llamada; D-01 y D-03 consumen `/status` y el contrato
`countLabel`, ambos ya auditados). La lista de huérfanas del proveedor respecto a
esta UI de §5 del informe previo permanece **idéntica**, dado que `routes.rs` no
cambió en el delta. No se encontró ninguna huérfana del lado cliente.

---

## 6. No verificado y límites

- **E2E vía `browser_e2e.rs` (fantoccini)**: **RESUELTO**. En la primera pasada la
  suite fallaba cerrada por incompatibilidad de versión (ChromeDriver 155 frente a
  Chrome 157). Se instaló ChromeDriver 157.0.8103.78 (coincide exacto con Chrome
  157.0.8103.78) y la suite corre ahora con evidencia válida: **8/8 ok en 2.40s**
  con `AMATL_BROWSER_E2E=1`. Sin la variable sigue siendo inerte por diseño.
- **`/answer` con backend disponible**: sigue NO VERIFICADO en ejecución (fuera
  del delta).
- **Calidad sobre fuentes reales**: excluida por diseño (`--mock`).
- **Cobertura del gating en E2E**: **RESUELTA** — se añadió
  `model_toggle_is_hidden_when_the_strategist_is_not_configured` (ver H-R02 en §7).
  No queda brecha de cobertura de comportamiento para este control.

---

## 7. Plan de corrección

Ningún hallazgo del delta es crítico, alto ni medio: no hay corrección obligatoria.
Las tres acciones de higiene quedaron **aplicadas** salvo la de proceso de H-R01
(que es una práctica, no un cambio de código).

| Orden | Hallazgo | Dependencia | Cambio mínimo | Verificación de cierre | Estado |
|---|---|---|---|---|---|
| 1 | H-R01 | ninguna | Proceso: reconstruir antes de E2E (o comprobar mtime de assets vs binario) | El HTML servido contiene `id="model-options"` | Práctica registrada (memoria `audit/embedded-asset-staleness`) |
| 2 | §6 (browser_e2e) | chromedriver acorde a Chrome | Instalar driver 157 y correr la suite | `AMATL_BROWSER_E2E=1 ... browser_e2e` → 8/8 ok | **CERRADO** |
| 3 | H-R02 | ninguna | Test E2E con strategist no configurado que afirme `#model-options` oculto y toggle deshabilitado | Falla si el gating se retira (probado por mutación) | **CERRADO** |

### H-R02 — [RESUELTO] El gating del toggle no tenía test E2E propio

- **Evidencia (estado inicial)**: [ESTÁTICO] `browser_e2e.rs` (7 tests) no mencionaba
  `model-options`/`modelOptions`; la cobertura del gating vivía en los tests
  estáticos de `amatl-ui` y en la verificación manual de esta corrida.
- **Impacto**: una regresión del *comportamiento* de gating (no del markup) no la
  atraparía un test de navegador.
- **Cierre aplicado**: se añadió el test
  `browser_e2e.rs::model_toggle_is_hidden_when_the_strategist_is_not_configured`
  (+75 líneas, único cambio de producto de esta remediación). Usa el
  `spawn_server` por defecto, cuyo `Config::default()` deja el strategist sin
  endpoint/model (`configured:false`), y afirma en navegador real: `hidden:true`,
  `display:none` (computed), `offsetHeight:0`, toggle `disabled` y `checked:false`.
- **Verificación de que no es un test vacío** (prueba de mutación): neutralizando
  el gating en `app.js` (2 sitios) y el `hidden` del markup, el test **falla**
  (`element never appeared: Css("#model-options[hidden]")`); restaurados los
  assets, vuelve a pasar. Detecta la regresión que dice cubrir.
- **Estado**: RESUELTO. Evidencia: 8/8 en `browser_e2e` con driver real (§6).

### Cierre de la brecha de infraestructura (§6, browser_e2e)

- **Resuelto**: se instaló **ChromeDriver 157.0.8103.78** en `~/.local/bin/chromedriver`
  (coincide exactamente con `google-chrome 157.0.8103.78`; el 155 previo quedó en
  `chromedriver-155.bak`). Con `AMATL_BROWSER_E2E=1`, la suite oficial corre con
  evidencia real: **8/8 ok en 2.40s** (antes fallaba cerrada por desajuste de
  versión, o quedaba inerte en 0.00s sin la variable).

---

## Apéndice — Reproducir la evidencia

```bash
# 1. Reconstruir el binario (evita el staleness de H-R01)
cargo build --release -p amatl-cli

# 2. Config efímera fuera del repo (mock, no_auth, puerto 8942 y 8943)
#    /tmp/user/1000/opencode/amatl-reaudit/amatl.audit.toml          (strategist configurado)
#    /tmp/user/1000/opencode/amatl-reaudit/amatl.nostrategist.toml   (enabled=false, sin endpoint/model)

# 3. Arrancar ambos proveedores locales
AMATL_SERVER_TOKEN=0123456789abcdef0123456789abcdef \
  ./target/release/amatl --config-file /tmp/.../amatl.audit.toml serve --mock
AMATL_SERVER_TOKEN=0123456789abcdef0123456789abcdef \
  ./target/release/amatl --config-file /tmp/.../amatl.nostrategist.toml serve --mock

# 4. Contrato de estado
curl -s http://127.0.0.1:8942/status | jq .strategist   # configured/available = true
curl -s http://127.0.0.1:8943/status | jq .strategist   # configured/available = false

# 5. Tests de regresión del conector
cargo test -p amatl-ui                                  # 25/25 ok

# 6. E2E en navegador real (Chromium de Playwright): cargar / y evaluar
#    #model-options { hidden, display, offsetHeight }, el panel access-filters,
#    y countLabel(1,"resultsCount") vs t("resultsCount").
```

Artefactos E2E capturados en Playwright (snapshots/screenshots) son efímeros y no
se persisten en el repo.
