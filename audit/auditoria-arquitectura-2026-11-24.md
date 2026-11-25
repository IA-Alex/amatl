# Auditoría Arquitectónica Full-Stack — AMATL (2026-11-24)

**Skill**: `architecture-audit-fullstack`
**Alcance**: workspace completo (4 crates, ~46 400 LOC Rust de producción + ~3 730 LOC de assets UI + ~6 580 LOC de tests de integración).
**Modo**: READ-ONLY. No se ha modificado ningún archivo de producto.
**Fuente**: `/home/alonso/amatl`, rama `fix/chromium-sandbox-path`, HEAD `56cb6ef0`, árbol limpio (solo `.playwright-mcp/` sin rastrear).
**Verificación en vivo ejecutada**: `cargo fmt --all -- --check`, `python3 tools/check_doc_refs.py`, `ruff check tools/`, inspección de `.github/workflows/*.yml`, `deny.toml`, `scripts/security-gate.sh`, grafo de dependencias entre crates, recuento de tests y de `unwrap/expect` en producción.

> **Relación con la auditoría previa del 2026-11-23** (`audit/auditoria-arquitectura-2026-11-23.md`):
> esa corrida ya cerró ARQ-001 (gate de formato) y ARQ-005 (doc de gates) vía el commit `56cb6ef0`.
> Este informe **re-verifica** el estado actual y añade los hallazgos que aquella corrida no
> cubrió: la **puerta `doc-refs` que hoy está roja** (ARQ-010), la **deriva de citas de línea en
> la documentación de seguridad** (ARQ-011), y precisiones sobre ARQ-001/ARQ-002. No duplica lo
> ya cerrado.

---

## 1. Resumen ejecutivo

AMATL se mantiene en el **cuartil alto de calidad arquitectónica** y el trabajo del 2026-11-23
mejoró una de las dos puertas rojas. Sin embargo, **el estado no es verde hoy** en una **segunda
puerta de CI**, distinta de la que se corrigió:

- **La puerta `doc-refs` (`static-analysis.yml` → `tools/check_doc_refs.py`) sale con exit 1**:
  **16 citas `file:line` muertas** en la documentación de seguridad. El verificador las reporta y
  falla; el step bloqueante del workflow las cuenta como *new* (no están en `FROZEN`).
- La causa raíz es el commit `44b44e64` (*finish retiring the answer/strategist surface*), que
  **encogió `crates/amatl-server/src/lib.rs` de ~3 540 a 3 369 líneas**, mientras
  `docs/security/{threat-model,http-hardening,asvs-checklist}.md` siguen citando rangos como
  `amatl-server/src/lib.rs:3434-3460` y `:3537-3538` — **más allá del fin del archivo**.
- Controles citados **siguen existiendo** (solo se movió el número de línea): es una **deriva de
  cita**, no una pérdida de implementación. Eso hace el arreglo barato (re-anclar líneas), pero el
  problema es real: la documentación de seguridad apunta a líneas inexistentes.
- Las otras puertas verificadas **sí están verdes**: `cargo fmt --check` (exit 0), `ruff check tools/`
  (All checks passed), y el `deny.toml` mantiene exactamente los dos advisories suprimidos/revisados.

| Severidad | Nº |
|---|---|
| Crítico | 0 |
| Alto | 2 |
| Medio | 3 |
| Bajo | 3 |

**Top 3 riesgos**: (1) puerta `doc-refs` roja en cada push que toque `crates/**` o `docs/**`
(ARQ-010); (2) citas de línea obsoletas que erosionan la auditabilidad de la doc de seguridad
(ARQ-011, eslabón de ARQ-010); (3) God Object `impl AmatlService` de 1 541 líneas (ARQ-002, ya
documentado, sigue abierto).

---

## 2. Fortalezas verificadas (lo que NO hay que tocar)

- **Dirección de dependencias correcta y DAG estricto**: `amatl-core` no importa `amatl-server`,
  `amatl-ui` ni `amatl-cli` (grep: solo referencias en comentarios). `server → core + ui`;
  `cli → core + server`. Cumple la invariante del README ("un solo core sirve a CLI, UI, API y MCP").
