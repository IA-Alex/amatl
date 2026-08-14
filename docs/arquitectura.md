# Arquitectura de AMATL

AMATL es un buscador generalista multi-fuente, Linux-first, con flujo visible
`buscar → revisar → abrir`. Un único núcleo funcional sirve a CLI, UI, API y
MCP; ninguna superficie replica reglas de negocio.

## Mapa del workspace

| Área del plan | Implementación | Responsabilidad |
|---|---|---|
| core/model | `amatl-core/src/model.rs`, `lib.rs` | Entidades y valores canónicos; `schema_version = "1"` |
| query/classify/planning | `query.rs`, `classify.rs`, `planning.rs` | Intención, heurística léxica y snapshot de estrategia |
| providers/router | `providers/`, `providers.rs`, `providers/registry.rs`, `router.rs` | Adapters, registro extensible, capabilities, disponibilidad y recomendación |
| execution/budget | `execution.rs`, `budget.rs`, `progressive.rs` | Orquestación, deadline, rondas, concurrencia y consumo |
| normalize/canonical/dedupe | `normalize.rs`, `canonical.rs`, `dedupe.rs` | Modelo común, identidad URL y consolidación conservadora |
| ranking/diversity | `ranking.rs`, `diversity.rs` | Ranking MVP explicable y límites visibles |
| deep/fetch/render/extract | `deep.rs`, `fetch.rs`, `render.rs`, `extract.rs` | Enriquecimiento opcional y capacidades aisladas |
| ingestión local | `ingest.rs` | lectura acotada, detección de tipo, despacho y conversión a `Document` |
| evidence/ranking v2/gaps | `evidence.rs`, `ranking_v2.rs`, `gaps.rs` | Señales Deep, gate de calidad y expansión acotada |
| cache/storage/telemetry | `cache.rs`, `document_cache.rs`, `storage.rs`, `telemetry.rs` | Estado opcional y tolerante a fallos |
| inferencia | `inference.rs` | Contrato de embeddings/reranker y backend local offline para Ranking v2 |
| errores | `errors.rs` | Catálogo único de códigos, estado HTTP y mensaje para todas las superficies |
| security/data policy | `config.rs`, `service.rs`, `security.rs` y middleware de `amatl-server` | Egress/inferencia, SSRF, exposición y hardening HTTP |
| superficies | `amatl-cli`, `amatl-ui`, `amatl-server` | Entrada/salida y transporte, sin lógica duplicada |

El core no contiene marcadores de superficie: la implementación de transporte
vive en `amatl-server` y consume `AmatlService`.

## Extensión de providers

Una fuente se declara en configuración (`[providers.<nombre>]`, el expediente de
gobernanza) y se implementa con un `ProviderFactory` registrado en
`ProviderRegistry`. `AmatlService::with_registry` acepta un registro propio, de
modo que añadir o retirar fuentes no requiere tocar `service.rs` ni añadir
campos a `ProviderConfig`. La configuración declara; el registro implementa; el
servicio falla con `provider_not_declared` o `provider_not_registered` si ambos
lados no coinciden.

## Inferencia

`data_policy.inference` expresa el permiso y `[inference]` dimensiona el
backend. `local_only` resuelve al backend offline `local_hashing_v1`
(embeddings por hashing con signo, determinista, sin red ni ficheros de modelo)
que alimenta `SemanticScorer` y `DeepReranker` de Ranking v2. `remote_explicit`
falla cerrado —AMATL no incluye backend remoto— y `disabled` deja el ranking
puramente léxico. Si los pesos semánticos exigen un backend que no está
disponible, Deep degrada con `inference_unavailable` en lugar de simular la
señal.

## Ciclo de vida

```text
Query → Classification → SearchPlan → ProviderResult → NormalizedResult
→ CanonicalResult → DeduplicatedResult → SearchResult
→ Document → Evidence → Gap → SubQuery
```

Search termina en `SearchResult`, que nunca contiene cuerpo completo ni
`final_url`. Deep parte de resultados Search y produce `Document`; sólo entonces
existen contenido, URL final y señales de evidencia. `Gap` describe un déficit
observable y `SubQuery` una expansión presupuestada.

Evidence v2 añade fragmentos acotados y verificables sobre `Document.content`,
con offsets, hashes y procedencia completa. Es aditivo: Evidence v1 conserva el
score usado por Ranking v2/Gap y Evidence v2 lo proyecta sin recalibrarlo. Ver
[contrato Evidence v2](evidence-v2.md).

La ingestión local tiene un ciclo independiente y no simula un resultado de
Search: `ruta explícita → detector → extractor por tipo → Document → Evidence
v1/v2`. Sólo la CLI expone esta capacidad. API y MCP no aceptan rutas locales,
por lo que un cliente de red no puede usar AMATL como lector del filesystem.

## Ownership y ejecución

`router` ordena y recomienda providers y solicitudes de capacidad; no asigna
saldo definitivo. `planning` construye `SearchPlan`. `SearchOrchestrator` posee
por valor el único `Budget`, lo reserva al materializar cada plan y conserva el
deadline global (`execution.rs:44-155,201-245`). Los adapters sólo reciben un
plan y timeout; no pueden expandir el Budget.

