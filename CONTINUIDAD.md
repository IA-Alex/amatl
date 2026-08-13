# Continuidad de desarrollo — AMATL

## Snapshot verificable

Estado revisado el **2026-08-13** sobre la rama `main`:

- revisión de partida: `f10f8fa` (`docs: complete repository documentation and continuity`);
- baseline de implementación: tag `baseline-fases-0-9`, commit `51c6d34`;
- workspace: Rust 2021, MSRV 1.88, versión candidata `0.1.0-rc.1`, cuatro crates;
- fases 0–9: cerradas y verificadas;
- publicación SemVer: todavía inexistente; pipeline reproducible de RC preparado;
- Fase 10: no existe en el golden template y no debe inferirse.

Los documentos rectores permanecen intactos:

| Documento | SHA-256 verificado |
|---|---|
| `plan_amatl.md` | `c8545d7bacb9f17131e7b901693a035e038532c0836f93df6eb9c78858d5309c` |
| `fase_a_contratos.md` | `03034b7abfbcfaba3da7ada7b43267ed38936ff09453326cd9db40b2cede4744` |
| `decisiones_amatl.md` | `52f465d22cf64fc18f62a5ef89617e3e9456dbba8b19080f426fa4c094d50d18` |

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
- `provider-canary` aísla una fuente real y valida enablement, gobernanza y
  credencial antes de red; su workflow sólo puede iniciarse manualmente bajo un
  environment aprobado.
- El benchmark `controlled-local-v1` mide Search/Deep, concurrencia SQLite y RSS
  sin atribuir esos valores a Internet o producción.
- El workflow de release produce el binario estático Linux musl, SBOMs y
  checksums. La attestation queda condicionada a soporte de GitHub y se omite de
  forma explícita en este repositorio privado; sólo un tag anotado y concordante
  publica prerelease.

## Evidencia de validación

La compuerta completa requerida para la candidata es:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace --benches
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cargo deny check
cargo cyclonedx
```

Resultados locales registrados el 2026-08-13:

- 167 pruebas aprobadas en el workspace;
- formato, benches y Clippy estricto aprobados;
- Cargo Audit sin vulnerabilidades y Cargo Deny aprobado (duplicados
  transitivos permitidos por la política vigente);
- CycloneDX generado para los cuatro crates;
- `cargo +1.88.0 check --workspace --all-targets --locked` aprobado;
- handshake HTTPS real contra rustls con certificado temporal confiado y
  rechazo de certificados no confiables;
- límites HTTP conflictivos rechazados sobre TCP real y eventos de seguridad
  verificados sin credenciales;
- límite agregado de cabeceras y cancelación por timeout del handler verificados
  con códigos estables y `X-Request-ID`;
- `X-Request-ID` correlaciona respuestas y eventos SSRF sin confiar en valores
  del cliente ni registrar la URL rechazada;
- canario fail-closed antes de red cuando falta aprobación;
- benchmark operativo completo bajo concurrencia;
- serialización de mutaciones SQLite compartida entre clones.

El target `x86_64-unknown-linux-musl` está instalado localmente, pero el host no
permite instalar `musl-tools` sin contraseña de administrador. El build local
se detuvo al no encontrar `x86_64-linux-musl-gcc`; el workflow instala el
toolchain de sistema en un runner limpio antes de construir. La
publicación y la atestación sólo pueden verificarse en GitHub Actions.

Ranking v2 conserva el baseline reproducible del corpus etiquetado:
`nDCG@3 0.655768 → 0.919378`; esto no reemplaza benchmarks ambientales.

`cargo cyclonedx` genera archivos dentro de cada crate; `.gitignore` ya cubre
`**/*.cdx.xml` y `**/*.cdx.json` para impedir que artefactos locales entren al
historial.

## Pendientes que requieren decisión externa

No son defectos del core y no deben resolverse inventando datos:

1. Definir propietarios verificables, `CODEOWNERS`, canal privado de seguridad,
   tiempos de respuesta y cumplimiento, y URL pública canónica.
2. Marcar `contract-gate` como required check en la protección de `main`.
3. Completar aprobación, ToS, cuotas, costes, región y credenciales de cada
   provider antes de habilitar red real.
4. Configurar el environment `provider-canary`, sus revisores y secretos; luego
   capturar latencia/errores reales sin incorporar credenciales al repositorio.
5. Configurar retención de artefactos y ejecutar el workflow de RC antes de
   crear un tag; la publicación externa requiere autoridad del propietario.
6. Repetir carga en el host objetivo y diseñar aislamiento CDP antes de medir o
   habilitar Renderer.

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

El siguiente paso inmediato no es crear una Fase 10: es configurar los controles
externos de GitHub, cargar gobernanza/credenciales en el environment protegido y
ejecutar el canario. Con esa evidencia, ejecutar manualmente el build de RC,
verificar el checksum en una máquina limpia (y la attestation sólo cuando el
hosting la soporte) y únicamente entonces crear el tag anotado
`v0.1.0-rc.1`.