- **Endurecimiento HTTP de primera**: `required_scope` es *fail-closed* por diseño
  (`amatl-server/src/lib.rs:557`), con la ruta no declarada cayendo en `Scope::Admin`; exención
  pública restringida a `GET|HEAD` para que un futuro `POST /health` no herede la exención.
  Comparación de credenciales en tiempo constante (`constant_time_eq`, `:3206`), digest SHA-256
  (nunca el token en claro), `config_write_lock` (`AsyncMutex`) que serializa validate→write→reload
  para evitar *lost updates* entre PATCHes admin concurrentes.
- **Contrato OpenAPI ↔ router verificado por test-guard** (`openapi_covers_every_router_operation`,
  `openapi_operations_match_router_inventory`, `openapi_error_catalog_matches_shared_catalog`).
  La coherencia docs↔código de *rutas* está ejecutada, no prometida.
- **Documentación excepcional**: 19 ADRs (`decisiones_amatl.md`), 9 docs de seguridad, C4
  (contexto/contenedores) + secuencia (búsqueda/deep/MCP) + flujo de pipeline, portal indexado.
- **Higiene de repositorio correcta**: `amatl.toml`, `.env`, `*.sqlite3*`, `*.log`, `output/`,
  `analiza/` correctamente en `.gitignore` y **no rastreados** (`git ls-files` limpio). Secretos
  solo desde entorno.
- **Gestión de secretos y advisories disciplinada**: `deny.toml` suprime exactamente 2 IDs, cada uno
  con ADR dueño (`ADR-017`, `ADR-018`) + modelo de amenaza + re-derivación en cada corrida del gate.
- **Supresión de `unwrap` en producción**: ~6 en código no-test (~4 en `operational.rs`, 1 en
  `config/mod.rs:358` justificado por invariante local, 1 en `lock_probe.rs`, 1 en `main.rs`). El
  resto de los 654 hits está dentro de módulos `#[cfg(test)]` (586 en `amatl-server/src/tests.rs`).
- **Migrations de SQLite numeradas** (0001–0008) con directorio `downgrade/` y `MIGRATION_VERSION`
  coherente; soporte explícito de downgrade y restore.

---

## 3. Hallazgos por severidad

### [ALTO] ARQ-010 — CI: la puerta `doc-refs` sale roja (16 citas `file:line` muertas)

**Descripción**
`python3 tools/check_doc_refs.py` sale con **exit 1** sobre el árbol limpio. El workflow
`.github/workflows/static-analysis.yml`, job `doc-refs`, ejecuta exactamente ese comando como
step **bloqueante** en cada `push`/`pull_request` que toque `crates/**`, `docs/**`, `README.md`
o `SECURITY.md`. El step está rojo.

