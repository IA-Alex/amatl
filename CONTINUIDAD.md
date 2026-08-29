# Continuidad de desarrollo — AMATL

## Snapshot verificable

Estado revisado el **2026-08-16** sobre la rama `consolidacion-ui-observabilidad`:

- último commit: `e9b0c9e` (`feat(answer): síntesis de respuesta citada opcional, tema claro/oscuro y refresco de marca`);
- baseline de implementación: tag `baseline-fases-0-9`, commit `51c6d34`;
- workspace: Rust 2021, MSRV 1.88, versión candidata `0.1.0-rc.1`, cuatro crates;
- fases 0–9: cerradas y verificadas;
- publicación SemVer: RC actual `0.1.0-rc.1`; estado externo verificable en GitHub Releases;
- Fase 10: no existe en el golden template y no debe inferirse; `answer` (ver
  ADR-011) tampoco es una fase — es una capacidad opcional transversal,
  documentada como tal en `docs/configuracion.md`.

Commits de esta rama sobre `ce99222`:

| Commit | Alcance |
|---|---|
| `56c5ec1` | Backups durables, extracción acotada y reranker por defecto medido |
| `1658a02` | Publicación en crates.io y AUR sólo en tags estables |
| `bd266cc` | Renderer Chromium conectado a través del harness de aislamiento |
| `71637d3` | Coste y viabilidad real de cada provider (sólo documentación) |
| `f46ad90` | SearXNG y Marginalia: adapter real, DuckDuckGo HTML retirado |
| `e9b0c9e` | Resumen con IA (grounded, opcional), tema claro/oscuro, refresco de marca, ADR-011 |

El binario `target/release/amatl` corriendo en el operador (PID variable por
sesión, ver `pgrep -af 'target/release/amatl'`) corresponde exactamente a
`e9b0c9e` — reconstruido después del commit, no antes, para no dejar ambigüedad
entre "lo compilado" y "lo comiteado". Primera ronda de pruebas en ambiente
real (no `--mock`) iniciada el 2026-08-16 contra SearXNG + Marginalia +
DeepInfra reales: `/search` y `/answer` verificados con resultados reales, no
sólo con el smoke test de CI.

Estado de los documentos rectores:

| Documento | SHA-256 | Estado |
|---|---|---|
| `plan_amatl.md` | `0fdd6761cb8c568145d6e00dfa0d37d56d68b02239bac8a92c061d4fb4b9ae11` | **Modificado** en `56c5ec1` |
| `fase_a_contratos.md` | `03034b7abfbcfaba3da7ada7b43267ed38936ff09453326cd9db40b2cede4744` | Intacto |
| `decisiones_amatl.md` | `3b260a0f34c934b61e103f71303b3c4effbebf3a0e231e2ff5574e41d0a36ee5` | **Modificado** (append-only): ADR-010 añadida, 2026-08-15 |

`plan_amatl.md` declaraba `Evidence` y `Gap` como «stub post-MVP» cuando
`evidence.rs` (527 líneas) y `gaps.rs` (513) llevaban implementados desde la
fase 5. La corrección alinea el documento con el código; la normativa original
se conserva como registro.

`decisiones_amatl.md` no es un documento protegido por ADR-001 (sólo
`plan_amatl.md` y `fase_a_contratos.md` lo son); su cambio es append-only —
ADR-010 documenta el retiro de `duckduckgo_html` y la implementación real de
Marginalia sin reescribir ADR-005.

**Actualización 2026-08-15 (comiteada como `f46ad90`):** cierre de la Etapa 1 de la brecha
de proveedores — `providers/marginalia.rs` deja de ser scaffold (adapter real
contra `api2.marginalia-search.com`, header `API-Key`), `router.rs` penaliza
por `estimated_cost`, y `duckduckgo_html` se retiró del código, del registro y
de la documentación de gobernanza (no es un adapter apagado: dejó de existir).
Compuerta local verificada: `cargo fmt --check`, `cargo clippy -D warnings` y
`cargo test --workspace` en verde. Detalle en ADR-010 y en
`docs/gobernanza-providers.md`.

