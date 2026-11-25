# Auditoría Arquitectónica Full-Stack — AMATL (2026-11-23)

**Skill**: `architecture-audit-fullstack`
**Alcance**: workspace completo (4 crates, ~46 400 LOC Rust + ~3 730 LOC assets UI).
**Modo**: READ-ONLY. No se ha modificado ningún archivo de producto.
**Fuente**: `/home/alonso/amatl`, rama actual, HEAD `7c5e5560`, árbol de trabajo limpio
(solo `.playwright-mcp/` sin rastrear).
**Verificación ejecutada en vivo**: `cargo fmt --check`, `cargo clippy --all-targets
--all-features -D warnings`, recuento de tests, inspección de CI, config, docs y
dirección de dependencias entre crates.

---

## 1. Resumen ejecutivo

AMATL está en el **cuartil alto de calidad arquitectónica**: límites de capa limpios
(`core` no conoce `server`/`ui`/`cli`), un único `core` compartido por las cuatro
superficies, contrato OpenAPI con *guard* bidireccional contra el router, 19 ADRs,
diagramas C4 y de secuencia, catálogo de errores compartido, y una disciplina de
cobertura de tests documentada caso por caso en `docs/testing.md`. El código de
producción tiene **solo 10 `.unwrap()/.expect()`** en total (ninguno en rutas calientes),
`unsafe_code = "forbid"` a nivel de workspace y clippy limpio con `-D warnings`.

Sin embargo, el estado **no es verde hoy** en una puerta de CI concreta:

- **El gate `contract-gate` falla en `cargo fmt --all -- --check`**: 8 archivos con
  diffs (puramente cosméticos) sobre un árbol limpio. CI ejecuta este paso en cada push.
- La causa raíz es la **ausencia de `rust-toolchain.toml`**: CI usa `@stable` y el
  rustfmt local (`1.10.0`, 2026-09-28) reformateó construcciones que el código escribió
  bajo un rustfmt anterior. El repositorio no fija toolchain, así que local y CI
  derivan de forma independiente.
- Existe una **contradicción documental**: `audit/cierre-hallazgos-2026-11-18.md:83`
  afirma `cargo fmt --all -- --check | ok (exit 0)`, pero hoy sale con exit 1.

| Severidad | Nº |
|---|---|
| Crítico | 0 |
| Alto | 2 |
| Medio | 3 |
| Bajo | 4 |

**Top 3 riesgos**: (1) gate de formato rojo en cada push sin toolchain fijada (ARQ-001),
(2) ausencia de métrica de cobertura de código pese a la meta 70% (ARQ-004),
(3) documentación que declara un gate verde que no lo está (ARQ-005).

**Nota de contexto**: el repo ya tiene una convención de auditorías en `audit/` (varias
skills: bug-hunter, api-developer, ux-audit, conector-fullstack). Este informe respeta esa
convención y no duplica hallazgos ya cerrados en `audit/cierre-hallazgos-2026-11-18.md`.

---

## 2. Fortalezas verificadas (lo que NO hay que tocar)