**Evidencia**
```
$ python3 tools/check_doc_refs.py ; echo $?
dead reference docs/security/asvs-checklist.md:20: amatl-server/src/lib.rs:3434-3460,699-807 (line past the end of the file)
dead reference docs/security/asvs-checklist.md:22: amatl-server/src/lib.rs:3474-3492 (line past the end of the file)
dead reference docs/security/asvs-checklist.md:27: amatl-server/src/lib.rs:3396-3426,3461-3492 (line past the end of the file)
dead reference docs/security/asvs-checklist.md:28: amatl-server/src/lib.rs:2970-2982,3517-3535 (line past the end of the file)
dead reference docs/security/asvs-checklist.md:32: amatl-server/src/lib.rs:3434-3460 (line past the end of the file)
dead reference docs/security/asvs-checklist.md:33: amatl-server/src/lib.rs:2984-3150,3434-3460 (line past the end of the file)
dead reference docs/security/http-hardening.md:40: amatl-server/src/lib.rs:3396-3416 (line past the end of the file)
dead reference docs/security/http-hardening.md:43: amatl-server/src/lib.rs:3461-3472 (line past the end of the file)
dead reference docs/security/http-hardening.md:46: amatl-server/src/lib.rs:3474-3492 (line past the end of the file)
dead reference docs/security/http-hardening.md:56: amatl-server/src/lib.rs:53,459-521,3385-3394 (line past the end of the file)
dead reference docs/security/http-hardening.md:108: amatl-server/src/lib.rs:3434-3460 (line past the end of the file)
dead reference docs/security/threat-model.md:58: amatl-server/src/lib.rs:53,241-267,459-521,735-739,2551-2560,3323-3350,3385-3394 (line past the end of the file)
dead reference docs/security/threat-model.md:59: amatl-server/src/lib.rs:3275-3295,3396-3426,3474-3492 (line past the end of the file)
dead reference docs/security/threat-model.md:76: amatl-server/src/lib.rs:3495-3535 (line past the end of the file)
dead reference docs/security/hallazgos-2026-10-26.md:131: docs/security/threat-model.md:124 (line past the end of the file)
dead reference docs/security/hallazgos-2026-10-26.md:142: crates/amatl-server/src/lib.rs:3537-3538 (line past the end of the file)
16 dead reference(s). Fix the path or the line, or freeze the pair in FROZEN with a reason ...
$ echo $?
1
```
`crates/amatl-server/src/lib.rs` tiene **3 369 líneas**; todas las citas apuntan a rangos ≥ 3 385.
Ninguna de estas parejas está en el set `FROZEN` de `tools/check_doc_refs.py` (que solo congela 8
pares, todos de rutas externas o históricas ajenas a `amatl-server`).

**Por qué importa**
Es una **puerta roja permanente** para cualquier cambio que toque `crates/**` o `docs/**` — es
decir, prácticamente cualquier PR de producto. Deja sin señal esa puerta y contradice el propio
encabezado del workflow, que la describe como *blocking*. Es el mismo antipatrón que el repo ya
combatió para `cargo audit` y para `cargo fmt` (ARQ-001): un gate que no puede ponerse verde
deja de portar información.

---

### [ALTO] ARQ-011 — Coherencia docs↔código: citas de línea de la doc de seguridad desactualizadas

**Descripción**
Las 16 citas muertas de ARQ-010 no señalan rutas inexistentes, sino **rangos de línea obsoletos**
en un archivo que existe. El commit `44b44e64` (retiro de la superficie answer/strategist) eliminó
~170 líneas de `amatl-server/src/lib.rs`; los documentos de seguridad citan rangos anteriores a
esa poda.

**Evidencia**
Controles citados que **siguen existiendo** (solo cambió el número de línea):
```
docs/security/http-hardening.md:56  cita  amatl-server/src/lib.rs:3385-3394  (constant-time token compare)
  → real: crates/amatl-server/src/lib.rs:3206  (fn constant_time_eq)  y  :3144 (authenticate)
docs/security/http-hardening.md:46  cita  amatl-server/src/lib.rs:3474-3492  (CORS)
  → real: crates/amatl-server/src/lib.rs:3305  (fn cors_layer)
docs/security/http-hardening.md:40  cita  amatl-server/src/lib.rs:3396-3416  (Host validation)
  → real: crates/amatl-server/src/lib.rs:3217  (fn valid_host)
```
`git log` de `crates/amatl-server/src/lib.rs`:
```
44b44e64 refactor!: finish retiring the answer/strategist surface in UI and docs
8f3fd080 refactor!: retire the answer/strategist feature end to end
```
`docs/security/http-hardening.md` no se toca desde `2026-10-26`; el retiro es de `2026-11-22`.

**Por qué importa**
La documentación de seguridad (ASVS checklist, modelo de amenaza, endurecimiento HTTP) es la
evidencia que audita los controles. Un auditor que siga `lib.rs:3385-3394` para verificar la
comparación en tiempo constante llegará a una línea arbitraria (o al EOF) y **no** al control.
Es deuda de *coherencia código-documentación*, no de código: el control está, la cita no.

---

