# Continuidad de desarrollo — AMATL

> **Nota de consolidación (estado vigente de fuentes en el frontend).** Este
> documento es un historial *append-only*; las secciones anteriores a esta nota
> describen estados intermedios. Para evitar confusiones, la verdad vigente es:
>
> - **Marginalia y SearXNG** son las fuentes de búsqueda activables en la UI.
>   Marginalia está **aprobada** (`reviewed_at = "2026-09-08"`); **SearXNG
>   sigue pendiente de aprobación real** del operador.
> - Los proveedores de pago que la UI llegó a exponer como opción opt-in
>   (selector, botón "Insertar key", endpoints `/providers/<nombre>/credential`
>   y `/providers/<nombre>/enabled`) quedaron **retirados** por política del
>   operador: exigen plan de pago y el core los mantiene
>   `approval_status = "rejected"` por defecto. Las fuentes activables son
>   **Marginalia** y **SearXNG**.
> - Referencias canónicas: «Disponibilidad real y límites», «Pendientes #3»,
>   y la sección final «Marginalia — key privada operativa, ficha de aprobación
>   completada».

> **Nota de consolidación (2026-10-15 — supersede la nota anterior en lo que la
> contradiga).** Sigue siendo un historial *append-only*; los conteos de pruebas
> de las secciones con fecha son fotografías de ese momento, no el estado actual.
> La verdad vigente es:
>
> - **Los proveedores de pago ya no forman parte del producto.** Se retiraron
>   del registry, de los registros de gobernanza y de la UI: no hay selector, ni
>   botón/diálogo de key, ni claves en el catálogo de mensajes. La nota
>   de consolidación de arriba (que los describía como opt-in en la UI) quedó
>   obsoleta. Las fuentes activables son **Marginalia** y **SearXNG**.
> - **Browser E2E: 6 pruebas**, no 4 ni 5. El historial registra «4 pruebas E2E»
>   (2026-08-14) y «5/5» (fix de contraste); después se añadió
>   `hidden_attribute_collapses_the_panels_it_hides`, que verifica en navegador
>   real que el atributo `hidden` colapsa los paneles con `display:` propio. El
>   suite ahora es **fail-closed**: con `AMATL_BROWSER_E2E=1` un driver ausente
>   o desemparejado **falla**, en vez de saltarse en verde.
> - **Conteos de workspace:** `cargo test --workspace` → **680 aprobadas, 0
>   fallos** (los 338/190 de la bitácora son históricos; el 661 anterior contaba
>   los tres tests del servidor que aún asumían tres fuentes declaradas; el 680
>   añade `declaring_a_table_replaces_the_builtin_record_instead_of_merging_it`).
>
> **Nota de consolidación (2026-10-26 — supersede las notas anteriores en lo que
> las contradiga).** Sigue siendo un historial *append-only*: las secciones con
> fecha describen el estado de ese momento. La verdad vigente es:
>
> - **Los proveedores de pago se retiraron del producto de extremo a extremo**,
>   no sólo del registry: se eliminaron sus módulos de adapter, sus factories y
>   sus entradas en `builtin_provider_records()`. `ProviderRegistry::builtin()`
>   despacha exactamente **SearXNG y Marginalia**. Las notas de arriba que los
>   describen como opción opt-in de la UI (2026-10-15 y anteriores) quedaron
>   obsoletas: no hay selector, diálogo de key ni claves en el catálogo de
>   mensajes.
> - **Dos puertas cerradas, no una.** Además de no haber adapter, la
>   configuración los rechaza **por nombre**: `RETIRED_PROVIDERS` y
>   `is_retired_provider()` en `config/providers.rs`, aplicados en
>   `Config::validate()` tanto a las tablas declaradas como a la lista
>   `providers.enabled`. Un nombre retirado produce error de configuración en
>   lugar de una fuente omitida en silencio; ni siquiera una config construida a
>   mano que los liste construye algo (`UnknownProvider` en el registro). Cobertura:
>   `retired_providers_are_absent_and_refused_by_validate`,
>   `retired_provider_canary_stops_at_the_registry_door`,
>   `provider_canary_rejects_retired_sources_as_configuration_errors` y
>   `retired_sources_are_not_shipped_in_the_ui`.
> - Las únicas apariciones que sobreviven de esos nombres son como **nombres de
>   motores upstream de SearXNG** en los fixtures de `providers/searxng.rs`; no
>   son fuentes de AMATL.
> - **Smoke test en runtime (2026-10-26, verificado en puerto 18457).** Con un
>   `amatl.toml` que habilita y aprueba sólo SearXNG, `amatl serve` arranca con
>   `{"authenticated":true,…,"surfaces":["ui","rest","mcp"]}` y responde `/health`
>   200, `/ready` 200 `sources_available: 1` y `/status` `ok` listando
>   **exactamente dos fuentes** (`marginalia` `provider_disabled`, `searxng`
>   `available`), con **cero** menciones de los nombres retirados en el payload
>   (`grep -oiE '<patrones retirados>' → 0`); `/metrics` publica `amatl_sources_declared 2`
>   / `amatl_sources_available 1`. Los nombres retirados se rechazan **antes de
>   cualquier intento de red**, con `exit=1` y `configuration_invalid`:
>   `provider-canary <retirado>` (no declarado en `enabled`) → «remove it from
>   providers.enabled»; `provider-canary <retirado>` con la tabla `[providers.<retirado>]`
>   completa y aprobada declarada a mano → «delete the [providers.<retirado>] table». Los otros
>   dos rechazos son mensajes distintos a propósito: `provider_not_enabled` para
>   `marginalia` declarado y no habilitado, y `provider_not_approved` para
>   `searxng` habilitado con ficha parcial. Control positivo contra la instancia
>   viva: `provider-canary searxng rust` → `status: success`, 12 resultados reales
>   en 559 ms. Reproducido de nuevo al cerrar la sesión (mismos códigos y mismos
>   conteos); `cargo test --workspace` → 680 aprobadas, 0 fallos, `cargo clippy
>   --workspace --all-targets -- -D warnings` limpio.
> - **Trampa de configuración descubierta en ese smoke test.** La tabla
>   `[providers.<nombre>]` **reemplaza** la ficha incorporada completa (el wire
>   struct es `#[serde(default)]`), no la fusiona campo a campo: una tabla con sólo
>   `approval_status`/`reviewed_at`/`reviewer` pierde `terms_url`, `rate_limit`,
>   `cost_model`, … y la fuente cae a `provider_not_approved` sin error de arranque.
>   Se documenta en `docs/configuracion.md` (semántica) y `docs/operacion.md`
>   (comprobación con el canario) y queda fijada por
>   `declaring_a_table_replaces_the_builtin_record_instead_of_merging_it`.

## Snapshot verificable

Estado revisado el **2026-08-16** sobre la rama `consolidacion-ui-observabilidad`:

- último commit: `e9b0c9e` (`feat(answer): síntesis de respuesta citada opcional, tema claro/oscuro y refresco de marca`);
- baseline de implementación: tag `baseline-fases-0-9`, commit `51c6d34`;
- workspace: Rust 2021, MSRV 1.88, versión candidata `0.1.0-rc.1`, cuatro crates;
- fases 0–9: implementadas y verificadas por las compuertas internas; esto no
  declara cierre funcional definitivo, que queda condicionado a WP-1
  (Empirical Search Gate) sobre providers reales autorizados;
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
| `f46ad90` | SearXNG y Marginalia: adapter real, adaptador HTML retirado |
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
| `decisiones_amatl.md` | `3b260a0f34c934b61e103f71303b3c4effbebf3a0e231e2ff5574e41d0a36ee5` | **Modificado** (append-only): ADR-010 añadida, 2026-08-14 |

`plan_amatl.md` declaraba `Evidence` y `Gap` como «stub post-MVP» cuando
`evidence.rs` (527 líneas) y `gaps.rs` (513) llevaban implementados desde la
fase 5. La corrección alinea el documento con el código; la normativa original
se conserva como registro.

`decisiones_amatl.md` no es un documento protegido por ADR-001 (sólo
`plan_amatl.md` y `fase_a_contratos.md` lo son); su cambio es append-only —
ADR-010 documenta el retiro de un adaptador HTML y la implementación real de
Marginalia sin reescribir ADR-005.