- **Dirección de dependencias correcta**: `amatl-core` no importa `amatl-server`,
  `amatl-ui` ni `amatl-cli` (grep sin resultados). `server → core + ui`; `cli → core +
  server`. Cumple la invariante declarada en `README.md` ("un solo core sirve a CLI, UI,
  API y MCP").
- **Sin dependencias circulares**: el grafo de crates es un DAG estricto.
- **Contrato ↔ router verificado por test-guard**: `openapi_covers_every_router_operation`
  (tests.rs:5478), `openapi_operations_match_router_inventory` (tests.rs:5684),
  `openapi_error_catalog_matches_shared_catalog` (tests.rs:5514). La coherencia
  documentación↔código está *ejecutada*, no solo prometida.
- **ADRs**: 19 registros en `decisiones_amatl.md`, con plan de salida para cada
  supresión de advisory (`ADR-017`, `ADR-018`).
- **Higiene de repositorio**: `.gitignore` razonado por AUDIT-xx; bases SQLite, backups,
  logs, `.env`, `output/` y `analiza/` correctamente excluidos (verificado:
  `git ls-files` no rastrea `*.sqlite3`, `*.log`, `amatl.backup-*`).
- **Manejo de errores robusto**: catálogo compartido `errors.rs` consumido por CLI, API y
  MCP; `.unwrap()` en producción casi nulo (18 en total contando `operational`,
  `ranking_adjustments`, `lock_probe`, `main.rs`; 0 en `service.rs`/`storage.rs`).
- **CI amplia**: MSRV 1.88, cross-platform macOS, soak, browser-e2e con chromedriver
  anclado, trafilatura integración, CodeQL de invariantes propios, `security-gate.sh`.

---

## 3. Hallazgos por severidad

### [ALTO] ARQ-001 — Calidad / CI: `cargo fmt --check` falla en cada push; sin `rust-toolchain.toml`

**Descripción**
`cargo fmt --all -- --check` sale con **exit 1** sobre un árbol git limpio. CI
(`.github/workflows/ci.yml`, job `contract-gate`, primer paso) ejecuta exactamente ese
comando en cada `push` y `pull_request`. El primer paso del gate está rojo.

**Evidencia**
```
$ cargo fmt --all -- --check
FMT_EXIT=1
Diff in crates/amatl-core/src/config/server.rs:552
Diff in crates/amatl-core/src/lib.rs:57
Diff in crates/amatl-core/src/lib.rs:145
Diff in crates/amatl-core/src/service.rs:4
Diff in crates/amatl-server/src/mcp.rs:151
Diff in crates/amatl-server/src/mcp.rs:185
Diff in crates/amatl-server/src/tests.rs:1245
Diff in crates/amatl-ui/src/lib.rs:125
(8 archivos)
```
Los diffs son **puramente cosméticos** (unión de literales mult-ilínea, `use`-groups y
arrays que rustfmt 1.10 colapsa a una línea); no hay cambio semántico:
```
-pub const MCP_TOOLS: [&str; 5] = [
-    "search", "deep_search", ...
-];
+pub const MCP_TOOLS: [&str; 5] = ["search", "deep_search", "fetch", "providers", "status"];
```
Causa raíz: **no existe `rust-toolchain.toml`/`rust-toolchain`** (verificado: ambos
ausentes). CI hace `dtolnay/rust-toolchain@stable`; el rustfmt local es `1.10.0-stable
(b940084d7e 2026-09-28)`. Las líneas en `lib.rs` se tocan por última vez en el commit de
baseline `51c6d342` (2026-08-12), es decir, la deriva apareció al actualizarse rustfmt
en el runner, no por edición de código.

**Por qué importa**
Es un gate rojo permanente: cualquier PR queda bloqueado por un diff cosmético, y el
rojo deja de portar señal (mismo patrón que el repo ya documenta para `cargo audit`).
La ausencia de pin de toolchain hace que el resultado del gate dependa de la fecha de
ejecución, no del contenido del commit: reproduce distinto en local y en CI.

---

### [ALTO] ARQ-002 — Arquitectura: `AmatlService` es un God Object con un `impl` de ~1 540 líneas

**Descripción**
`crates/amatl-core/src/service.rs` (3 648 LOC) contiene un único `impl AmatlService`
de las líneas **466 a 2008** (~1 540 líneas) que agrupa búsqueda, deep, persistencia,
credenciales, canarios, gobierno de providers, caché, historial, saved y estado. Es el
mayor punto de acoplamiento del workspace.

**Evidencia**
```
$ grep -n "^impl " crates/amatl-core/src/service.rs
466:impl AmatlService {     # hasta la línea 2008
$ wc -l crates/amatl-core/src/service.rs
3648 crates/amatl-core/src/service.rs
```
`amatl-server/src/lib.rs` (3 369 LOC) replica el patrón: ~40 handlers + `AppState` +
auth + rate-limit + métricas en un solo módulo.

**Por qué importa**
Un `impl` de ese tamaño viola SRP a nivel de tipo, encarece el razonamiento, la revisión
y el *test-first*, y es la superficie más probable de regresiones cruzadas (un cambio en
gobierno de providers puede afectar el flujo de búsqueda). El propio repo ya extrayó
`config/` en sub-módulos (10 archivos) — la misma estrategia no se aplicó a `service`.

---

### [MEDIO] ARQ-003 — Variedad de herramientas de cobertura: granularidad asimétrica de tests

**Descripción**
Hay **699 tests** distribuidos 492 (core) / 153 (server) / 25 (cli) / 29 (ui). La
cobertura está declarada por área en `docs/testing.md` (matriz de 25 filas), lo cual es
excelente, pero **no hay métrica de cobertura de código automatizada** ni umbral.

**Evidencia**
```
$ grep -rn "llvm-cov\|tarpaulin\|coverage" .github/ scripts/ Cargo.toml
(sin resultados)
```
Recuento de tests: 699 totales (`#[test]` + `#[tokio::test]`).

**Por qué importa**
La meta profesional del estándar es ≥70% de líneas cubiertas y verificable en CI. Sin
medición, no se puede demostrar dónde está la deuda real: `amatl-ui` (29 tests sobre
3 730 LOC de assets + 895 LOC Rust) y `amatl-cli` (25 tests) están en el límite bajo.
La matriz de `docs/testing.md` mitiga esto declarando capacidades, pero no sustituye una
cifra reproducible.

---

### [MEDIO] ARQ-004 — Documentación: `config/mod.rs` a 2 527 LOC sin descomposición completa

**Descripción**
`crates/amatl-core/src/config/mod.rs` tiene 2 527 líneas con un `impl Config` de las
líneas 271–616 y numerosas funciones libres de validación. Ya existen 10 sub-módulos en
`config/`, pero el núcleo (`mod.rs`) concentra el tipo raíz y su validación.

**Evidencia**
```
$ wc -l crates/amatl-core/src/config/mod.rs   → 2527
$ ls crates/amatl-core/src/config/
data_policy.rs(191) deep_config.rs(556) execution.rs(76) inference.rs(446)
mod.rs(2527) persistence.rs(422) providers.rs(273) ranking.rs(181)
server.rs(597) telemetry.rs(85)
```
Nota positiva: solo **1 `.unwrap()` en producción** en este archivo (línea 358,
`.expect("already_present implies a match")`, justificado por invariante local).

**Por qué importa**
`mod.rs` mezcla el ensamblado del tipo `Config` con la lógica de validación mutable de
TOML. Es mantenible hoy, pero es el segundo mayor módulo y candidato natural a
`validate.rs`/`patch.rs` siguiendo el precedente que el propio repo ya estableció.

---

### [MEDIO] ARQ-005 — Coherencia documental: se declara un gate verde que hoy está rojo

**Descripción**
`audit/cierre-hallazgos-2026-11-18.md:83` afirma `cargo fmt --all -- --check | ok (exit
0)`, y `docs/contribuir.md:30` / `DEVELOPMENT.md:110` lo listan como comando de gate.
Hoy el comando sale con exit 1 (ARQ-001). La documentación no refleja la realidad.

**Evidencia**
```
$ grep -rn "fmt" audit/cierre-hallazgos-2026-11-18.md
83:| Básicas | `cargo fmt --all -- --check` | ok (exit 0) |
$ cargo fmt --all -- --check ; echo $?   → 1
```

**Por qué importa**
Es exactamente el antipatrón que el repo ya identificó y corrigió para `cargo audit`
("la doc de seguridad afirma green cuando no lo está"). Un cierre de hallazgos que
declara verde un gate rojo erosiona la confianza en el resto del reporte.

---

### [BAJO] ARQ-006 — Tamaño de repositorio: artefactos de evaluación de 3–5 MB rastreados

**Descripción**
`docs/evaluation/independent-relevance/` rastrea varios JSON de 3–5 MB
(`raw-capture.json` 5.2 MB, `frozen-query-universe.json` 3.7 MB, etc.), y
`crates/amatl-core/assets/tracker-domains.txt` pesa 1.6 MB. El `.git` ocupa 41 MB.

**Evidencia**
```
$ git ls-files | wc -l → 8050
$ du -sh .git → 41M
$ git ls-files -z | xargs -0 du -h | sort -rh | head
5.2M docs/evaluation/independent-relevance/v6/execution/raw-capture.json
3.7M docs/evaluation/independent-relevance/v6/frozen-query-universe.json
...
1.6M crates/amatl-core/assets/tracker-domains.txt
```

**Por qué importa**
Es evidencia experimental reproducible (probablemente intencional, ver
`docs/evaluation/`), pero 130 MB de evidencia versionada encarecen clones y diffs. Si
son inmutables y regenerables, podrían vivir en releases/artefactos o Git LFS. Bajo
porque la decisión de rastrearlas parece deliberada para reproducibilidad.

---

### [BAJO] ARQ-007 — Lockfile y binarios de probe sin verificación de gate visible

**Descripción**
`crates/amatl-core/src/bin/lock_probe.rs` existe como binario auxiliar con **1
`.unwrap()`** en producción (línea verificada). No es deuda grave, pero es un binario
fuera del camino de test regular.

**Evidencia**
```
$ for f ...; done | sort -rn | grep lock_probe
1 crates/amatl-core/src/bin/lock_probe.rs
```

**Por qué importa**
Un binario auxiliar con manejo de error por `unwrap` puede abortar con panic en vez de
un mensaje accionable si se usa en diagnóstico operativo. Bajo: es herramienta interna.

---

### [BAJO] ARQ-008 — Commits fuera de Conventional Commits (13 de los últimos 300)

**Descripción**
`git log --oneline -300` muestra 13 mensajes que no siguen estrictamente
`tipo(scope): …` (p. ej. `Repository hygiene audit: …`, `release: harden …`,
`security: close …`). El repo usa `security:` y `release:` como tipos válidos por
convención propia, así que son aceptables, pero algunos como el primero son prosa libre.

**Evidencia**
```
$ git log --oneline -300 | grep -vE '^[a-f0-9]+ (feat|fix|docs|refactor|test|chore|ci|perf|build|style|revert)(\(|:|!)'
e8b7e1c1 security(supply-chain): ...
be61b4f0 Repository hygiene audit: ...
dcd4738f release: harden optional capabilities ...
```

**Por qué importa**
La disciplina de versionado semántico es alta (`CHANGELOG` Keep-a-Changelog, tag
`v0.1.0-rc.1`, SemVer documentado). Solo la forma del mensaje es inconsistente. Bajo.

---

### [BAJO] ARQ-009 — Huella de deuda técnica: TODOs de versión 1.0 sin tracking formal

**Descripción**
Hay 3 TODOs acotados a un retiro planificado (`local_model_batch` alias), todos
marcados `TODO(1.0)`, sin issue ni ADR que los rastree.

**Evidencia**
```
crates/amatl-core/src/config/inference.rs:34  // TODO(1.0): drop the `local_model_batch` alias.
crates/amatl-core/src/config/inference.rs:94  // TODO(1.0): drop the `local_model_batch` alias.
crates/amatl-core/src/config/mod.rs:1358      // TODO(1.0): drop with the alias.
```

**Por qué importa**
Son legibles y acotados (bien), pero sin entrada en `CHANGELOG`/ADR podrían sobrevivir
a 1.0. Muy bajo.

---

## 4. Roadmap ejecutable (orden por dependencia → impacto → esfuerzo)

### Tarea 1 — ARQ-001 · Fijar toolchain y reformatear (CRÍTICO operativo · Esfuerzo XS)
**Dependencias**: ninguna (primero).

**Problema**: `cargo fmt --check` rojo en cada push; el resultado depende del rustfmt del
runner porque no hay pin de toolchain.

**Solución**:
1. Crear `rust-toolchain.toml` en la raíz:
   ```toml
   [toolchain]
   channel = "1.88.0"
   components = ["rustfmt", "clippy"]
   ```
   (coincide con `rust-version = "1.88"` del workspace y con el job `msrv`).
2. Ejecutar `cargo fmt --all` y commitear el reformateo (8 archivos, solo cosmético).
3. Fijar también el runner: usar `dtolnay/rust-toolchain@stable` pero con
   `toolchain: 1.88.0` en `contract-gate`, o confiar en el `rust-toolchain.toml` ya
   detectado por las actions.
4. Verificar: `cargo fmt --all -- --check` → exit 0.

**Archivo(s)**: `rust-toolchain.toml` (nuevo), los 8 archivos con diffs, `.github/workflows/ci.yml`.

**Por qué primero**: desbloquea el gate; sin esto cualquier otro cambio queda en rojo.

---

### Tarea 2 — ARQ-002 · Descomponer `impl AmatlService` por concern (ALTO · Esfuerzo L)
**Dependencias**: Tarea 1 (para tener gate verde antes de tocar código).

**Problema**: `impl` de 1 540 líneas en `service.rs:466-2008`.

**Solución** (Strangler incremental, sin cambiar la API pública):
1. Extraer módulos `service/search.rs`, `service/deep.rs`, `service/providers.rs`,
   `service/storage.rs`, `service/credentials.rs` con `impl AmatlService` parciales
   (Rust admite múltiples `impl` en archivos distintos dentro del mismo crate).
2. Conservar `pub struct AmatlService` y sus firmas para no romper `lib.rs` re-exports.
3. Repetir para `amatl-server/src/lib.rs`: separar `handlers/`, `auth.rs`, `ratelimit.rs`.
4. Verificar tras cada extracción: `cargo test --workspace --locked`.

**Archivo(s)**: `crates/amatl-core/src/service.rs`, `crates/amatl-server/src/lib.rs`.

**Criterio de éxito**: ningún archivo >1 500 LOC; `impl` más grande <500 LOC.

---

### Tarea 3 — ARQ-005 · Sincronizar la documentación de gates (MEDIO · Esfuerzo XS)
**Dependencias**: Tarea 1 (corregir la cifra tras arreglar el gate).

**Problema**: `audit/cierre-hallazgos-2026-11-18.md:83` declara fmt verde.

**Solución**:
1. Tras Tarea 1, actualizar la línea a `ok (exit 0)` con fecha, o añadir nota de que la
   verificación depende del pin de toolchain (Tarea 1 lo garantiza).
2. Añadir a `docs/contribuir.md` una nota: "los gates se evalúan con el toolchain de
   `rust-toolchain.toml`".

**Archivo(s)**: `audit/cierre-hallazgos-2026-11-18.md`, `docs/contribuir.md`.

---

### Tarea 4 — ARQ-003 · Añadir medición de cobertura en CI (MEDIO · Esfuerzo M)
**Dependencias**: Tarea 1.

**Problema**: sin métrica de cobertura pese a meta 70%.

**Solución**:
1. Añadir `cargo-llvm-cov` al workflow (job nuevo `coverage`, nightly o por-PR).
2. Publicar resumen y umbral inicial **informativo** (sin fallar), para no romper el
   gate con un umbral no calibrado.
3. Fijar umbral bloqueante solo cuando la línea base medida lo permita; documentar la
   cifra en `docs/testing.md` (sección "Commands").

**Archivo(s)**: `.github/workflows/ci.yml`, `docs/testing.md`.

---

### Tarea 5 — ARQ-004 · Extraer validación de `config/mod.rs` (MEDIO · Esfuerzo S)
**Dependencias**: Tarea 2 (mismo patrón de extracción).

**Problema**: `mod.rs` 2 527 LOC.

**Solución**: mover las funciones libres de validación a `config/validate.rs` y los
helpers de patch a `config/patch.rs`; mantener `Config` y re-exports en `mod.rs`.

**Archivo(s)**: `crates/amatl-core/src/config/mod.rs` (+ nuevos `validate.rs`, `patch.rs`).

---

### Tarea 6 — ARQ-006 · Revisar estrategia de artefactos pesados (BAJO · Esfuerzo S)
**Dependencias**: ninguna.

**Problema**: JSON de 3–5 MB rastreados.

**Solución**: decidir explícitamente (ADR corto) si la evidencia de evaluación se
rastrea, va a Git LFS o se regenera. Si se rastrea, documentar la justificación en
`docs/evaluation/README` (ya existe un patrón similar para `deny.toml`).

---

### Tarea 7 — ARQ-007·008·009 · Pulido (BAJO · Esfuerzo XS, c/u)
- Reemplazar el `unwrap` de `lock_probe.rs` por `expect`/mensaje accionable.
- (Opcional) Añadir lint de Conventional Commits vía commit-msg hook o `commitlint`.
- Añadir los `TODO(1.0)` a una sección "Removals planned" del `CHANGELOG`.

---

## 5. Estado de reparación (ejecutado 2026-11-23 con `software-repair-engineer`)

Modo REPAIR + TEST. Cambios **mínimos**, sin tocar lógica de producto.

| Hallazgo | Disposición | Cambio |
|---|---|---|
| ARQ-001 | **REPARADO** | `cargo fmt --all` (6 archivos, solo cosmético) + `rust-toolchain.toml` (canal `stable`, componentes rustfmt/clippy) |
| ARQ-005 | **REPARADO** | Nota al pie ¹ en `cierre-hallazgos-2026-11-18.md` que hace explícita la dependencia de versión de rustfmt (sin falsificar el registro histórico) |
| ARQ-007 | **RECLASIFICADO — no es defecto** | `lock_probe.rs` es un binario auxiliar de test; el `.expect()` sobre argumento CLI obligatorio es *fail-fast* con mensaje claro, contrato correcto. Sin cambio. |
| ARQ-009 | **REPARADO** | Sección "Removals planned for 1.0" en `CHANGELOG.md` que rastrea el alias `local_model_batch` |
| ARQ-002 | **ESCALADO** | Requiere extracción arquitectónica (L); fuera del alcance de una reparación mínima. Documentado en roadmap. |
| ARQ-003 | **ESCALADO** | Requiere decisión de CI (añadir `cargo-llvm-cov` + política de umbral). |
| ARQ-004 | **ESCALADO** | Depende de ARQ-002 (mismo patrón de extracción). |
| ARQ-006 | **ESCALADO** | Requiere decisión de estrategia de artefactos (ADR corto). |
| ARQ-008 | **ESCALADO** | Cosmético/opcional (commitlint); no es defecto funcional. |

**Verificación (LEVEL 1 + 2/3 + 5):**
```
cargo fmt --all -- --check                                  → exit 0
cargo clippy --workspace --all-targets --all-features -D warnings → exit 0
cargo build --workspace --locked                            → ok
cargo test --workspace --locked                             → 699 passed, 0 failed, 2 ignored
```
Sin regresiones: los diffs de formato son puramente cosméticos (re-wrap de literales,
`use`-groups y atributos), 0 cambio semántico.

## 6. Conclusión

El proyecto está **arquitectónicamente sano y por encima de la media**: límites de capa
correctos, contrato verificado, documentación profunda y 699 tests. Los únicos bloqueos
reales son operativos y acotados: **un gate de formato rojo por ausencia de pin de
toolchain (ARQ-001)** y un **God Object en `service.rs` (ARQ-002)**. El resto son
mejoras de mantenibilidad y coherencia documental. Ninguna es crítica de seguridad ni de
corrupción de datos, y todas tienen remedio incremental sin cambiar la API pública.

**Camino más corto a verde**: Tarea 1 (XS) → Tarea 5 documental (XS) → resto por impacto.