### [ALTO] ARQ-002 — Arquitectura: `impl AmatlService` de 1 541 líneas (God Object) — **sigue abierto**

**Descripción**
`crates/amatl-core/src/service.rs` (3 648 LOC) contiene un único `impl AmatlService` de las líneas
**466 a 2006** = **1 541 líneas**, que agrupa búsqueda, batch, paginación, deep, persistencia,
credenciales, canarios, gobierno de providers, caché, historial, `saved`, backups y estado.

**Evidencia**
```
$ python3 (medición de bloques impl en service.rs)
 1541 lines  466-2006  impl AmatlService {
   42 lines  94-135   impl ExecutionLimits {
   27 lines  39-65    impl ServiceSurface {
```
`amatl-server/src/lib.rs` (3 369 LOC) replica el patrón: ~45 handlers + `AppState` + auth +
rate-limit + métricas + seguridad en un solo módulo.

**Por qué importa**
Violación de SRP a nivel de tipo; encarece revisión, *test-first* y razonamiento; es la superficie
con más probabilidad de regresiones cruzadas (un cambio en gobierno de providers puede afectar el
flujo de búsqueda). El repo ya aplicó la estrategia de sub-módulos a `config/` (10 archivos) pero
no a `service`. **Pendiente confirmado de la auditoría previa; sin cambio en el intervalo.**

---

### [MEDIO] ARQ-003 — Testing: sin métrica de cobertura automatizada pese a meta 70%

**Descripción**
699 tests (492 core / 153 server / 25 cli / 29 ui). La cobertura se declara por capacidad en
`docs/testing.md` (matriz de 25 filas), pero **no existe medición de líneas ni umbral** en CI.

**Evidencia**
```
$ grep -rn "llvm-cov\|tarpaulin\|coverage" .github/ scripts/ Cargo.toml
(sin resultados)
```
Concentración asimétrica: `amatl-core` (34 008 LOC) tiene 492 tests; `amatl-ui` (892 LOC Rust +
3 729 LOC de assets JS/CSS) tiene 29; `amatl-cli` (1 522 LOC) tiene 25.

**Por qué importa**
Sin número reproducible no se puede demostrar dónde está la deuda. Los assets UI (82 KB de `app.js`
+ 34 KB `styles.css`) son una superficie grande con validación de contrato delegada a tests de
navegador (`browser_e2e.rs`) que solo corren con `AMATL_BROWSER_E2E=1`.

---

### [MEDIO] ARQ-004 — Documentación: `config/mod.rs` a 2 527 LOC sin descomposición completa

**Descripción**
`crates/amatl-core/src/config/mod.rs` (2 527 líneas) concentra el tipo raíz `Config`, su
ensamblado y funciones libres de validación, pese a que ya existen 10 sub-módulos en `config/`.

**Evidencia**
```
$ ls crates/amatl-core/src/config/
data_policy.rs deep_config.rs execution.rs inference.rs mod.rs
persistence.rs providers.rs ranking.rs server.rs telemetry.rs
$ wc -l crates/amatl-core/src/config/mod.rs  → 2527
```
Nota positiva: 1 solo `.unwrap/.expect` en producción en este archivo (`:358`), justificado por
invariante local (`already_present implies a match`).

**Por qué importa**
Segundo mayor módulo del workspace; candidato natural a `validate.rs`/`patch.rs` siguiendo el
precedente que el repo ya estableció. Mantenible hoy, pero concentra la lógica más propensa a
regresiones de configuración.

---

### [MEDIO] ARQ-012 — UI: validación de contrato dependiente de assets no cubiertos por test Rust

**Descripción**
La mayor parte de la lógica de presentación vive en **JS vainilla** (`crates/amatl-ui/assets/app.js`,
1 904 líneas; `i18n.js`, 443; `client.js`+`store.js`, 111) embebido vía `include_bytes!`. Los 29
tests de `amatl-ui` cubren el *host* Rust (headers, tabla de assets), no el comportamiento del JS.

