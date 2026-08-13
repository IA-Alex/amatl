# Continuidad de desarrollo — AMATL

## Snapshot verificable

Estado revisado el **2026-08-12** sobre la rama `main`:

- revisión actual: `95a2ec5` (`docs: distinguish operational benchmark evidence`);
- baseline de implementación: tag `baseline-fases-0-9`, commit `51c6d34`;
- workspace: Rust 2021, versión interna `0.1.0`, cuatro crates;
- fases 0–9: cerradas y verificadas;
- publicación/release SemVer: todavía inexistente;
- Fase 10: no existe en el golden template y no debe inferirse.

Los documentos rectores permanecen intactos:

| Documento | SHA-256 verificado |
|---|---|
| `plan_amatl.md` | `c8545d7bacb9f17131e7b901693a035e038532c0836f93df6eb9c78858d5309c` |
| `fase_a_contratos.md` | `03034b7abfbcfaba3da7ada7b43267ed38936ff09453326cd9db40b2cede4744` |

## Jerarquía para retomar trabajo

En caso de discrepancia, usar este orden:

1. `plan_amatl.md` y `fase_a_contratos.md` para intención e invariantes.
2. Código, tests, migraciones, `Cargo.lock` y `amatl.example.toml` para el
   comportamiento realmente implementado.
3. ADRs y documentación especializada para decisiones y operación.
4. Este archivo sólo como snapshot de continuidad; no sustituye contratos ni
   evidencia ejecutable.

## Estado funcional

| Fase | Estado | Capacidad entregada |
|---|---|---|
| 0–2 | Cerrada | contratos Rust, Query/Classification/Plan/Budget, providers, Search y pipeline de resultados |
| 3–4 | Cerrada | SQLite/cachés/telemetría opcionales y routing adaptativo/progresivo |
| 5 | Cerrada | Deep acotado, fetch seguro, extracción, documentos/evidencias y cache documental |
| 6–7 | Cerrada | Ranking v2 calibrable, Diversity y Gap Analyzer con límites propios |
| 8 | Cerrada | UI estática, embebida y responsiva |
| 9 | Cerrada | servidor Axum compartido, REST, MCP, bearer, TLS y hardening HTTP |

Arquitectura vigente:

- `amatl-core`: única ubicación de contratos y lógica de producto;
- `amatl-cli`: adaptación CLI, códigos de salida y arranque del servicio;
- `amatl-server`: UI/API/MCP sobre un listener y un `AmatlService`;
- `amatl-ui`: assets embebidos y política de headers;
- `SearchOrchestrator` y `DeepOrchestrator`: únicos dueños de sus presupuestos y
  deadlines;
- SQLite, cachés, Trafilatura y Renderer quedan fuera del correctness de Search.

Invariantes no negociables:

- Search no ejecuta fetch, render ni extracción y nunca expone `final_url`.
- Deep es la única frontera de navegación y aplica controles SSRF antes de
  conectar, después de DNS y en cada redirect.
- CLI, UI, API y MCP consumen el mismo core; no duplicar orquestación.
- MCP conserva límites más estrictos que CLI/API.
- Un bind no-loopback exige autenticación y TLS completos.
- Secretos sólo por variables de entorno; no en TOML, logs ni URLs.
- No introducir LLM obligatorio, agent loops, crawler masivo ni nueva
  infraestructura sin decisión explícita.

## Disponibilidad real y límites

- `MockProvider` es la vía determinista para desarrollo y pruebas sin red.
- Brave y Mojeek tienen adapters, pero permanecen sujetos a configuración,
  credencial y aprobación vigente de gobernanza; ningún provider real está
  activo por defecto.
- DuckDuckGo HTML está bloqueado fail-closed con
  `provider_pending_explicit_approval`.
- Trafilatura es opcional; su ausencia degrada Deep a documento superficial.
- `ChromiumRenderer` permanece no disponible hasta implementar y verificar un
  backend CDP aislado; no habilitar Chromium como fallback inseguro.
- Persistencia y ambas cachés están deshabilitadas por defecto. Un fallo de
  SQLite no invalida Search.
- `/health` sólo comprueba proceso/router; no prueba providers, SQLite ni
  credenciales.

## Paquete documental pendiente de commit

El árbol de trabajo contiene una ampliación documental coherente con el código,
pero **aún no está versionada**. Debe revisarse y confirmarse como una sola
unidad lógica antes de continuar con otro alcance.