**Actualización 2026-08-16 (comiteada como `e9b0c9e`):** cuatro bloques de
trabajo nuevos, todos en `amatl-core`/`amatl-server`/`amatl-ui` y su
documentación, ninguno cambia `schema_version` ni las invariantes de Search:

1. **"Resumen con IA" (síntesis de respuesta, nuevo módulo `answer.rs`).**
   Sintetiza una respuesta en español citando `[n]` sólo sobre índices de
   fuente reales (`extract_citations` valida, `strip_invalid_citations`
   elimina marcadores fabricados del texto visible de forma segura en UTF-8);
   requiere `data_policy.inference = "remote_explicit"` y `[answer]` con
   credencial propia por variable de entorno — apagado y sin llamar a nadie
   por defecto. Expuesto en HTTP (`POST /answer`), MCP y UI (botón `Resumen
   con IA`, siempre visible, deshabilitado visualmente cuando no está
   disponible). Un interruptor `POST /answer/enabled` con scope `admin`
   permite encenderlo/apagarlo desde la propia UI: valida la configuración
   candidata completa antes de escribir, escribe sólo `answer.enabled` en
   `amatl.toml` con `toml_edit` (conserva comentarios y el resto del
   archivo intacto) y recarga el servicio sin reinicio. `AnswerStatus`
   separa a propósito `enabled` (intención de config), `configured`
   (endpoint+modelo presentes) y `available` (credencial cargada) como tres
   campos independientes — un bug real de diseño anterior ataba `configured`
   a `enabled` y volvía indescubrible el propio interruptor al apagar la
   función. Documentado en `docs/resumen-con-ia.md`.
2. **Selector de tema claro/oscuro.** Paleta clara completa en
   `styles.css` (verificada WCAG-AA), `data-theme` + `prefers-color-scheme`,
   ícono único visible por estado (sol u luna) mediante clase `is-active`,
   no `hidden`/`display` en el propio SVG — la primera implementación
   mostraba ambos íconos a la vez porque `element.hidden` en un `SVGElement`
   no es fiable entre motores.
3. **Reemplazo íntegro del logo.** Ícono de marca y favicon nuevos
   (`brand-icon.png`, `favicon.png`, PNG embebidos vía `include_bytes!`),
   sustituyendo el símbolo geométrico anterior por completo. Alcance
   acotado explícitamente por decisión del propietario: sólo el símbolo usa
   el color café de la imagen origen; el resto de la aplicación conserva su
   paleta funcional azul/cian/esmeralda, y el wordmark conserva JetBrains
   Mono. Paleta completa (oscura y clara) documentada como fuente única de
   verdad en `docs/identidad-visual.md`, con la regla de no introducir
   colores funcionales nuevos sin documentarlos ahí. El subtítulo
   `"Búsqueda multifuente y evidencia verificable"` bajo la marca se retiró
   por completo (leía como texto publicitario) — la cabecera sólo conserva
   marca y selector de tema.
4. **Curación operativa de motores de SearXNG (fuera de este repositorio,
   sin cambio de código).** El contenedor Docker autohospedado empezó a
   devolver cero resultados reales porque los motores upstream por defecto
   (Brave, DuckDuckGo, Google CSE, Startpage) bloqueaban la IP del operador
   por volumen de pruebas — confirmado con `docker logs searxng`, no con
   pruebas a nivel de AMATL. Se descartó explícitamente rotar IP/proxy por
   ir contra los términos de esos motores. La corrección fue editar
   `/etc/searxng/settings.yml` dentro del contenedor para deshabilitar los
   motores bloqueados y habilitar otros más tolerantes (Bing confirmado con
   resultados reales tras el cambio; Mojeek/Qwant quedaron habilitados aun
   fallando en las pruebas porque no perjudican el agregado cuando fallan).
   `persistence` y `cache.provider_search` se habilitaron en el
   `amatl.toml` del operador, pero la caché de ambos providers activos
   sigue siendo un no-op real porque `storage_rights = false` en sus
   fichas — no se cambió `storage_rights` "por conveniencia técnica",
   siguiendo `docs/gobernanza-providers.md`.