**Evidencia**
```
$ wc -l crates/amatl-ui/assets/*
 1904 app.js   443 i18n.js   955 styles.css   316 index.html   60 client.js   51 store.js
$ grep -c "AMATL_BROWSER_E2E" crates/amatl-server/tests/browser_e2e.rs  (suite opt-in)
```
El workflow `browser-e2e` es fail-closed **solo cuando se opta** con `AMATL_BROWSER_E2E=1`; un
checkout sin driver queda verde por omisión.

**Por qué importa**
La superficie UI es grande y su verificación primaria no corre en el gate por defecto. No es un
defecto, pero conviene mantenerlo visible: el test `tools`-style de coherencia existe para docs y
CLI, no para el contrato JS↔API.

---

### [BAJO] ARQ-006 — Repositorio: artefactos de evaluación de 3–5 MB rastreados

**Descripción**
`docs/evaluation/independent-relevance/` rastrea varios JSON de 3–5 MB (`raw-capture.json` 5.2 MB,
`frozen-query-universe.json` 3.7 MB, ×2 `candidate-universe.json` 3.7 MB) y
`crates/amatl-core/assets/tracker-domains.txt` (1.6 MB). `.git` = 41 MB, 8 053 archivos.

**Evidencia**
```
$ git ls-files -z | xargs -0 du -h | sort -rh | head -5
5.2M docs/evaluation/independent-relevance/v6/execution/raw-capture.json
3.7M docs/evaluation/independent-relevance/v6/frozen-query-universe.json
3.7M docs/evaluation/independent-relevance/candidates/candidate-v2/candidate-universe.json
3.7M docs/evaluation/independent-relevance/candidates/candidate-universe.json
1.6M crates/amatl-core/assets/tracker-domains.txt
```
Es evidencia experimental reproducible (parece deliberado para trazabilidad). Recomendación: un
ADR corto que decida rastrear / LFS / regenerar, o documentar la justificación.

**Por qué importa**
Clones y diffs más pesados. Bajo porque la decisión parece intencional.

---

### [BAJO] ARQ-008 — Versionado: algunos commits fuera de Conventional Commits

**Descripción**
`git log --oneline -300` muestra algunos mensajes que no siguen `tipo(scope): …` (p. ej.
`Repository hygiene audit: …`). El repo ya usa `security:`, `release:` y `ci:` por convención
propia, así que solo unos pocos son prosa libre.

**Evidencia**
```
$ git log --oneline -300 | grep -vE '^[a-f0-9]+ (feat|fix|docs|refactor|test|chore|ci|perf|build|style|revert)(\(|:|!)'
```
El versionado semántico sí es disciplinado: `CHANGELOG` Keep-a-Changelog, tag `v0.1.0-rc.1`,
`schema_version` documentado como eje independiente.

**Por qué importa**
Solo forma del mensaje. Bajo.

---

### [BAJO] ARQ-009 — Deuda: TODOs de versión 1.0 sin tracking formal

**Descripción**
3 TODOs acotados al retiro del alias `local_model_batch`, marcados `TODO(1.0)`, sin ADR que los
rastree.

**Evidencia**
```
config/inference.rs:34   // TODO(1.0): drop the `local_model_batch` alias.
config/inference.rs:94   // TODO(1.0): drop the `local_model_batch` alias.
config/mod.rs:1358       // TODO(1.0): drop with the alias.
```
La corrida previa ya añadió una sección "Removals planned for 1.0" al `CHANGELOG`; verificar que
cubre estos tres.

**Por qué importa**
Legibles y acotados (bien), pero podrían sobrevivir a 1.0 sin un ADR. Muy bajo.

---

## 4. Roadmap ejecutable (orden por dependencia → impacto → esfuerzo)

### Tarea 1 — ARQ-010/011 · Re-anclar las citas de la doc de seguridad (ALTO · Esfuerzo S)
**Dependencias**: ninguna (primero: desbloquea `doc-refs`).

**Problema**: 16 citas `file:line` en `docs/security/*` apuntan a líneas ≥ 3 385 de un archivo de
3 369 líneas; la puerta `doc-refs` falla.

