# Continuidad de desarrollo — AMATL

## Snapshot verificable

Estado revisado el **2026-08-13** sobre la rama `main`:

- revisión funcional documentada: `2b9baa9` (`feat: expose Deep evidence in the UI`);
- baseline de implementación: tag `baseline-fases-0-9`, commit `51c6d34`;
- workspace: Rust 2021, MSRV 1.88, versión candidata `0.1.0-rc.1`, cuatro crates;
- fases 0–9: cerradas y verificadas;
- publicación SemVer: RC actual `0.1.0-rc.1`; estado externo verificable en GitHub Releases;
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
| 8 | Cerrada y ampliada | UI estática, embebida y responsiva para Search y Deep/Evidence v2 |
| 9 | Cerrada | servidor Axum compartido, REST, MCP, bearer, TLS y hardening HTTP |

## Avances del flujo Evidence v2 → ingestión → UI Deep

Este bloque registra la secuencia implementada después del baseline, sin crear
una fase nueva ni cambiar el contrato global `schema_version = "1"`:

| Revisión | Entrega | Resultado verificable |
|---|---|---|
| `5d2e15d` | Evidence v2 | fragmentos exactos, offsets UTF-8, SHA-256, señales deterministas y procedencia enlazada a cada `Document` |
| `6c5572f` | Ingestión local | despacho acotado por tipo documental y producción de Document/Evidence v1/v2 sólo por CLI, sin ruta HTTP/MCP |
| `2b9baa9` | UI Deep | `POST /deep`, presentación de documentos/fragmentos/procedencia y verificación de integridad en navegador |

La ampliación de UI realizada en `2b9baa9` incluye:

- dos acciones sobre el mismo formulario: `Buscar` usa `POST /search` y
  `Analizar evidencia` usa `POST /deep`; ambos conservan filtros y bearer sólo
  en memoria;
- correlación de `EvidenceV2.document_id` con `Document.search_result_id`, y
  rechazo visual de evidencia cuyo `provenance.document_id` o linaje de URL no
  coincide con el documento;
- presentación de estado documental, fragmentos, señales observadas, método de
  adquisición, extractor, fecha de recuperación, linaje de URL y hashes de
  fuente/texto extraído;
- límites defensivos en UI de 20 documentos, ocho fragmentos por documento,
  512 bytes por fragmento y 8 MiB de contenido verificable;
- reconstrucción de `start_byte..end_byte` con decodificación UTF-8 estricta y
  recálculo SHA-256 mediante Web Crypto; la interfaz distingue rango/hash
  verificados, sólo rango verificable y fallo de verificación;
- renderizado exclusivo con `textContent` y nodos DOM, enlaces limitados a
  HTTP(S) sin credenciales, sin `innerHTML`, `document.write`, selector de
  archivos ni `FileReader`;
- estados accesibles para carga, éxito, degradación, autenticación, rate limit y
  timeout, con adaptación móvil y respeto a `prefers-reduced-motion`.

La UI no calcula evidencia, ranking ni procedencia: consume el contrato del
core. Tampoco introduce inferencia o LLM. La ingestión local continúa separada
y sólo accesible mediante `amatl ingest`, evitando convertir el listener en un
lector remoto del filesystem.

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
- La UI usa POST JSON para Search y Deep; consulta y bearer no aparecen en la
  URL y el token no tiene `name` de formulario ni persistencia de aplicación.
- La vista Deep limita documentos/fragmentos, conserva la procedencia Evidence
  v2, renderiza texto no confiable sin HTML y verifica rango/hash en navegador.
- MCP conserva límites más estrictos que CLI/API.
- Un bind no-loopback exige autenticación y TLS completos.
- Secretos sólo por variables de entorno; no en TOML, logs ni URLs.
- No introducir LLM obligatorio, agent loops, crawler masivo ni nueva
  infraestructura sin decisión explícita.
- Toda salida de red pasa por `data_policy`. `isolated` exige `egress = deny`,
  bind loopback y cero providers/renderer/inferencia remota; Search y extracción
  de evidencia siguen siendo independientes de LLM.
- Evidence v2 es aditivo: fragmentos exactos, acotados y enlazados a procedencia
  acompañan la evidencia v1 sin recalibrar Ranking v2 ni Gap.