Compuerta completa verificada tras estos cuatro bloques y antes de comitear:
`cargo fmt --all -- --check`, `cargo test --workspace`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo doc` (con
`RUSTDOCFLAGS="-D warnings"` — encontró y corrigió un intra-doc link privado
real en `answer.rs`), `cargo audit`, `cargo deny check`, `cargo cyclonedx`.
Documentación de contrato y gobernanza puesta al día en el mismo commit:
ADR-011 (`decisiones_amatl.md`), `docs/security/threat-model.md` (nuevo
límite `Core → inference (answer)`), `docs/api/openapi.yaml` (`/answer`,
`/answer/enabled`, `AnswerStatus`/`AnswerResult`), `docs/configuracion.md`,
`docs/operacion.md`, `CHANGELOG.md`. De paso se corrigió un patrón de
`.gitignore` que no cubría los backups reales que `storage.rs` genera
(`amatl.backup-<timestamp>.sqlite3` no coincidía con `/amatl.sqlite3*`).
Comiteado como `e9b0c9e`.

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
- Brave y Mojeek tienen adapters completos, pero están **descartados por
  política del operador** (`approval_status = "rejected"` por defecto, no
  `draft`): ambos son de pago y no se contratan providers de pago. No es un
  papeleo de gobernanza pendiente — reactivarlos exige revertir esa decisión
  explícitamente. Ver «Estado actual verificable» en
  `docs/gobernanza-providers.md`.
- SearXNG y Marginalia tienen adapter real y completo (ficha aprobable); ninguno
  está activo por defecto — falta `reviewer`/`reviewed_at`/`approval_status`
  con identidad real, decisión del propietario, no del código. Son los dos
  candidatos gratuitos.
- `duckduckgo_html` se retiró: DuckDuckGo no ofrece API de búsqueda web, sólo
  Instant Answer (no devuelve resultados web). No queda como adapter apagado.
- Trafilatura es opcional; su ausencia degrada Deep a documento superficial.
- La UI puede mostrar Search con el mock sin red, pero una vista Deep enriquecida
  exige candidatos obtenibles bajo `data_policy` y texto del extractor. Con
  `isolated`, el botón Deep muestra degradación sin intentar DNS; para archivos
  sensibles se usa la ingestión CLI, no la UI.
- `ChromiumRenderer` ejecuta JavaScript a través del harness
  `amatl-chromium-sandbox`. Recibe bytes, no una URL: `SafeFetcher` sigue siendo
  el único dueño del egress y el renderer no puede navegar. Queda no disponible
  —sin fallback inseguro— si faltan el harness, `bwrap`, `systemd-run` o el
  binario de Chromium.
- Persistencia y ambas cachés están deshabilitadas por defecto. Un fallo de
  SQLite no invalida Search.
- `/health` es la sonda de *liveness*: sólo comprueba proceso/router y devuelve
  `200` siempre. Es deliberado, porque un orquestador la usa para decidir si
  reinicia el proceso.
- `/ready` es la sonda de *readiness*, también pública: `200` cuando la
  instancia puede servir tráfico útil y `503` cuando está degradada. El cuerpo
  es agregado a propósito (`status`, `storage_ok`, `sources_available`); los
  nombres de fuentes y códigos internos siguen sólo en `GET /status`, que exige
  scope `read`.
- Los backups se escriben con `VACUUM INTO`, de modo que la copia es
  transaccionalmente consistente y ya está checkpointeada: no arrastra `-wal` y
  se restaura tal cual. La verificación abre la copia en solo lectura. `db
  backups` lista los tres formatos —automático, de migración y de
  pre-restauración— y la rotación sólo borra los automáticos.
- El reranker de Deep por defecto es **léxico**, no de embeddings. Sobre el
  corpus etiquetado, la similitud coseno entre feature hashes de
  `local_hashing_v1` puntúa peor que la cobertura léxica (nDCG@3 0,925 frente a
  1,000); la medición vive como test en `ranking_v2::reranker_measurement`. El
  reranker de embeddings sólo se elige con un backend de modelo real, y nunca
  con uno remoto, que enviaría el texto de cada documento candidato a un tercero.
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
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo audit
cargo deny check
cargo cyclonedx
```