**Solución**:
1. Para cada símbolo citado, localizar la línea actual y actualizar la cita:
   - `constant_time_eq` → `crates/amatl-server/src/lib.rs:3206`
   - `valid_host` → `crates/amatl-server/src/lib.rs:3217`
   - `valid_origin` → `crates/amatl-server/src/lib.rs:3238`
   - `cors_layer` → `crates/amatl-server/src/lib.rs:3305`
   - `required_scope` → `crates/amatl-server/src/lib.rs:557`
   - `token_digest` → `crates/amatl-server/src/lib.rs:530`
   - `security_middleware` → `crates/amatl-server/src/lib.rs:2805`
2. Repetir para `docs/security/{threat-model.md,http-hardening.md,asvs-checklist.md,hallazgos-2026-10-26.md}`.
3. Verificar: `python3 tools/check_doc_refs.py ; echo $?` → `0`.

**Archivo(s)**: `docs/security/threat-model.md`, `docs/security/http-hardening.md`,
`docs/security/asvs-checklist.md`, `docs/security/hallazgos-2026-10-26.md`.

**Alternativa aceptable**: si el repo prefiere conservar el registro histórico de
`hallazgos-2026-10-26.md` (que es un reporte fechado), congelar **solo esa pareja** en `FROZEN`
con una razón — pero **no** las citas de `threat-model.md`/`http-hardening.md`/`asvs-checklist.md`,
que son documentos vivos y deben apuntar al código actual.

---

### Tarea 2 — ARQ-010 (prevención) · Anclar el símbolo, no solo la línea (ALTO · Esfuerzo S)
**Dependencias**: Tarea 1.

**Problema**: la deriva reaparecerá en la próxima poda de `lib.rs`. El verificador detecta
"línea más allá del EOF" pero **no** detecta que una cita apunta a las líneas correctas de un
archivo que se movió (drift semántico), como el propio encabezado de `static-analysis.yml` admite.

**Solución**:
1. En `docs/security/*`, citar el **símbolo** además de la línea, p. ej.:
   `crates/amatl-server/src/lib.rs:3217 (fn valid_host)`.
2. (Opcional, mayor valor) extender `tools/check_doc_refs.py` para que, cuando exista un
   `(identificador)` tras la cita, compruebe que el token aparece en la línea citada; si no,
   reportar "cita desalineada" — barato y sin dependencias.

**Archivo(s)**: `docs/security/*`, opcionalmente `tools/check_doc_refs.py` + `tools/test_*`.

---

### Tarea 3 — ARQ-002 · Descomponer `impl AmatlService` por concern (ALTO · Esfuerzo L)
**Dependencias**: Tarea 1 (gate verde antes de tocar código).

**Problema**: `impl` de 1 541 líneas (`service.rs:466-2006`).

**Solución** (Strangler incremental, sin cambiar la API pública):
1. Extraer `service/search.rs`, `service/deep.rs`, `service/providers.rs`, `service/storage.rs`,
   `service/credentials.rs` con `impl AmatlService` parciales (Rust admite múltiples `impl` en
   archivos distintos del mismo crate).
2. Conservar `pub struct AmatlService` y firmas para no romper los re-exports de `lib.rs`.
3. Repetir en `amatl-server/src/lib.rs`: separar `handlers/`, `auth.rs`, `ratelimit.rs`, `metrics.rs`.
4. Verificar tras cada extracción: `cargo test --workspace --locked`.

**Archivo(s)**: `crates/amatl-core/src/service.rs`, `crates/amatl-server/src/lib.rs`.

**Criterio de éxito**: ningún archivo > 1 500 LOC; `impl` más grande < 500 LOC.

---

### Tarea 4 — ARQ-004 · Extraer validación de `config/mod.rs` (MEDIO · Esfuerzo S)
**Dependencias**: Tarea 3 (mismo patrón de extracción).

**Problema**: `config/mod.rs` 2 527 LOC.