- La ingestión de archivos es sólo CLI: despacha tipos en core, produce
  `Document` y Evidence v1/v2, no ejecuta Search y nunca expone rutas por
  HTTP/MCP. PDF respeta `data_policy` antes de crear `pdftotext`.

## Disponibilidad real y límites

- `MockProvider` es la vía determinista para desarrollo y pruebas sin red.
- Brave y Mojeek tienen adapters, pero permanecen sujetos a configuración,
  credencial y aprobación vigente de gobernanza; ningún provider real está
  activo por defecto.
- DuckDuckGo HTML está bloqueado fail-closed con
  `provider_pending_explicit_approval`.
- Trafilatura es opcional; su ausencia degrada Deep a documento superficial.
- La UI puede mostrar Search con el mock sin red, pero una vista Deep enriquecida
  exige candidatos obtenibles bajo `data_policy` y texto del extractor. Con
  `isolated`, el botón Deep muestra degradación sin intentar DNS; para archivos
  sensibles se usa la ingestión CLI, no la UI.
- `ChromiumRenderer` permanece no disponible hasta implementar y verificar un
  backend CDP aislado; no habilitar Chromium como fallback inseguro.
- Persistencia y ambas cachés están deshabilitadas por defecto. Un fallo de
  SQLite no invalida Search.
- `/health` sólo comprueba proceso/router; no prueba providers, SQLite ni
  credenciales.
- `provider-canary` aísla una fuente real y valida enablement, gobernanza y
  credencial antes de red; su workflow sólo puede iniciarse manualmente bajo un
  environment aprobado.
- El perfil `isolated` bloquea provider, canary, Deep y MCP fetch antes de DNS;
  `local_only` reserva inferencia local opcional, pero no existe backend LLM en
  la implementación actual ni se requiere API key.
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

- 190 pruebas aprobadas en el workspace;
- pruebas de UI verifican despacho POST Search/Deep, DOM seguro, límites,
  correlación de procedencia, uso de Web Crypto y ausencia de superficie local;
- prueba de servidor verifica autenticación y contrato `POST /deep` bajo perfil
  aislado, incluida la degradación `egress_denied`, y confirma que `/ingest` no
  existe en HTTP;
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

La ejecución de GitHub Actions `contract-gate` para `2b9baa9` terminó en verde
el 2026-08-13: aprobó el job principal completo y el job MSRV. `HEAD` y
`origin/main` quedaron alineados en esa revisión antes de esta actualización
documental.

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

1. La identidad `@IA-Alex`, `CODEOWNERS`, canal privado, correo de escalación,
   SLA y URL canónica ya están definidos; mantenerlos vigentes.
2. Marcar `contract-gate` como required check cuando GitHub habilite protección
   para este repositorio privado (requiere plan superior o hacerlo público).
3. Completar aprobación, ToS, cuotas, costes, región y credenciales de cada
   provider antes de habilitar red real.
4. Configurar el environment `provider-canary`, sus revisores y secretos; luego
   capturar latencia/errores reales sin incorporar credenciales al repositorio.
5. Para cada RC futura, ejecutar el workflow, validar musl/.deb/.rpm/Arch y sólo
   después crear el tag anotado; la publicación externa requiere autoridad del
   propietario.
6. El aislamiento real de Chromium está probado sin red; implementar y revisar
   el bridge CDP antes de habilitar Renderer en core.

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
`/providers`, MCP `/mcp`. En la página, `Buscar` conserva el flujo ligero y
`Analizar evidencia` ejecuta Deep y despliega procedencia/verificación. El token
introducido debe ser el mismo valor exportado en `AMATL_SERVER_TOKEN`.

## Próximo hito seguro

El siguiente paso técnico no es crear una Fase 10 ni cambiar contratos: es pasar
a pulido verificable de la interfaz sobre la superficie ya funcional. Debe
añadir pruebas browser E2E para Search/Deep, navegación por teclado,
accesibilidad automatizada, estados vacío/superficial/degradado y revisión
visual responsiva; después puede afinar jerarquía, densidad y legibilidad sin
introducir lógica de producto en `amatl-ui`.

En paralelo, los controles externos de GitHub, gobernanza/credenciales del
environment y canario real continúan como decisiones del propietario. No
bloquear el pulido local por falta de APIs ni simular su aprobación; futuras
publicaciones deben repetir las compuertas documentadas.