`DeepOrchestrator` posee un `DeepBudget` separado que contabiliza fetches, bytes,
redirects, navegador, crawl, subqueries, coste y deadline. Search no importa ni
invoca Fetcher, Renderer o Extractor. La llamada a Deep ejecuta Search primero y
luego construye explícitamente esas capacidades (`service.rs:167-275`).

## Núcleo único de superficies

`AmatlService` recibe `Config`, abre SQLite sólo de forma opcional y expone
`search`, `deep`, `fetch_public` y `provider_summaries`, más las superficies
locales que dependen de esa persistencia opcional —`history`, `saved_documents`,
`save_document` y sus borrados— y `status`, que agrega disponibilidad de
fuentes, salud de la persistencia y efectividad de cachés sin recalcular nada.
Cuando la persistencia no está disponible esas operaciones fallan cerradas con
`storage_unavailable`; búsqueda y Deep siguen funcionando. CLI, handlers Axum y MCP
delegan en ellas; MCP no crea su propio fetcher. `ServiceSurface` selecciona
límites: CLI y API usan los configurados; MCP reduce providers, tiempos,
fetches, bytes y subqueries (`service.rs:14-58`). La UI usa exclusivamente el
contrato HTTP público y envía Search y Deep mediante POST JSON; el token sólo se
coloca en `Authorization` y no se serializa como control del formulario. Al
mostrar Deep, correlaciona Evidence v2 por `document_id`, limita documentos y
fragmentos, renderiza contenido externo sólo con APIs DOM de texto y verifica
offsets UTF-8 y SHA-256 con Web Crypto. Estas comprobaciones son presentación
defensiva: el core sigue siendo dueño del contrato y de la evidencia.

El registro de providers es un punto de extensión en caliente: `POST /reload` y
`SIGHUP` reconstruyen el servicio desde la configuración y lo intercambian sin
reiniciar el proceso, y `ProviderRegistry` admite alta y baja de factories para
un embebedor. Sobre esa construcción actúan dos compuertas de ejecución —
gobernanza (`ApprovalStatus` completo y vigente) y cortacircuitos persistente—
que sólo pueden retirar una fuente de la ronda, nunca añadirla.

La inferencia mantiene dos backends bajo el mismo contrato asíncrono
`EmbeddingBackend`: el local determinista y uno remoto gobernado que sólo existe
con `remote_explicit`, perfil `standard` y endpoint declarado. El espacio
vectorial resultante (`backend@dimensiones`) namespacea la caché documental, de
modo que cambiar de backend o de ancho invalida por construcción en vez de
reutilizar artefactos de otro espacio.

La correlación de solicitud es transversal: el borde HTTP genera el
`request_id`, `ServiceSurface` lo transporta al core y desde ahí llega a cada
llamada saliente —`ProviderContext` para providers y `FetchRequest` para el
fetch de Deep— dentro de spans que lo declaran. Nunca se envía al tercero: sólo
etiqueta la traza local.

El comando CLI `ingest` invoca `LocalIngestor` del mismo core sin pasar por el
servidor. Esta excepción de transporte es intencional: mantiene el acceso a
archivos fuera de HTTP/MCP y no crea lógica de evidencia en la CLI.

Antes de construir capacidades de red, `data_policy` resuelve el perfil
efectivo. `isolated` instala fetch/transporte denegados y la validación rechaza
egress gobernado, inferencia remota, providers, renderer o bind no-loopback.
Esto mantiene el control en core para todas las superficies. Las extensiones de
inferencia siguen siendo opcionales y deberán consultar el permiso central; no
existe un backend LLM en el workspace actual.

## Persistencia y degradación

SQLite almacena cachés y telemetría opcionales; nunca decide correctness. Si no
abre, `AmatlService` continúa sin storage. Search conserva resultados válidos de
providers parciales, separa errores y degradaciones y emite `success`,
`partial_success` o `failure`. Deep conserva documentos superficiales cuando la
extracción opcional falla. Chromium permanece fail-closed en core; su harness
Linux sin red se valida por separado antes de implementar el bridge CDP.

## Invariantes de revisión

- El core no depende de ninguna superficie.
- Search no ejecuta fetch/render/extract ni expone `final_url`.
- El orquestador es el único dueño del Budget.
- Un provider no reinterpreta texto libre ni modifica `SearchPlan`.
- Toda estructura externa/persistida usa `schema_version` cuando lo prescribe el
  contrato; SemVer, adapter/extractor version y migraciones son independientes.
- Caché, SQLite, Trafilatura, Chromium y LLM no son requisitos de Search.
- Ninguna superficie crea una salida de red por fuera de `AmatlService`; un
  backend futuro de inferencia debe respetar `data_policy`.
- `plan_amatl.md` y `fase_a_contratos.md` son normas rectoras, no archivos de
  desarrollo ordinario.