- Modificados: `README.md`, `DEVELOPMENT.md`, `decisiones_amatl.md` y este
  documento.
- Nuevos: licencias MIT/Apache-2.0, changelog, políticas de contribución,
  conducta y seguridad, plantillas de GitHub y `CODEOWNERS` conservador.
- Nuevos bajo `docs/`: arquitectura, glosario, configuración, operación,
  testing, benchmarks, gobernanza de providers, OpenAPI, MCP y controles de
  seguridad.
- No hay cambios pendientes en `crates/`, `plan_amatl.md` ni
  `fase_a_contratos.md`.

Índices principales: `README.md`, `docs/arquitectura.md`,
`docs/api/openapi.yaml`, `docs/api/mcp.md`, `docs/operacion.md` y
`docs/security/threat-model.md`.

## Evidencia de validación

El 2026-08-12 se ejecutó con éxito la compuerta completa:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace --benches
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cargo deny check
cargo cyclonedx
```

Resultados registrados:

- 143 pruebas aprobadas en el workspace;
- formato, benches y Clippy sin errores ni warnings admitidos por Clippy;
- Cargo Audit: cero vulnerabilidades detectadas en el lockfile;
- Cargo Deny: aprobado; los avisos de versiones transitivas duplicadas están
  permitidos por la política actual y no equivalen a fallo;
- CycloneDX: generación correcta para los cuatro crates;
- 29 documentos Markdown sin enlaces locales rotos;
- OpenAPI 3.1 parseable, con todas las referencias internas resueltas;
- `git diff --check` limpio.

Ranking v2 conserva el baseline reproducible del corpus etiquetado:
`nDCG@3 0.655768 → 0.919378`; esto no reemplaza benchmarks ambientales.

Nota operativa: `cargo cyclonedx` genera archivos `*.cdx.xml` dentro de cada
crate. Los patrones actuales de `.gitignore` sólo cubren la raíz; esos archivos
se retiraron después de validar y no deben entrar accidentalmente al próximo
commit. Corregir el patrón requiere un cambio separado y autorizado.

## Pendientes que requieren decisión externa

No son defectos del core y no deben resolverse inventando datos:

1. Revisar y crear el commit del paquete documental pendiente.
2. Definir propietarios verificables, `CODEOWNERS`, canal privado de seguridad,
   tiempos de respuesta y cumplimiento, y URL pública canónica.
3. Marcar `contract-gate` como required check en la protección de `main`.
4. Completar aprobación, ToS, cuotas, costes, región y credenciales de cada
   provider antes de habilitar red real.
5. Definir publicación y retención de SBOM y política de releases/artefactos.
6. Capturar en el entorno objetivo latencia de red/Deep, memoria, SQLite bajo
   carga y Renderer; Criterion sólo cubre el core reproducible.
7. Diseñar el aislamiento CDP antes de habilitar Renderer.
8. Decidir si se fija `rust-version` y se añade un job MSRV. El lockfile actual
   exige efectivamente Rust 1.88; CI usa `stable` y la validación local usó
   Rust 1.97.1.

## Protocolo de reanudación

Antes de editar:

```bash
git status --short
git log -3 --oneline --decorate
sha256sum plan_amatl.md fase_a_contratos.md
git diff --check
```

Después:

1. Revisar primero cambios tracked y untracked; no descartar el paquete
   documental pendiente.
2. Confirmar que la tarea nueva está dentro de una fase existente o crear una
   ADR explícita si amplía alcance.
3. Implementar desde `amatl-core` y adaptar bordes sin duplicar lógica.
4. Añadir pruebas contractuales en la misma frontera afectada.
5. Ejecutar la compuerta completa mostrada arriba y retirar SBOM locales
   generados antes de preparar el commit.

## Ejecución local segura

```bash
cargo run -p amatl-cli -- search "rust async" --json --mock
cargo run -p amatl-cli -- deep "rust async" --json --mock
export AMATL_SERVER_TOKEN="$(openssl rand -hex 32)"
cargo run -p amatl-cli -- serve --mock
```

Defaults: `127.0.0.1:8080`; UI `/`, health `/health`, REST `/search`, `/deep` y
`/providers`, MCP `/mcp`.

## Próximo hito seguro

El siguiente paso inmediato no es crear una Fase 10: es **revisar, validar y
versionar el paquete documental actual**. Después, cualquier ampliación debe
partir de uno de los pendientes explícitos, con ADR, contrato, pruebas y
criterios de aceptación antes de modificar implementación.