Fuera de la compuerta por push, con sus propios disparadores:

```bash
# nocturno o workflow_dispatch
cargo test -p amatl-server --test soak -- --ignored

# requiere chromedriver escuchando
AMATL_BROWSER_E2E=1 cargo test -p amatl-server --test browser_e2e -- --test-threads=1

# requiere bwrap, systemd-run, Chromium y user namespaces
AMATL_CHROMIUM_INTEGRATION=1 cargo test -p amatl-core --test deep_phase5 -- --test-threads=1
```

Resultados locales registrados el 2026-08-14:

- **338 pruebas aprobadas** en el workspace, 0 fallos, 1 ignorada (soak, que
  corre en su propio job);
- soak ejecutado aparte: 160 655 peticiones, **0 errores**, p95 1,6 ms. Antes
  reportaba un 33 % de error constante —todas las peticiones MCP— que pasó
  inadvertido porque ningún workflow lo ejecutaba;
- 4 pruebas E2E de navegador contra Chrome real vía WebDriver;
- integración Chromium verificada contra el harness real: los scripts se
  ejecutan y mutan el DOM, y una página renderizada **no** alcanza un listener
  en loopback, con guarda de no vacuidad;
- `cargo doc` sin avisos sobre los cuatro crates;
- 190 pruebas aprobadas en el workspace (registro del 2026-08-13);
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
3. Aprobar la ficha de una fuente gratuita ya implementada (SearXNG o
   Marginalia): ambos `ProviderFactory` existen y sólo falta
   `reviewer`/`reviewed_at`/`approval_status` con identidad real — no es
   trabajo de código, es papeleo de gobernanza. Marginalia ya se aprobó en un
   `amatl.toml` de operador real (2026-08-15, gitignored, no forma parte del
   repositorio) usando la clave pública compartida de Marginalia mientras se
   espera una clave propia por correo; el `amatl.example.toml` que sí se
   versiona sigue con la ficha en `draft` a propósito, porque
   `reviewer`/`reviewed_at` son específicos de cada operador y no deben
   inventarse en un ejemplo. SearXNG sigue sin aprobar en ningún lado; exige
   además que el operador levante su propia instancia. Brave y Mojeek
   **quedan descartados, no pendientes**: ambos exigen plan de pago —Brave
   eliminó su tier gratuito en 2026-02—, y `config.rs` ya los fija en
   `approval_status = "rejected"` por decisión de política, con el motivo en
   `cost_model`/`operational_risk`. Ver la sección «Viabilidad y coste» de
   `docs/gobernanza-providers.md`.
4. Configurar el environment `provider-canary`, sus revisores y secretos; luego
   capturar latencia/errores reales sin incorporar credenciales al repositorio.
5. Para cada RC futura, ejecutar el workflow, validar musl/.deb/.rpm/Arch y sólo
   después crear el tag anotado; la publicación externa requiere autoridad del
   propietario.
6. Renderer conectado: el aislamiento se verifica en `chromium-isolation`, que
   además ejercita el backend desde Rust y prueba que una página renderizada no
   alcanza loopback. Falta decidir si ese workflow entra en `contract-gate`.

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

El hito anterior —pruebas browser E2E— **está cumplido, accesibilidad
incluida**: `browser_e2e.rs` ejercita Search, navegación por teclado, estado
vacío y viewport estrecho contra Chrome real, con su job en `ci.yml`. La
accesibilidad automatizada también está resuelta:
`the_ui_has_no_automatically_detectable_accessibility_violations` inyecta
axe-core 4.13.0 (vendorizado, `fixtures/axe-core/`) vía `execute_script` de
WebDriver — evita instalar Node — y confirma primero que la inyección no
quedó bloqueada por la CSP `script-src 'self'` (el test falla explícitamente
si `window.axe` no se define) antes de correr `axe.run()`. No queda nada
pendiente en este bloque.