**Solución**: mover funciones libres de validación a `config/validate.rs` y helpers de patch a
`config/patch.rs`; mantener `Config` y re-exports en `mod.rs`.

**Archivo(s)**: `crates/amatl-core/src/config/mod.rs` (+ `validate.rs`, `patch.rs`).

---

### Tarea 5 — ARQ-003 · Medición de cobertura en CI (MEDIO · Esfuerzo M)
**Dependencias**: Tarea 1.

**Problema**: sin métrica de cobertura pese a meta 70%.

**Solución**:
1. Añadir `cargo-llvm-cov` como job nuevo (`coverage`), por-PR o nightly.
2. Publicar resumen y umbral inicial **informativo** (sin fallar) para no romper el gate con un
   umbral sin calibrar; congelar el umbral bloqueante cuando la línea base lo permita.
3. Documentar la cifra medida en `docs/testing.md` (sección "Commands").

**Archivo(s)**: `.github/workflows/ci.yml`, `docs/testing.md`.

---

### Tarea 6 — ARQ-012 · Cobertura del contrato JS↔API (MEDIO · Esfuerzo M)
**Dependencias**: ninguna.

**Problema**: `app.js` (1 904 líneas) sin verificación primaria en el gate por defecto.

**Solución**: mantener el `browser-e2e` fail-closed y añadir un smoke test mínimo no-navegador que
valide el contrato de payloads (`validatePayload`) contra las formas que la API puede devolver, o
documentar explícitamente en `docs/testing.md` que el contrato JS se cubre solo end-to-end.

**Archivo(s)**: `crates/amatl-ui/assets/app.js`, `crates/amatl-server/tests/browser_e2e.rs`, `docs/testing.md`.

---

### Tarea 7 — ARQ-006 · Estrategia de artefactos pesados (BAJO · Esfuerzo S)
**Dependencias**: ninguna.

**Problema**: JSON de 3–5 MB rastreados; `.git` = 41 MB.

**Solución**: ADR corto que decida rastrear / Git LFS / regenerar. Si se rastrea, documentar la
justificación en `docs/evaluation/` (patrón análogo al de `deny.toml`).

---

### Tarea 8 — ARQ-008·009 · Pulido (BAJO · Esfuerzo XS, c/u)
- (Opcional) lint de Conventional Commits vía `commitlint`/hook.
- Confirmar que los 3 `TODO(1.0)` están en la sección "Removals planned for 1.0" del `CHANGELOG`.

---

## 5. Verificación en vivo (evidencia recogida hoy)

| Comando | Resultado |
|---|---|
| `cargo fmt --all -- --check` | **exit 0** (ARQ-001 cerrado; `rust-toolchain.toml` presente) |
| `python3 tools/check_doc_refs.py` | **exit 1** — 16 citas muertas (ARQ-010) |
| `ruff check tools/` | All checks passed |
| `deny.toml` `ignore` | `["RUSTSEC-2026-0459", "RUSTSEC-2026-0543"]` (2, revisados, con ADR) |
| Dirección de dependencias | core no importa server/ui/cli (grep: solo comentarios) |
| `.gitignore` vs `git ls-files` | `amatl.toml`, `.env`, `*.sqlite3*`, `*.log` no rastreados ✓ |
| Tests | 699 (492/153/25/29) |

**Nota metodológica**: no se han ejecutado los workflows de GitHub Actions remotos, ni la suite
`browser-e2e`/`soak`/`trafilatura-integration` (opt-in local), ni `cargo test --workspace`
completo en esta corrida; los hallazgos se apoyan en comandos reproducibles y en inspección de
CI/config. Las cifras de LOC y conteos son exactas al HEAD `56cb6ef0`.

## 6. Conclusión