**Actualización 2026-08-14 (comiteada como `f46ad90`):** cierre de la Etapa 1 de la brecha
de proveedores — `providers/marginalia.rs` deja de ser scaffold (adapter real
contra `api2.marginalia-search.com`, header `API-Key`), `router.rs` penaliza
por `estimated_cost`, y un adaptador HTML incompleto se retiró del código, del registro y
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
   bloqueaban la IP del operador por volumen de pruebas — confirmado con
   `docker logs searxng`, no con pruebas a nivel de AMATL. Se descartó
   explícitamente rotar IP/proxy por ir contra los términos de esos motores.
   La corrección fue editar
   `/etc/searxng/settings.yml` dentro del contenedor para deshabilitar los
   motores bloqueados y habilitar otros más tolerantes (Bing confirmado con
   resultados reales tras el cambio; otros motores quedaron habilitados aun
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
- Los proveedores de pago **se retiraron del producto de extremo a extremo** (ver la
  nota de consolidación 2026-10-26): no hay adapter (módulos eliminados de
  `providers/`), no están declarados en los expedientes
  incorporados, no están registrados en `ProviderRegistry` y `Config::validate()`
  los rechaza **por nombre** (`RETIRED_PROVIDERS`), así que no son una fuente
  omitida en silencio sino un error de configuración. La UI tampoco los expone:
  no hay selector, diálogo de key ni claves en el catálogo de
  mensajes. Ver «Estado actual verificable» en `docs/gobernanza-providers.md`.
- SearXNG y Marginalia tienen adapter real y completo. **Marginalia está
  aprobada** en el `amatl.toml` real del operador (`reviewed_at = "2026-09-08"`;
  aprobación intermedia 2026-08-15 con clave pública compartida). **SearXNG
  queda pendiente de aprobación real** — falta `reviewer`/`reviewed_at`/
  `approval_status` con identidad real, decisión del propietario, no del
  código. Ambos son los candidatos gratuitos activables; ningún example
  versionado lleva identidad de gobernanza inventada.
- El adaptador HTML se retiró: no hay API de búsqueda web disponible, sólo una
  Instant Answer API (que no devuelve resultados web). No queda como adapter apagado.
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
3. Gobernanza de fuentes gratuitas — **Marginalia ya está aprobada**: ficha
   `approval_status = "approved"` en el `amatl.toml` real del operador
   (gitignored, fuera del repositorio), con `reviewer = "Alexis Hernandez"` y
   `reviewed_at = "2026-09-08"` (aprobación intermedia 2026-08-15 con la clave
   pública compartida; definitiva 2026-09-08 con la clave privada propia). El
   `amatl.example.toml` versionado sigue con la ficha en `draft` a propósito,
   porque `reviewer`/`reviewed_at` son específicos de cada operador y no deben
   inventarse en un ejemplo. **SearXNG sigue sin aprobar en ningún lado**;
   exige además que el operador levante su propia instancia y complete la
   identidad de gobernanza. Los proveedores de pago
   **quedan descartados, no pendientes**: exigen plan de pago y `config.rs` ya los fija en
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
   candidato, de modo que un mal día de SearXNG ya no empuja a una fuente de
   pago a primera ronda sin control de coste.
4. **Adaptador HTML retirado**, no sólo bloqueado: el módulo del adaptador
   se eliminó junto con su entrada en `ProviderRegistry`, `config.rs` y la
   documentación de gobernanza, porque no hay API de búsqueda web disponible
   (sólo una Instant Answer API). ADR-005 queda como registro histórico; ADR-010
   documenta el cierre.
5. **Los proveedores de pago quedan descartados, decisión cerrada (2026-08-14): no se
   contratan providers de pago.** Sus adapters están completos, pero
   `builtin_provider_records()`
   (`config.rs`) los fija en `approval_status = "rejected"` por defecto, con
   el motivo en `cost_model`/`operational_risk`, y un test dedicado
   (`paid_providers_are_rejected_by_default_not_merely_draft`) impide que esto
   regrese silenciosamente a `draft`. No queda abierto a "segunda ronda con
   datos de uso": reactivarlos exige revertir la política, no juntar métricas.
   **Matiz de superficie:** el frontend se construyó para seleccionar
   fuentes de pago como opción opt-in (selector, botón "Insertar key",
   endpoints `/providers/<nombre>/credential` y `/providers/<nombre>/enabled`),
   y aparecen en `GET /providers` como declaradas (aunque `enabled=false`,
   `status=Unavailable` por el `rejected` por defecto). Es decir, **Marginalia y
   SearXNG son las fuentes activables y las de pago quedan como opcionales**: la
   UI las expone, pero sólo se vuelven usables si el operador overridea la
   política en su `amatl.toml`.

Compuerta completa verificada, comiteado como `f46ad90`.

**El paso de gobernanza que quedaba pendiente ya se resolvió**, pero en el
`amatl.toml` real del operador, no en el `amatl.example.toml` versionado (que
sigue en `draft` a propósito — `reviewer`/`reviewed_at` son específicos de
cada operador y no deben inventarse en un ejemplo público). SearXNG y
Marginalia están `approval_status = "approved"` en la configuración real. Nota
de fechas: hubo una aprobación intermedia el 2026-08-15 (con la clave pública
compartida de Marginalia) y la **aprobación definitiva con la clave privada
propia se completó el 2026-09-08** (ver la sección «Marginalia — key privada
operativa, ficha de aprobación completada» al final de este documento), con
`reviewer = "Alexis Hernandez"` y `reviewed_at = "2026-09-08"`. Desde `e9b0c9e`
el binario corre con ambas fuentes activas contra tráfico real,
más "Resumen con IA" grounded sobre esos resultados (ver la actualización
2026-08-16 más arriba y ADR-011). Motores upstream de SearXNG curados a nivel
de operación (Docker, fuera de este repositorio) tras detectar bloqueo por
volumen en varios motores; Bing confirmado con
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

## Cierre de auditoría y experimentos de relevancia (rama `fix/audit-repository-hygiene`)

Actualización 2026-08-30. `HEAD = 10e8aa6`, 14 commits por delante de
`origin/fix/audit-repository-hygiene`, árbol de trabajo limpio. Todos los gates
de código pasan en esta revisión: `cargo fmt --all -- --check`, `cargo check
--workspace --all-targets --locked`, `cargo clippy --workspace --all-targets
--all-features -- -D warnings` y `cargo test --workspace --locked` (576 pruebas
aprobadas, 0 fallos, 1 ignorada — soak, que corre en su propio job).

### Remediaciones de auditoría (AUDIT-01 … AUDIT-06)

- **AUDIT-01** — inventario de rutas OpenAPI sincronizado con el router real y
  guarda de cobertura que falla el build si falta una ruta pública en el spec.
- **AUDIT-02** — `.gitignore` ignora el estado SQLite anidado de AMATL.
- **AUDIT-03** — `.gitignore` ignora los logs de runtime.
- **AUDIT-04** — ciclo de vida de artefactos de target documentado
  (`DEVELOPMENT.md`, `docs/operacion.md`).
- **AUDIT-05** — las credenciales de Marginalia exigen verificación antes de
  usarse (`docs/security/secrets.md`, `amatl.example.toml`,
  `docs/gobernanza-providers.md`, `docs/operacion.md`).
- **AUDIT-06** — preservación de filtros de SearXNG y sincronización de
  contacto de seguridad (`providers/searxng.rs`, `CODE_OF_CONDUCT.md`,
  `SECURITY.md`).
- Remediaciones de higiene de repositorio del audit de validación
  (`config.rs`, `errors.rs`, `lib.rs`, `i18n.js`, `bench.rs`,
  `benchmark_plan_runner.py`).

### Experimentos de relevancia (STEP 4A → 4D)

Serie de experimentos de relevancia semántica, todos **advisory** y fuera del
camino de búsqueda por defecto:

- **STEP 4A** — viabilidad de embeddings locales
  (`docs/experiments/local-embedding-feasibility.md`).
- **STEP 4B** — viabilidad de backend Candle/musl de producción
  (`docs/experiments/candle-musl-feasibility.md`).
- **STEP 4C** — ruta release + latencia E2E, frontera tipada
  (`docs/experiments/candle-e2e-4c.md`).
- **STEP 4D** — integración real de Candle en `amatl-core` tras la feature
  `experimental-local-embeddings` (OFF por defecto): `CandleBackend`,
  `ModelPackage` y `SemanticEvaluator`
  (`docs/experiments/candle-integration-4d.md`,
  `STATUS = STEP_4D_REAL_CANDLE_INTEGRATION_COMPLETE`).

En el camino de búsqueda por defecto se añadieron, de forma aditiva y
determinista: roles de provider primario/expansión (`[expansion]`), métricas de
complementariedad entre providers, evaluación determinista de relevancia
(`assess_result`) y señales semánticas acotadas (`relevance_semantics.rs`) con
precedencia fija. Ninguna de ellas muta routing, ranking, telemetría ni
selección de providers; la capa semántica sólo puede *sugerir*
`PossiblyRelevant → Relevant` sin contradicción y con corroboración
independiente.

### Unificación de rutas/OpenAPI

`routes.rs` declara cada ruta pública (path + métodos) en un solo lugar y
genera tanto el registro Axum como el inventario que usa la guarda de cobertura
OpenAPI, eliminando la duplicación previa entre `lib.rs` y `tests.rs`.

### Estado previo a WP-1

Las remediaciones y experimentos anteriores describen sólo evidencia de
implementación y compuertas internas. No constituyen cierre funcional
definitivo: WP-1 (Empirical Search Gate) sigue siendo el siguiente trabajo
requerido y debe ejecutarse contra providers reales autorizados antes de una
conclusión de cierre.

Al entrar en la campaña, el estado era:

```
CODE_GATES=PASS
CONTRACT_GATES=PASS
KNOWN_INTERNAL_FUNCTIONAL_WORK_PACKAGES=NONE
EMPIRICAL_SEARCH_GATE=NOT_EXECUTED
STATUS=READY_FOR_EMPIRICAL_VALIDATION
```

### WP-1 / WP-2 / WP-3 — Empirical Search Gate (2026-08-31)

Se ejecutó WP-1 con la cohorte congelada de diez consultas y los dos providers
reales autorizados (`marginalia`, `searxng`), en un fixture que desactiva
persistencia, cachés, reintentos y cortacircuito. La primera ejecución expuso
un defecto interno de clasificación: una respuesta vacía podía etiquetarse
`success` si un peer fallaba. WP-2 lo corrigió en `execution.rs` y añadió la
regresión `empty_successful_provider_does_not_mask_a_peer_failure`; las
compuertas de código posteriores pasan (`fmt`, workspace tests y clippy).

La repetición completó las diez posiciones: `failure=10`,
`success=0`, `partial_success=0`; Marginalia devolvió `provider_rate_limit` en
las diez y SearXNG fue llamado en las diez pero no entregó resultados
utilizables. El aislamiento de llamadas se conserva (el fallo de Marginalia no
canceló SearXNG), pero normalización y deduplicación no pudieron observarse
porque no hubo candidatos. Una consulta directa controlada confirmó que la
instancia SearXNG ya devolvía cero resultados, antes del pipeline de AMATL.
La evidencia completa y los hashes reproducibles están en
`test-results/wp1-empirical-search/20260831-empirical-search-gate/README.md`.

```
CODE_GATES=PASS
CONTRACT_GATES=PASS
KNOWN_INTERNAL_FUNCTIONAL_WORK_PACKAGES=NONE
EMPIRICAL_SEARCH_GATE=BLOCKED_EXTERNAL
STATUS=BLOCKED_ON_AUTHORIZED_PROVIDER_AVAILABILITY
WP2_INTERNAL_DEFECT=FIXED_AND_REGRESSION_TESTED
WP3_FINAL=COMPLETED_WITH_EXTERNAL_BLOCK
```

AMATL no se declara funcionalmente cerrado. El bloqueo es externo: recuperar
disponibilidad de una fuente autorizada y repetir WP-1 con la misma cohorte;
no se abren nuevos work packages ni se atribuye ese bloqueo al core.

## Baseline operacional E2E real congelado (2026-08-31)

Este snapshot registra la primera ejecución real aceptada de extremo a extremo
posterior al bloqueo de disponibilidad descrito arriba. Es un baseline de
operación, no una reinterpretación de los contratos: la autoridad de
arquitectura permanece en `docs/arquitectura.md` y la de Evidence v2 en
`docs/evidence-v2.md`.

- **Baseline HEAD:** `37d157d0d1a3e65b93d69cfd1193b710c23e8a25`
  (`fix/audit-repository-hygiene`).
- **Estado operacional:** `SEARCH_REAL_OPERATIONAL=YES`,
  `DEEP_REAL_OPERATIONAL=YES`, `EVIDENCE_REAL_OPERATIONAL=YES` y
  `WEBUI_REAL_OPERATIONAL=YES`.
- **Flujos E2E validados:** `WebUI → Search → AmatlService → Search
  orchestrator → SearXNG → normalización → canonicalización → deduplicación →
  SearchResult`; y `Search → Deep → fetch → extracción → Document → Evidence
  v2 → WebUI`.

### Providers y Search real

SearXNG está habilitado y operativo. La ruta de resultados reales fue validada
usando el motor actualmente funcional **Wiby** (`SEARXNG_WORKING_ENGINE=wiby`).
Marginalia está habilitado pero en `RATE_LIMITED` (último HTTP conocido: `429`);
su estado de autenticación queda sin confirmar y no es necesario para que
Search tenga éxito. No se infiere resiliencia general: sólo queda probado el
escenario de degradación controlada en el que SearXNG permanece disponible.

La consulta real aceptada `SQLite WAL documentation` devolvió `success`, tres
resultados reales de `searxng` (no fixtures), con título, URL y procedencia.
Entre los dominios representativos verificados están `sqlite.org` y
`mjtsai.com`. El orden de esos resultados no es un invariante de release.

### Deep, Document y Evidence v2

`POST /deep` acepta `{"q":"SQLite WAL documentation"}`. A partir de los tres
resultados Search, Deep completó diez fetches y diez documentos exitosos bajo
la política de adquisición acotada vigente: `max_depth=1` y `max_fetches=10`.
La expansión observada de tres resultados a diez documentos es comportamiento
actual validado, no un defecto declarado por este baseline. Deep no soporta
IDs de `SearchResult` seleccionados ni un tope de fetch por request.

El contrato Document quedó validado con URL, URL final, contenido no vacío,
tipo de contenido, linaje de fuente, hash de documento, metadatos de fetch y
metadatos de extracción. En el caso representativo previamente validado
`https://sqlite.org/wal.html`, Evidence v2 creó evidencia con linaje de fuente,
URL fuente, referencia a Document, hash de contenido, fragmento y offsets; la
validación de offsets UTF-8 pasó.

La cadena de procedencia completa es un candidato a invariante de release:
`query → SearchResult → searxng → URL canónica → Deep fetch → URL final →
Document → Evidence v2` (`PROVENANCE_CHAIN_COMPLETE=YES`).

### Límites no bloqueantes de este baseline

Los siguientes elementos se clasifican como `KNOWN_NON_BLOCKING_LIMITATION`;
no se corrigieron ni se amplía su alcance más allá de la validación realizada:

1. Marginalia está rate-limited y su autenticación sigue sin confirmar mientras
   persista ese estado.
2. SearXNG depende operativamente de Wiby para resultados web generales.
3. El renderer Chromium opcional no estuvo disponible durante la validación.
4. La automatización de DOM vivo de la WebUI no se ejecutó en esta validación
   porque no había WebDriver instalado.
5. Deep no acepta IDs de SearchResult seleccionados ni expone un límite de
   fetch por request.
6. `max_fetches=10` adquirió diez documentos desde un conjunto Search inicial
   de tres.
7. Un lanzamiento previo duplicado del servidor dejó una línea de log
   `ServerError::Io` obsoleta; la operación del servidor vivo se validó de
   forma independiente.

```
REAL_SEARCH_VALIDATED=YES
REAL_DEEP_VALIDATED=YES
REAL_DOCUMENT_VALIDATED=YES
REAL_EVIDENCE_VALIDATED=YES
CONTROLLED_PROVIDER_DEGRADATION_PROVEN=YES
DEEP_SELECTED_RESULT_IDS_SUPPORTED=NO
PER_REQUEST_FETCH_CAP_SUPPORTED=NO
DEEP_BUDGET_EXPANSION_OBSERVED=YES
```

Próxima fase: revisar las integraciones grandes restantes sin cambiar los
límites, providers, routing, ranking, relevancia ni comportamiento Evidence
congelados en este baseline.

### Nota de continuidad — límite 2 (SearXNG/Wiby) superado (2026-09-07)

El límite 2 de la lista de arriba registra el estado del 2026-08-31: la ruta
de resultados web generales de SearXNG se validó entonces usando únicamente el
motor `wiby` (`SEARXNG_WORKING_ENGINE=wiby`). La configuración actual del
contenedor SearXNG tiene habilitados motores de propósito general adicionales
(`engine-a`, `engine-b` y otros motores de noticias/imágenes,
`startpage news`, `google news`,
`stackoverflow`, `arxiv`, `pubmed`, entre otros; ~80 engines `enabled` según
`GET /config`). La dependencia operativa de `wiby` como único motor funcional
ya no aplica. Esto es una nota de continuidad, no una reescritura del baseline:
no cambia código, providers, routing ni ranking; el ítem 2 se mantiene como
registro histórico de lo validado el 2026-08-31.

## Fix de routing — salud de telemetría ya no excluye de forma dura (2026-09-07)

Hallazgo sobre el mecanismo de combinación de providers (`router.rs`,
`AdaptiveRouter::recommend_with_roles`): el gate por salud de telemetría
(`ProviderHealth::Unavailable`, `success_rate < 0.2` sobre una ventana de
hasta `TELEMETRY_MAX_RETENTION_DAYS`) excluía al provider por completo de
`eligible` -- a diferencia de `circuit.rs`, que gobierna la misma pregunta
("¿lo llamamos ahora?") con recuperación explícita (`half_open`, cooldown de
`open_seconds`). Un provider excluido por telemetría nunca vuelve a ser
llamado, así que nunca puede generar una observación de éxito nueva que
levante su `success_rate`: se autobloquea durante toda la ventana de
retención, sin sondeo de recuperación. Ningún test cubría esa rama por
nombre; no está documentada en `fase_a_contratos.md` ni en `docs/api/`.

Con Marginalia en `rate_limit`/`429` persistente (ver baseline arriba), este
mecanismo podía dejarlo fuera de toda ronda de forma indefinida aunque
SearXNG y Marginalia estuvieran ambos `enabled` y `approved` -- no por
límite de presupuesto ni por decisión explícita del operador, sino por un
candado de scoring sin salida.

**Cambio (`crates/amatl-core/src/router.rs`):** la salud `Unavailable` deja
de excluir; pasa a penalizar el score (`health_penalty = 1.5`, contra `0.5`
de `Degraded`), de modo que un peer sano gana la primera ronda pero el
provider degradado sigue siendo elegible y puede ser re-sondeado por
`exploration_boost` cuando `exploration_due`. `excluded_providers` conserva
sus dos motivos reales (`provider_unavailable`, `required_capability_missing`);
`provider_health_unavailable` deja de emitirse. No se tocó `circuit.rs`,
`ranking.rs`, `dedupe.rs`, `complementarity.rs` ni ningún provider.

No es cambio de contrato público (`debug_reasons`/`excluded_providers` son
internos a `AdaptiveRoutingRecommendation`, no aparecen en `SearchResponse`
ni en `docs/api/openapi.yaml`) -- no requiere ADR.

Test de regresión: `r07_unavailable_health_is_penalized_not_excluded`
(`router.rs`), verifica que un provider con `success_rate = 0` sigue en
`ordered_providers` y ya no aparece en `excluded_providers`.

## Cierre de "próxima fase": Deep targets seleccionados y tope de fetch por request (2026-09-07)

Este append cierra la fase declarada al final del baseline del 2026-08-31
("revisar las integraciones grandes restantes sin cambiar límites, providers,
routing, ranking, relevancia ni Evidence"). No reescribe el baseline anterior;
lo continúa. Autoridad de arquitectura y Evidence v2 sin cambios.

### Estado de partida verificado

`amatl doctor` y `amatl providers` antes de tocar nada: `data_policy.profile =
standard`, `egress = governed`, `inference = remote_explicit`; `marginalia` y
`searxng` `available`; los proveedores de pago `provider_disabled`; SQLite
`migration=7`. Sin deriva respecto del baseline.

### Hallazgo

Los dos `KNOWN_NON_BLOCKING_LIMITATION` de mayor impacto funcional en Deep
(`DEEP_SELECTED_RESULT_IDS_SUPPORTED=NO`,
`PER_REQUEST_FETCH_CAP_SUPPORTED=NO`) ya estaban implementados en el commit
`fdfac71` ("feat(deep): support selected targets and request fetch cap"),
fechado el 2026-08-31 a las 11:36 — posterior al HEAD del baseline
(`37d157d`, 08:23). Ese commit actualizó núcleo, superficie HTTP, MCP,
OpenAPI (`docs/api/openapi.yaml`) y tests, pero no dejó entrada en
`decisiones_amatl.md` pese a tocar API pública (ADR-001).

### Trabajo de este cierre

1. **ADR-013** en `decisiones_amatl.md` — documenta el cambio de comportamiento
   ya enviado: `deep_with_fetch_cap` (el tope sólo estrecha el presupuesto de
   superficie; `0` y `> deep_max_fetches` se rechazan como `InvalidInput`) y
   `deep_selected` (Deep desde identidades de `SearchResult` sin re-ejecutar
   Search; `DeepTarget` sólo lleva `url`/`title`/`provider`, nunca contenido;
   canonicalización + dedup antes del `DeepBudget`; router/providers/ranking no
   se invocan — `RoutingRecommendation` vacía con
   `debug_reasons=["selected_search_targets"]`).
2. **Simetría CLI**: `amatl deep --max-fetches N` (delegando en
   `deep_with_fetch_cap`, un solo core). Sin flag, `amatl deep` se comporta
   exactamente como en el baseline.

No se tocó routing, ranking, relevancia, providers, límites configurados ni
comportamiento Evidence. El default de `/deep` y `amatl deep` es idéntico al
baseline. WP-1 no se reabrió.

### Verificación

- Compuertas: `cargo fmt --check` PASS, `cargo clippy --workspace -- -D
  warnings` PASS, `cargo test --workspace` PASS (incl. `deep_phase5.rs`,
  `amatl-server` tests de `max_fetches` 0/11 y `targets`).
- Smoke real CLI: `amatl deep "SQLite WAL documentation" --max-fetches 2`
  adquirió exactamente 2 documentos (`Enriched`) y reportó
  `Deep resource budget exhausted (fetch_limit)` — vs. 10 en el baseline.
  `amatl deep "rust" --max-fetches 999` (por encima del configurado 10)
  rechazado como `invalid_request`.

```
DEEP_SELECTED_RESULT_IDS_SUPPORTED=YES
PER_REQUEST_FETCH_CAP_SUPPORTED=YES
PER_REQUEST_FETCH_CAP_CAN_ONLY_NARROW=YES
DEEP_DEFAULT_BEHAVIOR_UNCHANGED_FROM_BASELINE=YES
CLI_DEEP_MAX_FETCHES_EXPOSED=YES
ADR_013_RECORDED=YES
ROUTING_RANKING_RELEVANCE_PROVIDERS_EVIDENCE_UNCHANGED=YES
WP1_REOPENED=NO
CODE_GATES=PASS
CONTRACT_GATES=PASS
```

Límites 3, 4, 5, 6, 7 del baseline permanecen como
`KNOWN_NON_BLOCKING_LIMITATION` sin cambios.

## Cierre CI de la PR #1: tres checks en rojo resueltos (2026-09-08)

La PR #1 (`fix/audit-repository-hygiene` → `main`, 78 commits) estaba
`UNSTABLE`: `contract-gate` PASS pero `browser-e2e`, `cross-platform
(windows-latest)` e `isolated-render` en rojo. Ninguno era regresión de código
de esta sesión; los tres eran defectos preexistentes de portabilidad/entorno.
No se tocó núcleo, providers, routing, ranking, relevancia, límites ni
Evidence. No amplía alcance — no requiere ADR.

1. **`browser-e2e` — violación real de accesibilidad (WCAG 2 AA).** axe-core
   reportó `color-contrast` 4.37:1 en `.result-title` (`--accent` `#2f6fe0`
   sobre `--background` `#f5f7fa`) en el tema claro; el umbral es 4.5:1. El
   comentario del bloque de tema claro en `styles.css` ya afirmaba que todos
   los tokens despejaban 4.5:1, así que era el token el que estaba mal, no la
   regla. `--accent` del tema claro se profundiza a `#2b66d6` (4.92:1,
   mismo matiz) en los dos bloques de tema claro. El tema oscuro no se toca
   (`#4f8cff` sobre `#111315` ya despeja ~6.6:1). Verificado local con
   `AMATL_BROWSER_E2E=1 cargo test -p amatl-server --test browser_e2e`: 5/5.

2. **`cross-platform (windows-latest)` — tres fallos de `amatl-cli` test
   `cli.rs`.**
   - `deep_command_is_exposed_without_running_network_on_help` afirmaba
     `contains("Usage: amatl deep")`; en Windows clap imprime
     `Usage: amatl.exe deep`. Ahora afirma `contains("deep [OPTIONS]
     <QUERY>")`, independiente del nombre del binario.
   - `history_and_saved_commands_manage_local_domain_state` y
     `db_maintenance_reports_health_and_rolls_the_schema_back` fallaban en
     `search --mock` porque `persistent_config()` interpolaba
     `database.display()` sin escapar en una cadena básica TOML; una ruta
     Windows (`C:\Users\...`) rompe el parseo (`\U`, `\a`). Ahora escapa `\`
     antes de interpolar, igual que el `{:?}` que ya usa otro helper del
     archivo. Verificado local: 18/18.

3. **`isolated-render` — flake de entorno en `systemd-run --user`.** El script
   `packaging/amatl-chromium-sandbox` envuelve Chromium en `systemd-run
   --user`, que necesita manager y bus de sesión por-usuario. Los runners de
   GitHub no lo levantan para el usuario del job, de ahí los "Failed to
   connect to the bus" y los verdes/rojos alternos el mismo día. El workflow
   `chromium-isolation.yml` gana un paso previo: `loginctl enable-linger`,
   espera a `/run/user/$uid/bus`, exporta `XDG_RUNTIME_DIR` /
   `DBUS_SESSION_BUS_ADDRESS` al resto del job y prueba
   `systemd-run --user -- true`. El contrato de aislamiento (namespaces,
   read-only, límites) no cambia. Render local con el script: exit 0,
   `>rendered<`.

Compuertas locales: `cargo fmt --check` PASS, `cargo clippy --workspace
--all-targets -- -D warnings` PASS, `cargo test --workspace --locked` PASS.

```
BROWSER_E2E_A11Y_FIXED=YES
LIGHT_ACCENT_CONTRAST_AA=YES (4.92:1)
DARK_THEME_UNCHANGED=YES
WINDOWS_CLI_TESTS_FIXED=YES (18/18)
CHROMIUM_ISOLATION_USERBUS_BOOTSTRAP=YES
ISOLATION_CONTRACT_UNCHANGED=YES
CORE_PROVIDERS_ROUTING_RANKING_RELEVANCE_EVIDENCE_UNCHANGED=YES
CODE_GATES=PASS
```

Pendiente tras esto: re-ejecutar la CI de la PR #1 y confirmar los tres checks
en verde; decisión del propietario sobre si la PR de 78 commits se mergea
entera (re-titulando/re-describiendo) o se parte. Los `KNOWN_NON_BLOCKING`
y los ítems de «Pendientes que requieren decisión externa» siguen sin cambios.

## Corrección — ef90b5e descansaba en una premisa falsa (2026-09-08)

`ef90b5e` gateó `dropping_the_service_releases_the_exclusive_database_lock`
a `#[cfg(unix)]` afirmando que `PRAGMA locking_mode = EXCLUSIVE` de SQLite
protege en Windows. Es falso: `SqliteLockingMode` (`config.rs`) documenta
explícitamente que el modo `Exclusive` de AMATL es un `flock` advisorio
propio sobre un `.lock` sibling, no ese PRAGMA -- que además está
deshabilitado a propósito porque bloquearía lectores legítimos
(`amatl db health`, `sqlite3` CLI) e es incompatible con WAL. Gatear el
test dejaba Windows sin ninguna protección real bajo `locking_mode =
"exclusive"`, sin que quedara registrado en ningún lado.

Corregido: `acquire_file_lock` (`storage.rs`) ahora escribe un byte en el
lock file antes de `try_lock_exclusive()` -- hipótesis: `LockFileEx` de
Win32 (que usa `fs2` 0.4.x en Windows) trata un rango de longitud cero
como no-operativo en algunas implementaciones. El test vuelve a correr en
todas las plataformas. No verificable localmente (sin toolchain Windows
en este entorno) -- pendiente de confirmación en `cross-platform
(windows-latest)`.

## Diagnóstico — Marginalia 429 `provider_rate_limit`: cuota externa compartida agotada, sin fix de código (2026-09-08)

**Premisa a verificar:** una llamada HTTP directa (curl, misma key) devolvía
200 mientras el cliente interno recibía 429/`provider_rate_limit` segundos
después.

**Resultado real:** la premisa NO se sostiene. Al reproducir hoy, el curl
directo también devuelve 429 — no 200. Ambos caminos están bloqueados por la
misma cuota externa.

### Evidencia (PASO 3)

`MARGINALIA_API_KEY=public` — es literalmente la clave pública compartida de
Marginalia (documentado en `amatl.toml`:
`rate_limit = "shared with all public-key users; 503 when saturated"`,
`plan_or_contract = "non-commercial, public key"`).

curl directo, mismo endpoint que arma el adapter
(`https://api2.marginalia-search.com/search?query=rust&count=20`), header
`api-key: public`:

```
HTTP/1.1 429 Too Many Requests
Server: nginx/1.26.3
API-Remaining-Daily-Capacity: 0
API-Event-Type: OverLimitBlock
Content-Length: 20    -> cuerpo: "QPM Limit Exceeded"
time_total: 0.83s
```

Tres curls espaciados 20 s, misma key:
```
07:06:19  "QPM Limit Exceeded"   [429]
07:06:40  "Daily Limit Exceeded" [429]
07:07:00  "QPM Limit Exceeded"   [429]
```

Probado con `API-Key:` (capitalizado) y `api-key:` (como lo manda el adapter):
idéntico 429 en ambos. El casing del header no cambia nada.

Adapter (`amatl search "rust" --json`, `RUST_LOG=...=trace`): dos intentos
(`max_retries = 1`), ambos `outcome":"provider_error"` (NO `timeout`),
latencias 1000 ms y 1320 ms — consistente con el ~831 ms de respuesta del
servidor + red, no con un timeout enmascarado. Error final:
`code":"provider_rate_limit"`, `message":"Marginalia: provider rate limit
exceeded"`, `recoverable":true`. `degradations":[]` — el circuit breaker NO
intervino (habría emitido `provider_circuit_open`, no `provider_rate_limit`).

### Hipótesis (PASO 4)

| # | Hipótesis | Veredicto | Evidencia |
|---|-----------|-----------|-----------|
| a | Circuit breaker local seguía abierto de una corrida anterior → el cliente no intentó la llamada real | **DESCARTADA** | `provider_circuit` en `amatl.sqlite3` tenía `marginalia` con `open_until = 1788854488` (2026-09-08 14:01:28 UTC), ~6 h en el pasado respecto a la corrida (≈14:04 UTC) → estado `HalfOpen`, `allows_call() == true`. La corrida no produjo ninguna degradación `provider_circuit_open`; los logs muestran dos llamadas HTTP reales salientes (`provider call finished`, `attempt 0` y `attempt 1`). El breaker se abrió *después* de la corrida (13→14 fallos) como consecuencia de los 429, no como causa. |
| b | El adapter arma headers/URL distintos al curl que funcionó | **DESCARTADA** | El adapter (`providers/marginalia.rs::request`) manda `GET api2.marginalia-search.com/search?query=<q>&count=20`, headers `accept: application/json`, `cache-control: no-cache`, `api-key: <key>`. Reproduje ese request exacto con curl (incluido `count=20` y el casing `api-key:`): 429 idéntico. El único delta con el "curl de prueba" original era `count=20` y dos headers benignos — ninguno cambia el resultado. |
| c | Rate limit de Marginalia es por ventana corta / diario, la key pública compartida está agotada | **CONFIRMADA** | Cuerpos `"QPM Limit Exceeded"` y `"Daily Limit Exceeded"` del propio servidor; header `API-Remaining-Daily-Capacity: 0`, `API-Event-Type: OverLimitBlock`. `MARGINALIA_API_KEY=public` = key pública compartida entre todos sus usuarios. curl directo hoy = 429, igual que el adapter. |
| d | El adapter interpreta un código que NO es rate-limit real (403, timeout, 200-con-error) como `provider_rate_limit` | **DESCARTADA** | El servidor devuelve HTTP `429` genuino con cuerpo explícito de rate-limit. `status_error` (`marginalia.rs`) mapea `429 → ProviderErrorKind::RateLimit → "provider_rate_limit"` — clasificación correcta. Los logs marcan `provider_error`, no `timeout`. No hubo 200 con cuerpo de error (el parser exige `status == 200` antes de leer JSON). |
| e | Otra causa | — | Ninguna otra apareció. SearXNG (127.0.0.1:8888, sin cuota) respondió normal en la misma corrida y la búsqueda global terminó en éxito con 12 resultados. |

### Acción (PASO 5)

**NO hay fix de código.** El adapter arma el request correctamente, clasifica
el 429 correctamente y el circuit breaker se comporta como debe. La causa es
100% externa: cuota diaria + por-minuto de la clave pública compartida de
Marginalia, agotada por el conjunto de todos sus usuarios.

Esto depende de una **key privada propia de Marginalia** (BLOQUE 1, ya
documentado en rondas anteriores). Sin ella, `marginalia` seguirá degradando
a `provider_rate_limit` de forma intermitente según el uso global de la key
pública; SearXNG cubre el hueco y la búsqueda no falla entera (verificado en
esta corrida).

No se aplica ningún workaround (retry agresivo, cache de resultados falsos,
subir umbrales del breaker para "tolerar" más 429): ninguno arregla la causa.

Gates: no se corrieron `fmt`/`clippy`/`test` porque no hubo cambio de código.
`git status` limpio en `ef16261`. El `amatl.sqlite3` local (gitignored) se
restauró a su estado pre-reproducción.

## Marginalia — key privada operativa, ficha de aprobación completada (2026-09-08)

Confirmado con evidencia real: `curl` directo a `api2.marginalia-search.com/search`
con la key privada del operador (cargada desde `~/amatl/.env`, gitignored) devuelve
**HTTP 200** con `results` no vacío y headers `API-Remaining-Daily-Capacity: 1998` /
`API-Event-Type: UnderLimit` (antes: 429 / "Daily Limit Exceeded" con la key pública
compartida). La key vive sólo en `~/amatl/.env`; nunca en TOML, Git ni logs.

Búsqueda real end-to-end vía `amatl search "rust programming language" --json`:
`providers_used = ["marginalia", "searxng"]`, `providers_failed = []`, `errors = []`,
`degradations = []`. 32 resultados: **20 de marginalia**, 12 de searxng, en 828 ms.
AMATL ahora combina ambas fuentes en producción, no sólo en teoría de
router/dedupe/ranking.

No hizo falta resetear el circuit breaker: `provider_circuit` para marginalia estaba
en `consecutive_failures = 0`, `open_until` vacío — la primera búsqueda con la key
válida lo dejó cerrado.

Ficha de gobernanza completada en `amatl.toml` (`[providers.marginalia]`, no
versionado) según `docs/gobernanza-providers.md:141-159`:
`approval_status = "approved"`, `reviewer = "Alexis Hernandez"` (convención ya usada
en el resto del archivo), `reviewed_at = "2026-09-08"`,
`plan_or_contract = "private key, individual account"`,
`rate_limit = "2000 requests/day per private key (header API-Remaining-Daily-Capacity,
verificado 2026-09-08)"` — datos reales extraídos del header, no placeholders.

Puerta de activación (`ProviderRuntimeConfig::approved_on`, `config.rs:922`):
`reviewed_at` dentro de la ventana `[day, day+90]` → vigencia hasta **2026-12-07**.
`amatl doctor` y `amatl providers` reportan `marginalia	available`.

Con esto, de los 4 pendientes originales de la sesión (Marginalia, Chromium,
WebDriver persistente, decisión de merge), sólo quedan Chromium (decisión de
operador, sin urgencia) y la decisión de merge de la PR #1 — que ya no tiene
bloqueos técnicos: todos los checks de CI están en verde.

## BA-01 — rustls 0.23.43 → 0.23.46 en el lockfile (2026-10-19)

Hallazgo BA-01 (severidad media, condicionada a TLS, `CONFIRMED` por análisis
estático). Reproducido antes de tocar nada: `cargo tree -i rustls` mostraba un
único `rustls 0.23.43` compartido por TLS entrante (`axum-server`) y saliente
(`reqwest` → `hyper-rustls`/`tokio-rustls`), con features `ring,std,tls12`, y
`cargo deny check advisories` fallaba con **RUSTSEC-2026-0285**,
**RUSTSEC-2026-0346** y **RUSTSEC-2026-0349**.

Remediación: `cargo update -p rustls` → **0.23.46**, la versión compatible más
nueva dentro de `0.23` y la primera que cierra las tres (0285 desde 0.23.45;
0346 y 0349 desde 0.23.46). El diff del lockfile es **un solo paquete**
(versión + checksum); `Cargo.toml` no cambió. **RUSTSEC-2026-0348** (Brotli:
descompresión de certificado con hasta 1 GiB antes de autenticar) no era
alcanzable: la advisory exige la feature no-default `brotli` y este grafo
compila `ring,std,tls12`.

Regresión añadida: `crates/amatl-core/tests/supply_chain_contract.rs` fija un
**piso** de versión por crate (no un pin) leyendo el `Cargo.lock` del workspace,
con los IDs de advisory anotados junto al número para que el valor se
re-derive y no se confíe. Es hermético: no necesita base RustSec ni red, corre
en `cargo test --workspace` y por eso también cubre los jobs de macOS/Windows,
que nunca ejecutan `cargo audit`. Se verificó que el test falla de verdad
—subiendo el piso a 0.23.99 el test falló nombrando el advisory y la versión
bloqueada— y se revirtió a 0.23.46.

Higiene de CI (hallazgo del mismo informe): `clippy::single_element_loop` en
`config/mod.rs` (el `for name in [...]` del test de política de providers)
ya no rompe `cargo clippy --workspace --all-targets --all-features -- -D
warnings`. El bucle de un elemento pasó a ser un helper de aserción con nombre,
reutilizable si mañana se añade otro provider de pago, en vez de un `#[allow]`
que habría silenciado el lint.

Advertencia de dependencias sin cambio, por decisión: `chacha20 0.10.1` (vía
`rand 0.10.2` → `rmcp 3.1.2`) está *yanked* (`yank_message: null`; la versión
0.10.2 se publicó el mismo día, 2026-08-27) y `cargo deny` lo reporta como
**warning**, no como vulnerabilidad: no hay advisory de RustSec y CI instala con
`--locked`, así que el build es reproducible. La política del repo
(`docs/security/supply-chain.md`) clasifica los yanked como juicio del revisor;
la corrección disponible es de una línea (`cargo update -p chacha20` → 0.10.2)
y queda para el operador. No se tocó. *(Cerrado el 2026-10-26: el lockfile quedó
en `chacha20 0.10.2`; ver «H3 a H6 del informe externo» al final de este
archivo.)*

Controles positivos del informe revisados y mantenidos sin cambios: scope
fail-closed para rutas no declaradas (`amatl-server/src/lib.rs`), validación
previa de Host/Origin/cabeceras/tamaño/rate limit/Bearer, bind remoto que exige
TLS y autenticación (`amatl-core/src/config/server.rs`), y protección SSRF con
validación DNS, bloqueo de IP privadas y pinning (`amatl-core/src/fetch.rs`).
`/metrics`, `/health`, `/ready` y los assets siguen públicos por decisión
explícita y documentada; la restricción en proxy/red es una tarea de despliegue,
no un cambio de código.


## Auditoría de dependencias tras BA-01 — `quinn-proto` cerrado y `zip` abierto y bloqueante (2026-10-19)

El paso de verificación de BA-01 («volver a ejecutar auditoría de dependencias y
la suite completa») se ejecutó de verdad, y la auditoría no estaba limpia. El
pase reportó **8 vulnerabilidades** en `Cargo.lock`: `rustls` ya no aparecía (el
bump a 0.23.46 lo cerró), pero `cargo audit` seguía fallando por
`quinn-proto 0.11.16` (5 advisories) y `zip 7.2.0` (3). El informe de BA-01 no
las mencionaba porque su paso de auditoría fue `cargo deny check advisories`, y
esa orden resuelve el grafo con **features por defecto**: lo que sólo entra por
la feature opcional `experimental-local-embeddings` no está en su grafo.
`cargo audit`, en cambio, escanea **todo** el lockfile. Verificado con las dos
órdenes sobre el mismo árbol: `cargo deny check advisories` en verde y
`cargo deny --all-features check advisories` en rojo por los mismos advisories
de `zip` (y por `paste`, que en ese grafo escala de warning a error). Las dos
herramientas no coincidían, y la que CI ejecuta
(`.github/workflows/ci.yml`: `cargo audit` y luego `cargo deny check`) es la
estricta.

`quinn-proto 0.11.16 → 0.11.19` (`cargo update -p quinn-proto`). Cierra
RUSTSEC-2026-0339, 0341 y 0342 (arregladas en 0.11.17) y RUSTSEC-2026-0338 y
0340 (0.11.18). Es dependencia **opcional** de `reqwest 0.12.28` para HTTP/3 y
este workspace nunca la habilita: `cargo tree -i quinn-proto` no imprime nada,
ni con features por defecto ni con `--all-features`, así que el cambio no puede
alterar ningún camino compilado. El diff del lockfile es un paquete (versión,
checksum y el borde `getrandom` que la versión nueva usa). Su piso (0.11.18)
entró en el guard hermético, que ahora cubre `rustls` y `quinn-proto`.

`zip 7.2.0` queda **abierto y bloqueante**, sin remediación compatible
disponible:

- Cadena: `amatl-core` → `candle-core 0.9.2` → `zip`, y `zip` es dependencia
  **obligatoria** (no opcional) de `candle-core`: no existe feature que la
  apague.
- Sólo se alcanza con la feature `experimental-local-embeddings`, apagada por
  defecto: el build y la suite por defecto no la compilan.
- Advisories: RUSTSEC-2026-0367 (7.3 high), RUSTSEC-2026-0368 (sin rating) y
  RUSTSEC-2026-0369 (9.1 critical), las tres parcheadas **sólo** en
  `zip >= 9.0.0` (verificado en la fuente del advisory:
  `patched = [">= 9.0.0"]`).
- No hay upgrade compatible: `candle-core 0.11.0` (máxima estable hoy) exige
  `zip ^8.6.0`, por debajo del arreglo, y 0.10.x sigue en `^7.2.0`. Es decir,
  toda la línea de candle está expuesta y no hay bump de lockfile que cierre el
  hallazgo.
- Alcance real: los tres advisories son de **extracción** a disco
  (`ZipArchive::extract`, `ZipStreamReader::extract`: destinos de symlinks,
  permisos Unix, bits SUID/SGID). `candle-core` sólo usa `ZipArchive::new`,
  `by_index`, `by_name` y `ZipWriter` (`src/npy.rs`), nunca `extract()`, y su
  única entrada es un paquete de modelo local provisto por el operador. Eso
  acota el impacto, pero no cierra el hallazgo: la política del repo exige ADR
  escrito para cualquier excepción.

Decisión solicitada a `@IA-Alex` (la política dice explícitamente que ninguna
excepción está pre-aprobada, así que no se aplicó ninguna):

1. **ADR de excepción** con los campos que exige `docs/security/supply-chain.md`
   —advisory, ruta afectada, explotabilidad, controles compensatorios,
   responsable, fecha de expiración y plan de retirada— más la entrada de ignore
   correspondiente. El SLA de `SECURITY.md` pediría remediación en 14 días para
   el crítico y 30 para el alto.
2. **Sacar el stack experimental del lockfile del workspace**, como ya se hizo
   con `experiments/embedding-relevance` (excluido del workspace, lockfile
   propio): `zip`, `paste`, `gemm*`, `pulp` y `tokenizers` salen de la
   superficie auditada de producción y de su SBOM. Es un cambio arquitectónico,
   no un arreglo de una línea.
3. Independiente de lo anterior: `cargo update -p chacha20` → 0.10.2 (yanked,
   vía `rand 0.10.2` → `rmcp 3.1.2`) sigue pendiente como decisión de una línea.
   *(Hecho el 2026-10-26: el lockfile quedó en `chacha20 0.10.2` con
   `cargo audit` sin advertencia de yanked y `cargo deny check` limpio; ver «H3 a
   H6 del informe externo» al final de este archivo.)*

Lo que **no** se hizo, a propósito: no se añadió ningún `ignore` a `deny.toml`
(la política lo prohíbe sin ADR aprobado); no se tocó
`.github/workflows/ci.yml`, que tiene cambios en vuelo de otro alcance, y
endurecerlo a `cargo deny --all-features check` —con `paste` escalando de
warning a error— es una decisión de política, no de este arreglo; y no se añadió
`zip` al guard hermético, porque el guard afirma pisos que el lockfile
**cumple** y afirmar 9.0.0 hoy pondría rojo `cargo test --workspace` en todos
los hosts sin remediar nada. Estado real de los gates con este conjunto de
cambios: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--all-features -- -D warnings`, `cargo test --workspace --locked` y
`cargo deny check` en verde; `cargo audit` en rojo **sólo** por los tres
hallazgos abiertos de `zip`.

*(Resuelto después, el 2026-10-25 en `def066aa` —«quarantine the candle stack out
of the workspace lockfile»—: se ejecutó la opción 2, y `zip`, `paste`, `tokenizers`,
`gemm*` y `pulp` salieron del lockfile del workspace junto con la feature
`experimental-local-embeddings` de `amatl-core`, así que `cargo audit` queda limpio
sin ADR de excepción. Hoy `grep -c 'name = "zip"' Cargo.lock` → 0.)*

## Revisión crítica externa del archivo — H1 y H2 cerrados sin cambio de contrato (2026-10-26)

Llegó un informe de revisión crítica sobre una copia archivada del repositorio
(`amatl_revision_critica.md`) con seis hallazgos. Los dos primeros tocan D2 y
ninguno es un defecto de la lógica: se cerraron en documentación y prueba, que
es donde estaba el defecto real.

**H1 — la tabla narrativa estaba mal; el código no.** El informe citaba una fila
«sólo orígenes deshabilitados (declarados)» → `/ready` `200`. Esa fila no está
en el repositorio: la única aparición de esa frase en el árbol es el propio
informe, así que la tabla errónea vivía en el informe de validación, no en el
código ni en la documentación publicada. Y contradice
`crates/amatl-core/src/service.rs`, donde la degradación se calcula con
`summary.enabled && status == Available && circuit == Closed` por fuente y
`degraded = summaries_unavailable || (storage.enabled && !storage.available) ||
!enabled_usable`; `tests.rs::ready_reports_unavailable_when_no_source_is_enabled`
ya exigía HTTP `503` desde D2. Lo que se corrigió es el enunciado, porque la
frase admite la lectura equivocada: `docs/operacion.md` publica ahora la regla
como tabla —uno o más orígenes **habilitados** con al menos uno aprobado,
disponible y con el circuito cerrado responde `200`; habilitados pero ninguno
utilizable, `503`; ningún habilitado, `503`— y dice expresamente que declarar y
deshabilitar orígenes no es una incidencia, y tampoco es capacidad.

**H2 — `sources_available` es un conteo de superficie, no el predicado de
readiness.** El handler `ready` (`crates/amatl-server/src/lib.rs`) cuenta
`ProviderSurfaceStatus::Available` sin excluir los circuitos abiertos, mientras
`service.rs` sí los excluye al decidir: un despliegue de un solo origen puede
responder `503` con `sources_available: 1`. Confirmado leyendo ambos caminos y
ahora fijado por
`ready_reports_degraded_while_counting_an_available_source_with_an_open_circuit`
(`crates/amatl-server/src/tests.rs`), que primero comprueba `200` con el mismo
despliegue sano, luego rompe el único origen con
`circuit_breaker.failure_threshold = 1` y exige `503` con el contador todavía en
`1` —la misma aserción que el núcleo ya hacía en
`status_degrades_when_the_only_enabled_source_circuit_is_open`, ahora también en
la superficie HTTP—. El rustdoc de `ready` y `docs/operacion.md` documentan la
diferencia y mandan automatizar sobre el código HTTP o sobre `status`. **No se
cambió el significado del campo**: renombrarlo o redefinirlo es un cambio de
contrato que exige revisar consumidores y versionar, justo lo que el informe pide
no mezclar con este cierre.

El rustdoc nuevo desplazó `crates/amatl-server/src/lib.rs` nueve líneas, así que
las citas `file:line` de `docs/security/threat-model.md`,
`docs/security/http-hardening.md` y `docs/security/asvs-checklist.md` que
apuntan a ese archivo se reanclaron (+9 en todo extremo ≥ 2730) y se verificaron
comparando el contenido citado antes y después del desplazamiento: mismo texto,
líneas nuevas. `tools/check_doc_refs.py` sigue en `12 known-dead, 0 new`.

## H3 a H6 del informe externo: lo que se verificó y lo que queda fuera del gate local (2026-10-26)

Los cuatro hallazgos restantes no son defectos de código y ninguno se puede cerrar
ejecutando algo en esta máquina: dos piden ejecución remota, uno autorización, y el
cuarto es una deuda de dependencias que sí se remedió.

**H5 — `chacha20` yanked: remediado.** El lockfile pasa de `0.10.1` a `0.10.2` con
`cargo update -p chacha20`, que es la corrección de una línea que la entrada
anterior de este archivo dejaba al operador. La ruta no cambia:
`chacha20 v0.10.2` ← `rand v0.10.2` ← `rmcp v3.1.2` ← `amatl-server` ←
`amatl-cli`, confirmada con `cargo tree -i chacha20 --locked`. Evidencia de cierre
con la base de avisos real: `cargo audit` descarga RustSec, carga 1352 avisos,
escanea 382 dependencias y sale `0` **sin** la advertencia de yanked; `cargo deny
check` responde `advisories ok, bans ok, licenses ok, sources ok`. No se añadió
ningún `ignore` a `deny.toml`: la clase `yanked` sigue siendo advertencia y lo que
cambió es que ya no hay ninguna entrada en esa clase.
`docs/security/supply-chain.md` deja de listar el caso como advertencia abierta.

**H3 — el gate local no acredita CI remoto: sigue abierto, y ahora está
demostrado por qué.** El commit que contiene todo esto (`365b5836`, 2026-10-25) no
es alcanzable desde ninguna punta remota verificada, así que no puede tener una
corrida de CI. La punta real de `chore/start-sh-token-bootstrap` en `origin` es
`b6820867` (2026-10-14, «fix(ui): balance mobile search actions…»),
`git merge-base --is-ancestor b6820867 HEAD` confirma que es ancestro de HEAD —de
modo que los 19 commits son locales— y `git fetch origin --dry-run` no trae nada,
así que las referencias de seguimiento locales no estaban obsoletas. Lo que **sí**
se pudo acreditar sin publicar nada, ejecutando los comandos exactos
de cada workflow con las versiones que el propio CI fija, el 2026-10-26 en esta
máquina y todo con `--locked`:

| Job de CI | Comando replicado | Resultado |
|---|---|---|
| `contract-gate` | `cargo check --workspace --benches --locked` | ok |
| `contract-gate` | `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked` | ok |
| `contract-gate` | `cargo run --locked -p amatl-cli -- benchmark ranking-v2 --json` | `passed: true`, nDCG@3 0.92 frente a 0.66 |
| `contract-gate` | `cargo cyclonedx` | 4 SBOM `*.cdx.xml` |
| `msrv` | `cargo +1.88.0 check --workspace --all-targets --locked` | ok (1.88.0 es el `rust-version` del workspace) |
| `soak` | `cargo test --locked -p amatl-server --test soak -- --ignored` | 2 passed |
| `browser-e2e` | `chromedriver --port=4444` + `AMATL_BROWSER_E2E=1 cargo test --locked -p amatl-server --test browser_e2e -- --test-threads=1` | 6 passed en Chrome real |
| `trafilatura-integration` | `trafilatura==2.2.0` en venv + `AMATL_TRAFILATURA_INTEGRATION=1 cargo test --locked -p amatl-core --test trafilatura_integration` | 1 passed |
| `chromium-isolation` | los tres pasos de shell con `packaging/amatl-chromium-sandbox` + `AMATL_CHROMIUM_INTEGRATION=1 cargo test --locked -p amatl-core --test deep_phase5 -- --test-threads=1` | 13 passed, incluido `rendered_pages_cannot_reach_the_network`; el sandbox bloquea loopback |
| `static-analysis` | `ruff check tools/ --output-format=github` (ruff 0.15.22) | `All checks passed!` |
| `static-analysis` | `python3 tools/check_doc_refs.py` | `12 known-dead, 0 new` |
| `static-analysis` | `ruff format --check tools/` (advisory) | 52 |
| `static-analysis` | `semgrep scan --config=p/python` / `p/rust` (advisory) | 1 y 51 hallazgos |
| `codeql-invariants` | `codeql pack install tools/codeql` + `codeql database create --language=rust --command="cargo build --workspace --locked"` + `codeql database analyze` + `python3 tools/codeql/check_sarif.py` (CLI 2.27.3 y `codeql/rust-all` 0.2.24, las versiones fijadas) | `no invariant violations` |

Tres matices que el informe obliga a no difuminar. Primero: replicar el comando no
es replicar la corrida —otro runner, otra caché y, sobre todo, Linux—; el sandbox
local usó Chrome 156 en lugar del `chrome-headless-shell` 152.0.7977.42 que
descarga el workflow, así que acredita el aislamiento, no el binario fijado.
Segundo: lo que sigue sin poder acreditarse aquí es la matriz `cross-platform`
(macOS y Windows, la única prueba de que el workspace compila y pasa su suite
completa fuera de Linux) y `provider-canary`, que es manual y exige credenciales
reales. Tercero: publicar el SHA y ver `contract-gate` en verde para ese SHA sigue
siendo la única forma de cerrar H3; el desglose de qué workflow corre con qué
evento quedó en `docs/release.md`.

**H4 — publicación remota: verificada la lectura, no autorizada la escritura.**
`git ls-remote` sin credenciales especiales responde: `origin` es
`https://github.com/IA-Alex/amatl.git` y su `main` está en `98ed0282`. Lo que falta
no es información sino autorización: **no se hizo push, no se creó ninguna
etiqueta y no se despachó ningún workflow**, ni se hará en este cierre sin una
instrucción explícita de `@IA-Alex` sobre destino y SHA.

**H6 — workflows con efectos distintos según evento: documentado como tabla.** La
diferencia no es cosmética y ya está escrita donde se decide una publicación
(`docs/release.md`): `contract-gate` corre en push, PR, cron nocturno y manual,
con `soak` sólo en cron o manual; `static-analysis` y `codeql-invariants` filtran
por rutas y además tienen cron semanal; `chromium-isolation` es contrato Linux;
`provider-canary` es sólo manual; `release-candidate` acepta `workflow_dispatch`,
push a `main` limitado a `packaging/**` y tags `v*-rc.*`; y
`publish-crates`/`publish-aur` sólo tags estables `v[0-9]+.[0-9]+.[0-9]+` o
despacho manual. La regla operativa que se deriva, y que este archivo hace
explícita: **una etiqueta es una publicación, no una marca** —crates.io sólo
permite yank y el AUR reemplaza el paquete de cada usuario Arch—, así que ni tags
ni despachos manuales sin aprobación específica del propietario.

**Dos hallazgos nuevos, de la clase que el informe sí señala: enunciado que
contradice implementación.** No están en el informe externo; aparecieron al
replicar el gate local y ninguno cambia contrato.

El primero es un número desfasado en un workflow. El comentario de
`.github/workflows/static-analysis.yml` justificaba el paso advisory de formato
diciendo «52 files would be rewritten today», y hoy son **53**: `365b5836`, el
mismo commit bajo revisión, añadió seis tuplas a `tools/check_doc_refs.py` en
estilo compacto cuando el archivo ya estaba en la forma canónica de `ruff format`,
de modo que el archivo dejó de estar formateado y el conteo subió en uno —el
único archivo que cambió de estado, verificado comparando el conteo en el commit
del baseline (`6948046a`, 52 de 53 archivos) con HEAD (`53` de 54)—. Se restauró
la forma canónica con `ruff format tools/check_doc_refs.py` (sólo reflow: 16
inserciones y 4 borrados, ninguna línea de lógica), con lo que el conteo vuelve a
52 y el comentario a ser cierto; `tools/check_doc_refs.py` sigue respondiendo
`12 known-dead, 0 new` con código `0` y `python3 -m compileall -q tools` sigue en
verde. El otro número del comentario —la deuda congelada, 24 archivos de 53— sí
coincidía: la tabla `per-file-ignores` de `tools/ruff.toml` tiene exactamente 24
entradas, y `ruff check tools/` responde `All checks passed!` con ruff 0.15.22,
la misma versión que instala el workflow.

El segundo es la relación entre el gauge de fuentes declaradas y el conteo de
disponibles, que no estaba escrita en ningún sitio. `amatl_sources_declared`
cuenta **todas** las fuentes declaradas, incluidas las deshabilitadas —que
reportan `unavailable` con `provider_disabled`—, mientras `sources_available` de
`/ready` y `amatl_sources_available` de `/metrics` cuentan sólo las de superficie
`available` —en un despliegue real, `sources_available` de `/ready` y
`amatl_sources_available` de `/metrics` son la misma cantidad en dos superficies—.
Por tanto `declared − available` vale dos en un despliegue de un solo origen por
pura configuración: alertar sobre esa diferencia avisa de una decisión del
operador, no de una avería. `docs/operacion.md` lo dice ahora junto a la regla de
readiness y
`source_gauges_count_declared_sources_while_readiness_counts_available_ones`
(`crates/amatl-server/src/tests.rs`) fija los tres números —3 declaradas, 1
disponible, 0 con circuito abierto— contra un solo despliegue, de modo que ningún
gauge pueda derivar al significado del otro.

Al medirlo apareció una segunda divergencia, esta vez *entre* superficies y sólo
en `amatl serve --mock`: `/ready` informa las fuentes simuladas con las que el
servicio busca —`200` con `sources_available: 2` en el despliegue mock aislado—
mientras `/metrics` sigue describiendo las declaradas reales
(`amatl_sources_declared 3`, `amatl_sources_available 0`), así que un despliegue
mock sano publica `amatl_sources_available 0` y una alerta de caída sobre ese gauge
dispararía siempre. Es la misma forma que D2 quitó de `/status` —una superficie
anunciando que nada puede servir mientras `/search` responde— y por eso queda
**fijada, no arreglada**, por
`mock_deployments_disagree_between_ready_and_the_source_gauges`: hacer que
`/metrics` sea consciente del modo mock cambia el valor de una métrica publicada,
y eso es una decisión del propietario con su propia revisión de consumidores.

## Revisión de seguridad del 2026-10-26 — entornos hijos, rutas de programa y las dos intermitencias (`9afc8334`)

Esta ronda se cerró en un solo commit de código, `9afc8334` («fix(core,packaging,ui):
scrub child environments and close the spawn and deadline flakes», 32 archivos,
+1 861/−173), con su informe fechado en `docs/security/hallazgos-2026-10-26.md`
—enlazado desde `docs/README.md` y `docs/seguridad.md`— y con
`amatl_revision_critica.md`, el informe de revisión crítica del archivo que cita
la sección anterior de este historial, ya bajo control de versiones: las citas de
este documento resuelven ahora sobre un clon limpio, no sobre el árbol de trabajo.

**H1 — el hijo heredaba el entorno del servicio (Alta, remediado).** Ningún
`Command::new` llamaba a `env_clear`, y el entorno del padre es justo donde viven
`server.token_env` y el `credential_env` de cada provider: un fallo de memoria en
Trafilatura, `pdftotext` o Chromium —los tres parsean contenido hostil— era robo
silencioso de credenciales, sin señal en logs. `crates/amatl-core/src/child_process.rs`
(nuevo) concentra la política: `scrub_environment` vacía el entorno y reenvía sólo
`PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_ALL` y `TZ` —una prueba comprueba que ninguno
de esos nombres puede contener un secreto—, y lo que un sitio necesite además se
nombra con `forward_environment` (el renderer reenvía sólo `AMATL_CHROMIUM_BIN`,
`AMATL_CHROMIUM_TASKS_MAX`, `XDG_RUNTIME_DIR` y `DBUS_SESSION_BUS_ADDRESS`). Los
cuatro sitios de spawn lo aplican (`extract.rs:70`, `extract.rs:122`,
`ingest.rs:327`, `render.rs:207`) y la prueba es por camino real, no por
convención: canarios en
`extract::tests::extractor_child_does_not_inherit_server_credentials`,
`ingest::tests::pdf_extractor_child_does_not_inherit_server_credentials` y
`the_renderer_hands_the_harness_a_scrubbed_environment` (`tests/deep_phase5.rs`,
con harness de reemplazo), más un paso nuevo del workflow `chromium-isolation`
que sustituye Chromium por un volcador de `env` y exige que el canario no
aparezca y que `HOME=/tmp/home` sí.

**H4 — el harness re-heredaba dentro del sandbox (Media, remediado).** El harness
pasa ahora `--clearenv` a `bwrap` antes de aplicar sus propios `--setenv`, así que
el vaciado no depende de que el padre lo haya hecho bien.

**H2 — rutas relativas de programa (Media, remediado).** Una ruta *relativa* con
separador (`./trafilatura`, `bin/extract`) la resuelve `Command::new` contra el
directorio de trabajo, de modo que quien pudiera escribir ahí elegía el binario
que ejecuta el servicio. `is_relative_program_path` rechaza esa forma en
`deep.extractor.executable` y —sólo mientras el renderer está habilitado, y se
revalida al habilitarlo, para que el orden de configuración no salte la
comprobación— en `deep.renderer.sandbox_path`: sólo se aceptan ruta absoluta o
nombre simple.

**H3 — `Debug` derivado de `HttpRequest` (Media, remediado).** El struct ya no
deriva `Debug`: su impl manual imprime la URL saneada y sólo los *nombres* de las
cabeceras, de modo que el `tracing::warn!(?request, …)` obvio no puede publicar la
clave que algunos providers llevan en la query ni el valor de `Authorization`.

**C2a y C2b — las dos intermitencias (Baja, remediadas y medidas).** C2a era
`ETXTBSY`: `execve` rechaza arrancar un fichero que todavía tiene un escritor y la
prueba escribe el script que luego ejecuta, así que un `fork` concurrente de otra
prueba puede mantener el descriptor abierto; `spawn_with_busy_retry` reintenta
exactamente ese `ExecutableFileBusy` (siete duplicaciones desde 1 ms, 127 ms en el
peor caso) en los sitios de spawn, en vez de que la prueba duerma a ciegas. C2b
era un presupuesto de 1 000 ms con reserva de producción de 750 ms
(`minimum_remaining_deadline_ms`): un hipo de planificación metía la ronda en
`DeadlineNear`, añadía la degradación `deadline_near` y bajaba el estado a
`PartialSuccess`; los presupuestos de esas pruebas suben a 8 000 ms (el
`timeouts.global_ms` de producción) en `execution.rs` y en los arneses de
`operational.rs`. **Ningún presupuesto de producción cambió.** Medición de cierre:
`cargo test -p amatl-core -- --test-threads=16` ×12 con 24 procesos `yes` en 24
núcleos → 12/12 `cargo_exit=0`, 5 976 pruebas, 0 fallos, sin `DeadlineNear`; y el
control crudo sin reintento falló 54 de 4 000 intentos (1,35 %) en la sonda
temporal, borrada al cerrar el hallazgo —las citas del informe que apuntaban a
ella se reescribieron.


**C3 y la documentación de los controles.** `docs/security/secrets.md` gana la
sección «Procesos hijos»; `asvs-checklist.md` pasa 13.3.1 a cubrir *todo* parser
lanzado y 13.4.1 a registrar `--clearenv`; `http-hardening.md` declara el coste de
no confiar en `X-Forwarded-For` en vez de dejar un proxy inverso de confianza como
pendiente; `chromium-isolation.md` documenta la sonda de entorno;
`docs/configuracion.md` precisa la forma aceptada de las dos rutas; e
`identidad-visual.md` registra el límite de control de WCAG 1.4.11. Las citas
`file:line` que la inserción en `providers/http.rs` había desplazado se
re-anclaron, y `python3 tools/check_doc_refs.py` sigue respondiendo
`12 known-dead, 0 new` con código `0`.

**Dos defectos de UI cerrados en el mismo commit.** El primero es un
`ReferenceError`: `reportFailure` leía un identificador `message` que `ac11624a` ya
había eliminado, así que cualquier fallo que llegara a la rama del `504` lanzaba
`ReferenceError: message is not defined` en vez de informar del timeout —el
operador no veía estado alguno—, y nada lo detectaba porque las pruebas de assets
comprueban presencia de cadenas, no semántica de JS, y el CI no trae linter de JS;
`report_failure_reads_only_error_properties_never_a_bare_message` parsea ahora el
cuerpo de la función y rechaza una `message` desnuda. El segundo es de
accesibilidad: `--border` es un divisor (~1,26:1 sobre `--surface`, por debajo del
3:1 que WCAG 2.2 AA (1.4.11) exige al límite de un control), así que los campos de
texto, los `select` y el selector de idioma usan `--border-control`, alias
semántico del `--muted` ya existente (5,20:1 oscuro, 5,60:1 claro) y no un color
nuevo; en el mismo paso `aria-busy` pasa del input al panel `#results` que sí
cambia, el formulario es `novalidate` para que se vean los mensajes del catálogo,
el `lang` estático coincide con el locale por defecto del catálogo y se corrige
antes del primer pintado, y el campo de idioma de resultados gana `datalist` y
`aria-describedby`. El selector de idioma de la interfaz (`#ui-language`,
English/Español) es una elección explícita y persistida (`localStorage`),
resuelta como elección guardada → preferencia del navegador → defecto del
catálogo, y conmuta sin recargar y sin perder resultados.

**Estado verificado de este commit, en esta máquina y con `--locked`.** Los
comandos del gate local, ejecutados sobre el árbol de `9afc8334`: `cargo fmt --all
-- --check` en verde; `cargo clippy --workspace --all-targets --all-features
--locked -- -D warnings` sin avisos; `cargo test --workspace --locked` → **690
aprobadas, 0 fallos** (las 680 de la nota de consolidación más las diez pruebas
nuevas de esta ronda); `cargo deny check` → `advisories ok, bans ok, licenses ok,
sources ok`; y `python3 tools/check_doc_refs.py` → `12 known-dead, 0 new`.

**Lo que este cierre no puede afirmar.** No se ejecutó CI remoto: el commit no es
alcanzable desde ninguna punta remota verificada, así que `contract-gate`,
`static-analysis` y `chromium-isolation` no tienen corrida sobre este SHA, y
publicarlo —y ver esos tres jobs en verde— es la única forma de cerrarlo, lo que
exige una instrucción explícita del propietario sobre destino y SHA: **no se hizo
push, no se creó ninguna etiqueta y no se despachó ningún workflow.** Tampoco se
ejecutó `cargo publish`, ni la matriz `cross-platform` (macOS/Windows), ni
`provider-canary` (manual y con credenciales reales), que son las comprobaciones
que este entorno no puede replicar.

## Revisión de empaquetado del 2026-10-27 — el aviso de `readme` de `amatl-core`, C1 refutado y una deuda de dependencias que no existía (`167fb8dd`)

Tercera pasada sobre el informe de seguridad del 2026-10-26, centrada en el
empaquetado y hecha **sobre los tarballs ya extraídos**, no sobre listas de ficheros.
El árbol bajo revisión no se movió: `167fb8dd`. `cargo package --workspace --locked
--allow-dirty` termina en exit 0 y los cuatro tarballs (`amatl-core`, `amatl-ui`,
`amatl-server`, `amatl-cli`) se verifican compilando, sin un solo aviso.

**C4 era real y está cerrado: un `readme` que apuntaba fuera del paquete.**
`crates/amatl-core/Cargo.toml:10` declaraba `readme = "../../README.md"`, una ruta
fuera del paquete, mientras el crate embarca su propio
`crates/amatl-core/README.md`. Cargo resuelve esa colisión a favor del fichero local
y **reescribe el campo** en el manifiesto publicado, así que la declaración del
fuente describía un paquete que nunca se publicó, y cada `cargo package -p
amatl-core` —incluido el `--list` que corre `publish-crates.yml`— pedía una ruta
relativa a la raíz del paquete. El campo es ahora `readme = "README.md"`, que es lo
que Cargo ya publicaba: **el tarball no cambió** (el `readme` empaquetado era y sigue
siendo `README.md`, y el README del tarball es byte a byte el del crate, 3 999 B) y el
aviso desapareció. Los otros tres crates mantienen `../../README.md`, y ahí es
correcto: no embarcan README propio y Cargo copia el de la raíz al paquete.

**C1: la premisa «el tarball no se puede construir» es falsa para `cargo package` y
cierta sólo para `cargo test` sobre el tarball.** Las cuatro citas del `include_str!`
del OpenAPI existen, pero viven en `src/tests.rs`, que `lib.rs` declara con
`#[cfg(test)] mod tests;`. La verificación de `cargo package` construye la librería y
los binarios, no los objetivos de prueba, de modo que el `include_str!` no se expande
y el empaquetado pasa: el fallo de publicación de `amatl-server` es la dependencia
`path` de `amatl-core`/`amatl-ui` —reescrita a `version = "0.1.0-rc.1"` al empaquetar
y no publicada—, una **condición de orden de release**, no un defecto de código. La
severidad baja de Alta a Baja y la parte que sí queda es deuda de empaquetado, medida
el 2026-10-27 sobre el tarball extraído en `target/package/amatl-server-0.1.0-rc.1/`:
allí `cargo test --offline --no-run` muere **en la resolución**, antes de compilar
(`no matching package named 'amatl-core' found`), así que hoy el `include_str!` no es
lo primero que rompe sino lo que romperá después, cuando las dependencias internas
estén publicadas; y que romperá es un hecho estático —`src/tests.rs` sigue llevando
`include_str!("../../../docs/api/openapi.yaml")`, `docs/` no existe dentro del paquete
y la ruta resuelve fuera de él—. Quedan dos salidas y ninguna es gratis: duplicar el
contrato dentro del crate, con riesgo de deriva, o mover el contrato canónico al
crate. La decisión sigue abierta.

**La corrección aparente de C1 tampoco sirve, y falla en silencio.** Añadir
`"../../docs/api/openapi.yaml"` al `include` de `crates/amatl-server/Cargo.toml` se
probó y se revirtió: `cargo package -p amatl-server --list --locked --allow-dirty`
sale con exit 0, **sin ningún aviso**, y el listado conserva los mismos 13 ficheros,
sin el OpenAPI. Cargo descarta en silencio las entradas que salen del directorio del
paquete, de modo que un `include` con `..` deja el manifiesto **peor** que antes
—promete un fichero que no viaja— y sin señal de que no se cumplió. Es un hallazgo
durable: la corrección evidente es la que hay que descartar primero.

**La deuda de `chacha20` que el informe daba por abierta no existía en su base.** El
informe afirmaba un `chacha20` yanked transitivo por `rand` y `rmcp`. `git show
ce172d9d:Cargo.lock` ya trae `chacha20 0.10.2`, y el cierre está documentado en
`docs/security/supply-chain.md:111-115`. Enunciado corregido en las dos secciones que
lo sostenían —la lista de deuda abierta y «Lo que no se puede afirmar»—, no borrado.

**El gate de referencias gana una entrada congelada, no una excepción tácita.** El
hallazgo C4 cita **literalmente** el aviso de Cargo, y la ruta que contiene
(`../../README.md`) es relativa a la raíz del crate, no al repositorio, así que el
resolutor de `tools/check_doc_refs.py` —que sólo conoce rutas relativas al
repositorio— no podrá resolverla nunca. Se añadió `("docs/security/hallazgos-2026-10-26.md",
"../../README.md")` a `FROZEN` con su justificación escrita al lado: la cita es la
prueba sobre la que descansa el hallazgo, de modo que se deja literal en vez de
reescribirla en algo que el gate sepa leer. El gate sigue respondiendo
`13 known-dead, 0 new` con código `0`.

**Estado verificado de este cierre, en esta máquina y con `--locked`.** `cargo fmt
--all --check` en verde; `cargo clippy --workspace --all-targets --all-features
--locked -- -D warnings` sin avisos; `cargo check --workspace --all-targets --benches
--locked` sin avisos; `cargo doc --workspace --no-deps` con `-D warnings` sin avisos;
`cargo test --workspace --locked` → **690 aprobadas, 0 fallos, 2 ignoradas** sobre 25
binarios; `cargo package --workspace --locked --allow-dirty` → exit 0 con los cuatro
tarballs verificados y **0 avisos de `readme`**; `cargo run --locked --quiet -p
amatl-cli -- benchmark ranking-v2 --json` → `"passed": true`; `cargo audit` y `cargo
deny check` → `advisories ok, bans ok, licenses ok, sources ok`; y
`python3 tools/check_doc_refs.py` → `13 known-dead, 0 new`. Los archivos tocados por
esta ronda son cinco: `CHANGELOG.md`, `CONTINUIDAD.md`, `crates/amatl-core/Cargo.toml`,
`docs/security/hallazgos-2026-10-26.md` y `tools/check_doc_refs.py`.

**Lo que este cierre no puede afirmar.** No se ejecutó `cargo publish` ni se creó
ninguna etiqueta: lo reproducido es `cargo package`, que es la fase donde aparece la
dependencia `path` sin publicar, y por tanto **el orden de publicación
`amatl-core`/`amatl-ui` antes que `amatl-server` sigue sin ejercitarse de verdad**.
No se ejecutó CI remoto sobre `167fb8dd`, así que `publish-crates` —el único job que
vería el aviso de `readme` en un flujo real— no tiene corrida que confirme la
desaparición del aviso en su salida, sólo la corrida local. Y el `include_str!` del
OpenAPI sigue sin remediar en su parte real: la decisión entre duplicar el contrato y
moverlo al crate queda abierta, como deuda de empaquetado de severidad Baja.

## 2026-11-15 — `published_at` con offset compacto (regresión de `8dfa6a06`)

**Síntoma.** Una respuesta limpia de SearXNG cuyo `publishedDate` trae el offset en
forma ISO 8601 compacta (`2024-06-11T13:20:31+02`, `-05`, `-0530`) producía
`status: "partial_success"` con una degradación `invalid_published_at` y
`published_at: null` en el resultado. Es el defecto que `8dfa6a06` decía cerrar,
parcialmente resuelto: arregló el instante naive, no el offset.

**Causa raíz.** `normalize_date` tenía dos caminos y ninguno consumía esa forma: la
vía RFC 3339 exige `±HH:MM`, y el bloque naive añadido en `8dfa6a06` no acepta
offset. El valor caía por ambos y devolvía `None`. SearXNG retransmite
`publishedDate` tal cual lo emite el motor, y el contrato público de `published_at`
no lo restringe a RFC 3339, así que el valor es legítimo.

**Reproducción (antes).** Mock HTTP local con forma de SearXNG servido a
`amatl search --json`: `partial_success` + `invalid_published_at`, con la fecha
perdida. **Después:** `success`, cero degradaciones, y
`2024-06-11T13:20:31+02` se retiene como `2024-06-11T13:20:31+02:00`. Los fixtures
previos (naive, microsegundos, nanosegundos, `Z`, bare date, `+02:00`) conservan su
salida byte a byte.

**Fix.** Un tercer intento, sólo para el caso con offset, con
`Iso8601::DEFAULT` (ya presente vía `time`): acepta `±HH`, `±HHMM` y fracciones, y
**rechaza** un instante naive, de modo que no puede adelantarse a la regla de anclaje
UTC. El offset se preserva; sólo se normaliza su render a RFC 3339. El test
`normalize::tests::published_date_accepts_offsets_without_minutes` es rojo antes del
fix y cubre la ausencia de degradación en el pipeline, no sólo la función pura.

**Estado verificado en esta máquina, con `--locked`.** `cargo fmt --all --check` en
verde; `cargo test --workspace --locked` → **726 aprobadas, 0 fallos**;
`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` sin
avisos; `cargo check --workspace --benches --locked` sin avisos; `cargo doc
--workspace --no-deps --locked` con `RUSTDOCFLAGS=-D warnings` sin avisos.

**Lo que este cierre no puede afirmar.** La verificación end-to-end se hizo contra un
mock local con forma de SearXNG, no contra una instancia viva; no se ejecutó
`cargo audit`/`cargo deny` ni el browser E2E, que no tocan esta ruta. Sigue abierto,
como riesgo separado y no corregido, que `SearXngAnswer.answer` sea
`Option<String>`: un answer promocional sin ese campo haría fallar el `Deserialize`
del cuerpo entero.

## 2026-11-15 — `SearXngAnswer` sin `answer`: descartado como bug, cubierto por test

**Lo que se sospechaba.** Que un *answer* promocional sin campo `answer` (tarjeta
de entidad que sólo trae `url`) hiciera fallar el `Deserialize` del cuerpo entero
y costara todos los resultados usables de la respuesta.

**Lo que se midió.** Falso. Con `serde 1.0.229`, una clave ausente produce `None`
en un campo `Option<T>` **sin** `#[serde(default)]`; el cuerpo se deserializa y la
respuesta sobrevive. El test que se escribió para reproducirlo lo evidenció al
fallar en `left: 2, right: 1`, no en el parseo. El `title` del answer promovido
queda en `null`, que el contrato público admite (`title` es `required` y
`string|null` en `openapi.yaml`), y `dedupe` prefiere al representante con título,
así que no degrada la fusión.

**Lo que queda.** El test `a_url_only_answer_does_not_discard_the_whole_response`
fija la garantía: si alguien añade `deny_unknown_fields` o cambia el campo a un
tipo no opcional, la regresión pasa a ser visible en vez de silenciosa.

**Estado verificado.** `cargo test --workspace --locked` → **727 aprobadas, 0
fallos**; `cargo fmt --all --check`, `cargo clippy … -- -D warnings` y `cargo doc
… -D warnings` en verde.

## 2026-11-15 — `SEARXNG_INSTANCE_URL` malformada deja de ser invisible

**Síntoma.** `SEARXNG_INSTANCE_URL=not-a-url` (o `192.168.1.50:8888`, sin
esquema) producía `error_code=provider_network`, idéntico a "el servidor no
responde". El adapter consultaba otra instancia — la de loopback — sin ninguna
señal.

**Causa raíz.** `SearXngFactory::build` hacía
`Url::parse(value).ok().unwrap_or_else(loopback)`: el mismo `.ok()` que descarta
una URL inválida descarta también el caso "variable ausente", que es un estado
soportado y documentado. Ambos colapsaban en el default sin dejar rastro.

**Fix.** La resolución se extrae a `resolve_searxng_instance_url(name,
credential)`, que distingue los dos casos y emite `tracing::warn` (target
`amatl::providers`, con el provider y el error de `url`) sólo cuando el valor
está presente y no parsea. El fallback se conserva porque
`ProviderFactory::build` devuelve `Arc<dyn Provider>`, no un `Result`, y cambiar
esa firma afectaría a todas las factories: eso es una decisión de producto, no
una reparación.

**Test.** `factory_warns_only_when_the_instance_url_is_present_but_unparseable`
captura los eventos de `tracing` con un subscriber local (sin dependencia nueva)
y verifica los cuatro casos: inválida avisa, `host:port` avisa, ausente **no**
avisa, válida **no** avisa. El primer intento de test no servía: aseveraba sólo
la URL resuelta, que es idéntica con y sin el fix, y pasaba en verde tras
revertirlo. Se reescribió contra el log, y ahora falla con `must warn: []` bajo
el revert.

**Verificado end-to-end.** Con `RUST_LOG=warn` y una URL malformada el CLI
imprime el aviso; con una URL válida, no. `cargo test --workspace --locked` →
**728 aprobadas, 0 fallos**; `fmt`, `clippy -- -D warnings` y `doc -D warnings`
en verde.

**Riesgo residual.** El fallback silencioso del caso *ausente* sigue siendo
deliberado y documentado. Llevar la URL a `SourceStatus` para que `/ready` la
muestre exigiría tocar el contrato público (`openapi.yaml` y la UI): queda como
decisión de producto, no como deuda de esta reparación.