**Conectar una fuente de búsqueda real está cumplido, código y gobernanza**
(ver ADR-010, comiteado como `f46ad90`):

1. **SearXNG autohospedado** — `providers/searxng.rs` implementado y probado;
   sin credencial. Ficha aprobable.
2. **Marginalia** — `providers/marginalia.rs` deja de ser scaffold: `search()`
   real contra `api2.marginalia-search.com` (el endpoint `api.marginalia.nu`
   de la referencia original está deprecado; se verificó contra la
   documentación oficial), header `API-Key`, traducción de `site:`, manejo
   tipado de rate limit/auth/errores de servidor. Ficha aprobable.
3. **Prioridad por coste cerrada** (era el hueco conocido): `router.rs` resta
   una penalización proporcional a `estimated_cost` al score de cada
   candidato, de modo que un mal día de SearXNG ya no empuja a Brave (fuente
   de pago) a primera ronda sin control de coste.
4. **DuckDuckGo HTML retirado**, no sólo bloqueado: `providers/duckduckgo.rs`
   se eliminó junto con su entrada en `ProviderRegistry`, `config.rs` y la
   documentación de gobernanza, porque DuckDuckGo no tiene API de búsqueda web
   (sólo Instant Answer). ADR-005 queda como registro histórico; ADR-010
   documenta el cierre.
5. **Brave y Mojeek quedan descartados, decisión cerrada (2026-08-15): no se
   contratan providers de pago.** Su adapter está completo (Brave: 366 líneas,
   endpoint y parseo correctos), pero `builtin_provider_records()`
   (`config.rs`) fija ambos en `approval_status = "rejected"` por defecto, con
   el motivo en `cost_model`/`operational_risk`, y un test dedicado
   (`paid_providers_are_rejected_by_default_not_merely_draft`) impide que esto
   regrese silenciosamente a `draft`. No queda abierto a "segunda ronda con
   datos de uso": reactivarlos exige revertir la política, no juntar métricas.

Compuerta completa verificada, comiteado como `f46ad90`.

**El paso de gobernanza que quedaba pendiente ya se resolvió**, pero en el
`amatl.toml` real del operador, no en el `amatl.example.toml` versionado (que
sigue en `draft` a propósito — `reviewer`/`reviewed_at` son específicos de
cada operador y no deben inventarse en un ejemplo público). SearXNG y
Marginalia están `approval_status = "approved"`, `reviewer = "Alexis
Hernandez"`, `reviewed_at = "2026-08-15"` en la configuración real, y desde
`e9b0c9e` el binario corre con ambas fuentes activas contra tráfico real,
más "Resumen con IA" grounded sobre esos resultados (ver la actualización
2026-08-16 más arriba y ADR-011). Motores upstream de SearXNG curados a nivel
de operación (Docker, fuera de este repositorio) tras detectar bloqueo por
volumen en Brave/DuckDuckGo/Google CSE/Startpage; Bing confirmado con
resultados reales. Contexto de coste y viabilidad de cada fuente: sección
«Viabilidad y coste» de `docs/gobernanza-providers.md`.

**Próximo hito real: primera ronda de pruebas en ambiente real** (no
`--mock`), iniciada el 2026-08-16 con el binario `target/release/amatl`
construido desde `e9b0c9e`, corriendo en segundo plano (`nohup`) contra
SearXNG autohospedado, Marginalia y DeepInfra reales — no queda nada de
código bloqueando este paso. Persistencia y hallazgos de esa ronda son la
próxima entrada a registrar aquí, no una fase nueva.

En paralelo, los controles externos de GitHub, gobernanza/credenciales del
environment y canario real continúan como decisiones del propietario. No
bloquear el pulido local por falta de APIs ni simular su aprobación; futuras
publicaciones deben repetir las compuertas documentadas.