El proyecto sigue **arquitectónicamente sano y por encima de la media**: límites de capa correctos,
contrato OpenAPI verificado por test, documentación profunda (19 ADRs, 9 docs de seguridad, C4 +
secuencia), higiene de secretos y supply-chain disciplinada, y ~6 `unwrap` en producción. La
corrida previa del 2026-11-23 dejó verde la puerta de formato, pero **queda una segunda puerta roja
que aquella corrida no ejecutó**: `doc-refs`, con 16 citas de línea obsoletas **exclusivamente en la
documentación de seguridad**. El arreglo es barato (re-anclar líneas, Esfuerzo S) y de alto valor
porque restaura la auditabilidad de la evidencia de seguridad y devuelve señal a la puerta.

Ninguno de los hallazgos es **crítico**: no hay riesgo de seguridad, corrupción de datos ni
arquitectura quebrada. Los dos Altos son operativos (una puerta de CI roja) y de mantenibilidad
(una deriva de citas); el God Object ya documentado sigue siendo la mayor deuda estructural y tiene
remedio incremental sin cambiar la API pública.

**Camino más corto a verde**: Tarea 1 (S) → verificar `doc-refs` exit 0 → resto por impacto.

---

## 7. Estado de reparación (ejecutado 2026-11-24 con `software-repair-engineer`)

Modo REPAIR + TEST. Cambios **mínimos**: solo documentación; **ningún archivo de producto ni de código Rust** tocado.

| Hallazgo | Disposición | Cambio |
|---|---|---|
| ARQ-010 | **REPARADO** | Re-ancladas las 16 citas `file:line` muertas de `docs/security/*` a las líneas actuales del código; `doc-refs` pasa de **exit 1 → exit 0** |
| ARQ-011 | **REPARADO** | Deriva de citas corregida; además se re-anclaron citas que resolvían como rango pero apuntaban a código distinto (`secured`, `metrics`, `status`, `health`, `api_error`, rechazo de query) |
| ARQ-002 | **ESCALADO** | God Object `impl AmatlService` (1541 líneas); requiere extracción arquitectónica (L), fuera de una reparación mínima |
| ARQ-003 | **ESCALADO** | Requiere decisión de CI (`cargo-llvm-cov` + umbral) |
| ARQ-004 | **ESCALADO** | Depende de ARQ-002 (mismo patrón de extracción) |
| ARQ-012 | **ESCALADO** | Requiere estrategia de cobertura del contrato JS (M) |
| ARQ-006 | **ESCALADO** | Requiere ADR de estrategia de artefactos pesados |
| ARQ-008 | **ESCALADO** | Cosmético/opcional (commitlint) |
| ARQ-009 | **VERIFICADO** | Los 3 `TODO(1.0)` ya figuran en "Removals planned for 1.0" del `CHANGELOG`; sin cambio |

La **prevención de ARQ-010 (Tarea 2)** se aplicó como parte de la reparación: cada cita re-anclada
lleva ahora el **símbolo** entre paréntesis (p. ej. `lib.rs:3217-3236 (valid_host)`), de modo que la
próxima poda de `lib.rs` deja una cita verificable contra el nombre, no solo contra el número.

**Archivos cambiados** (5, solo documentación):
`docs/security/asvs-checklist.md`, `docs/security/http-hardening.md`,
`docs/security/threat-model.md`, `docs/security/secrets.md`, `docs/security/hallazgos-2026-10-26.md`.

**Verificación (LEVEL 1 + 5):**
```
python3 tools/check_doc_refs.py          → exit 0 ("14 known-dead, 0 new"; los 14 son externos/históricos congelados en FROZEN)
cargo fmt --all -- --check               → exit 0
git diff --name-only                     → solo docs/security/*.md (ningún .rs)
```
Verificación de contenido: cada línea citada se comprobó contra el símbolo que la cita declara
(`valid_host`, `cors_layer`, `constant_time_eq`, `authenticate`, `required_scope`, `secured`,
`service_error`, `update_server_pending_config`, etc.) — todas coinciden.

**Riesgo residual**: el verificador `check_doc_refs.py` aún no detecta *drift semántico* (una cita
que resuelve como número pero apunta a código distinto); por eso la prevención robusta (extender el
script para validar el token entre paréntesis) queda como Tarea 2 opcional.

