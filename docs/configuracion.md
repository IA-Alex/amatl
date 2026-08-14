# Configuración de AMATL

AMATL carga `amatl.toml` por defecto o la ruta de `--config-file`. Si el archivo
no existe usa todos los defaults; si existe, `#[serde(default)]` completa claves
omitidas. La configuración se valida antes de ejecutar CLI/servidor. El ejemplo
versionado es `amatl.example.toml`.

Los secretos **no** son valores TOML: sólo se configura el nombre de su variable
de entorno. Tipos enteros son no negativos salvo indicación; tamaños están en
bytes y tiempos en milisegundos/segundos según el sufijo.

## Fases y secciones

| Sección | Fase | Rol |
|---|---:|---|
| `data_policy` | transversal | perfil de confidencialidad, egress e inferencia |
| `providers`, `timeouts`, `budget` | 0–1 | fuentes, deadline y cuota Search |
| `execution` | 1 | concurrencia/retry |
| `ranking_policy`, `diversity_policy` | 2 | resultado Search |
| `persistence`, `cache.provider_search`, `telemetry` | 3 | estado opcional |
| `search_policy` | 4 | rondas, cobertura y ganancia marginal |
| `deep`, `deep.extractor`, `deep.renderer`, `cache.document` | 5 | enriquecimiento |
| `deep.ranking_v2` | 6 | ranking Deep sujeto a benchmark |
| `deep.gaps` | 7 | déficits y SubQuery |
| `server` | 9 | UI/API/MCP y exposición |

## Política de datos, egress e inferencia

| Clave | Enum/default | Validez y efecto |
|---|---|---|
| `data_policy.profile` | `standard` | `standard` o `isolated`; `isolated` niega red de forma efectiva aunque una instancia embebida omita validación |
| `data_policy.egress` | `governed` | `governed` permite las fronteras ya gobernadas; `deny` sustituye fetch/transporte por implementaciones que fallan antes de conectar |
| `data_policy.inference` | `disabled` | `disabled`, `local_only` o `remote_explicit`; expresa permiso, no instala ni activa un backend |

El perfil confidencial válido es:

```toml
[data_policy]
profile = "isolated"
egress = "deny"
inference = "local_only" # usar "disabled" si no habrá inferencia
```

`isolated` exige bind loopback, prohíbe `remote_explicit`, providers habilitados
y renderer, y cierra provider canaries, Deep/MCP fetch y transporte de providers
con `egress_denied`. Search y los mocks deterministas siguen funcionando; Deep
conserva su contrato y registra degradación cuando no puede recuperar un
documento. La extracción/ranking/evidencia locales y una inferencia local futura
siguen siendo opcionales: AMATL no contiene hoy un backend LLM ni depende de él.

`standard` + `remote_explicit` sólo declara que una integración futura podría
usar inferencia remota; esa integración deberá consultar
`allows_remote_inference()` y tener gobernanza explícita. `egress = "deny"` es
incompatible con `remote_explicit` y con cualquier provider habilitado.

La política es un control de aplicación. No sustituye firewall, namespace de
red, sandbox del extractor ni la comprobación de que el cliente MCP/LLM sea
local: un cliente cloud podría haber recibido la consulta antes de invocar
AMATL.

## Providers

| Clave | Tipo/default | Validez y efecto |
|---|---|---|
| `providers.enabled` | `array<string>` / `[]` | Cada nombre debe tener su tabla `[providers.<nombre>]` declarada; los no declarados se rechazan |
| `providers.<p>.adapter_version` | string opcional | Requerido para aprobación y clave de caché |
| `.approval_status` | enum / `draft` | `draft`, `approved`, `expired`, `rejected` |
| `.reviewed_at` | string opcional | Fecha `YYYY-MM-DD`; aprobada sólo durante 90 días inclusive |
| `.reviewer` | string opcional | Identidad real no vacía requerida para aprobación |
| `.terms_url` | string opcional | No vacío requerido; no hay validación URL en Config |
| `.terms_version_or_date` | string opcional | No vacío requerido |
| `.allowed_access_method` | string opcional | No vacío requerido |
| `.plan_or_contract` | string opcional | No vacío requerido |
| `.rate_limit` | string opcional | No vacío requerido |
| `.cost_model` | string opcional | No vacío requerido |
| `.credential_env` | string opcional | Nombre de variable; no guardar el secreto |
| `.storage_rights` | bool / `false` | `true` sólo con derecho verificado; habilita escrituras de caché |
| `.supported_regions` | array<string> / `[]` | Declaración del adapter |
| `.supported_filters` | array<string> / `[]` | Declaración usada para mapping/capabilities |
| `.data_handling_notes` | string opcional | No vacío requerido |
| `.operational_risk` | string opcional | No vacío requerido |

Defaults específicos: Brave usa `brave-v1`, `BRAVE_API_KEY`, API oficial, URL
de términos, fecha de términos `2026-02-11` y filtros site/filetype/language/
region/time_range. Mojeek usa `mojeek-v1`, `MOJEEK_API_KEY`, API oficial y URL de
soporte. Ambos siguen `draft`; DuckDuckGo queda completamente `draft` y su
adapter está bloqueado aunque aparezca en `enabled`. Ver
`docs/gobernanza-providers.md`.

`[providers]` es un mapa abierto: cada tabla `[providers.<nombre>]` declara una
fuente y se fusiona sobre los expedientes incorporados, de modo que ajustar uno
no elimina los demás. El nombre debe ser una clave estable (minúsculas ASCII,
dígitos y `_`). Declarar una fuente no la implementa: el servicio la construye
sólo si hay un `ProviderFactory` con ese mismo nombre en el `ProviderRegistry`
(ver `docs/arquitectura.md`).

## Inferencia

| Clave | Tipo/default | Validez y efecto |
|---|---|---|
| `inference.backend` | string / `local_hashing_v1` | Backend local; otro valor se rechaza |
| `inference.embedding_dimensions` | usize / `256` | Entre 32 y 4096 |
| `inference.max_documents` | usize / `64` | >0; superarlo falla el backend opcional y Deep degrada |
| `inference.max_input_chars` | usize / `20000` | >0; recorte por documento antes de embeber |
| `inference.reranker_prior_weight` | f64 / `0.5` | Entre 0 y 1; peso que el reranker conserva de la relevancia previa |
| `inference.remote_endpoint` | string / vacío | Obligatorio con `remote_explicit`; URL absoluta https, o http sólo en loopback, y sin credenciales embebidas |
| `inference.remote_model` | string / vacío | Obligatorio con `remote_explicit`; identificador enviado en el cuerpo |
| `inference.remote_credential_env` | string / vacío | Variable de entorno con el bearer; el valor nunca se escribe en configuración ni en logs |
| `inference.remote_timeout_ms` | u64 / `5000` | 100..=60000 por solicitud remota |
| `inference.remote_max_batch` | usize / `32` | 1..=256 entradas por solicitud |

Los pesos `deep.ranking_v2.policy.weight_semantic` y `weight_reranker` mayores
que cero exigen un modo de inferencia con backend disponible: `disabled` se
rechaza. `local_only` usa el backend offline; `remote_explicit` exige además
perfil `standard`, `egress = "governed"`, endpoint y modelo declarados, y envía
al tercero exactamente la consulta y el texto acotado que Deep ya recuperó.
Cambiar el backend o `embedding_dimensions` cambia el espacio vectorial: la
caché documental queda namespaced por `backend@dimensiones`, de modo que las
entradas del espacio anterior dejan de coincidir en vez de reutilizarse en
silencio.

## Search, ejecución y Budget

| Clave | Tipo/default | Rango validado |
|---|---:|---|
| `timeouts.provider_ms` | u64 / `3000` | El código no impone rango; operacionalmente debe ser >0 |
| `timeouts.global_ms` | u64 / `8000` | El código no impone rango; 0 agota inmediatamente |
| `budget.max_provider_calls` | u32 / `3` | El código admite 0; 0 impide llamadas |
| `execution.global_concurrency` | usize / `4` | >0 |
| `execution.per_provider_concurrency` | usize / `1` | >0 y ≤ global |
| `execution.max_retries` | u32 / `1` | 0–2 |
| `execution.retry_jitter_ms` | u64 / `25` | 0–1000 |

### `ranking_policy` (contrato `v1`)

| Clave | Default | Rango |
|---|---:|---|
| `version` | `"v1"` | exactamente `v1` |
| `rrf_k` | 60 | >0 |
| `weight_rrf` | 0.35 | 0–1 |
| `weight_title_match` | 0.30 | 0–1 |
| `weight_snippet_match` | 0.15 | 0–1 |
| `weight_freshness` | 0.10 | 0–1 |
| `weight_provider_agreement` | 0.10 | 0–1 |
| `freshness_half_life_days` | 30 | >0 |
| `freshness_unknown` | 0.0 | 0–1 |

Los cinco pesos `weight_*` suman exactamente 1 con tolerancia `1e-12`.

### `diversity_policy` (contrato `v1`)

| Clave | Default | Rango |
|---|---:|---|
| `version` | `"v1"` | exactamente `v1` |
| `max_visible_per_domain` | 2 | >0 |
| `max_visible_per_provider` | 5 | >0 |
| `max_visible_per_result_type` | 6 | >0 |
| `relevance_override_ratio` | 1.15 | finito y ≥1 |

### `search_policy` (contrato `v1`)

| Clave | Default | Rango/relación |
|---|---:|---|
| `version` | `"v1"` | exactamente `v1` |
| `first_round_min_providers` | 2 | >0 |
| `first_round_max_providers` | 3 | ≥ mínimo |
| `minimum_useful_results` | 8 | >0 |
| `target_useful_results` | 12 | ≥ mínimo |
| `minimum_unique_domains` | 4 | >0 |
| `target_unique_domains` | 6 | ≥ mínimo |
| `low_diversity_domain_ratio` | 0.50 | finito, 0–1 |
| `low_diversity_provider_ratio` | 0.20 | finito, 0–1 |
| `low_diversity_result_type_ratio` | 0.20 | finito, 0–1 |
| `minimum_marginal_gain` | 0.15 | finito, 0–1 |
| `minimum_expected_marginal_gain` | 0.15 | finito, 0–1 |
| `minimum_remaining_deadline_ms` | 750 | >0 |
| `maximum_results_per_domain` | 2 | >0; debe igualar Diversity |
| `maximum_results_per_provider` | 5 | >0; debe igualar Diversity |
| `maximum_results_per_result_type` | 6 | >0; debe igualar Diversity |
| `minimum_exploration_ratio` | 0.10 | finito, 0–1 |

## Persistencia, caché y telemetría

| Clave | Tipo/default | Rango/relación |
|---|---:|---|
| `persistence.enabled` | bool / `false` | habilita intento SQLite, no correctness |
| `persistence.path` | string / `amatl.sqlite3` | no se valida vacío ni permisos hasta abrir |
| `persistence.history_enabled` | bool / `true` | sólo aplica si `persistence.enabled`; registra cada búsqueda ejecutada en SQLite local |
| `persistence.saved_document_max_bytes` | u64 / 1048576 | 1..=16777216; límite del payload aceptado por `POST /saved` |
| `circuit_breaker.enabled` | bool / `true` | si false, una fuente en fallo se sigue llamando en cada búsqueda |
| `circuit_breaker.failure_threshold` | u32 / 3 | 1..=100 fallos consecutivos abren el circuito |
| `circuit_breaker.open_seconds` | u64 / 60 | 1..=3600; al expirar se permite una sonda (`half_open`) |
| `cache.provider_search.enabled` | bool / `false` | requiere persistence si true |
| `.ttl_seconds` | u64 / 300 | >0 |
| `.max_entries` | u64 / 10000 | >0 |
| `.max_bytes` | u64 / 268435456 | >0 |
| `cache.document.enabled` | bool / `false` | requiere persistence si true |
| `.ttl_seconds` | u64 / 86400 | >0 |
| `.max_entries` | u64 / 1000 | >0 |
| `.max_bytes` | u64 / 268435456 | >0 |
| `.store_content` | bool / `false` | si false elimina `Document.content` antes de persistir |
| `telemetry.persistence_enabled` | bool / `false` | requiere persistence si true |
| `telemetry.retention_days` | u32 / 30 | exactamente 30 en v1 |

Los límites positivos se validan incluso con la capability deshabilitada.

## Deep

| Clave | Tipo/default | Rango validado |
|---|---:|---|
| `deep.top_k` | u32 / 5 | >0 |
| `deep.max_fetches` | u32 / 10 | >0 |
| `deep.max_bytes` | u64 / 20971520 | >0 |
| `deep.max_redirects` | u32 / 5 | cualquier u32; 0 prohíbe redirects |
| `deep.max_crawl_urls` | u32 / 10 | >0 |
| `deep.max_depth` | u8 / 1 | 0–2 |
| `deep.timeout_ms` | u64 / 20000 | >0 |

Evidence v2 no introduce configuración ni una llamada de inferencia: Deep
selecciona localmente hasta ocho fragmentos de 512 bytes por documento. Estos
límites son invariantes del contrato actual, no valores ajustables en TOML.

La ingestión local tampoco añade claves TOML. Sus límites actuales son fijos:
20 MiB de archivo, 8 MiB de texto y 8 segundos para PDF. El comando deriva el
permiso del extractor PDF de `data_policy`: `isolated` o `egress = "deny"` lo
bloquean antes de crear el proceso; los tipos procesados dentro de AMATL siguen
disponibles.

### Extractor

| Clave | Tipo/default | Rango validado |
|---|---:|---|
| `deep.extractor.executable` | string / `trafilatura` | sin validación de contenido; ausencia degrada |
| `.version` | string / `trafilatura-2.2.0-cli-json-v1` | contrato fijado; participa en caché |
| `.timeout_ms` | u64 / 8000 | >0 |
| `.max_output_bytes` | u64 / 4194304 | >0 |

### Renderer

| Clave | Tipo/default | Rango validado/estado |
|---|---:|---|
| `deep.renderer.enabled` | bool / `false` | backend de core aún no activo; el harness Linux aislado se valida por separado |
| `.max_browser_calls` | u32 / 2 | >0 |
| `.timeout_ms` | u64 / 8000 | >0 |
| `.shutdown_grace_ms` | u64 / 500 | >0 |
| `.max_memory_mb` | u64 / 512 | >0 |
| `.max_redirects` | u32 / 5 | cualquier u32 |

### Ranking v2

| Clave | Default | Rango/relación |
|---|---:|---|
| `deep.ranking_v2.enabled` | `true` | bool; sólo actúa dentro de Deep |
| `.policy.version` | `"v2"` | exactamente v2 |
| `.policy.bm25_k1` | 1.2 | finito y >0 |
| `.policy.bm25_b` | 0.75 | finito, 0–1 |
| `.policy.weight_bm25` | 1.0 | 0–1 |
| `.policy.weight_semantic` | 0.0 | 0–1 |
| `.policy.weight_reranker` | 0.0 | 0–1; estos tres suman 1 |
| `.policy.weight_relevance` | 0.85 | 0–1 |
| `.policy.weight_evidence` | 0.15 | 0–1; ambos suman 1 |
| `.policy.benchmark_minimum_ndcg_delta` | 0.05 | 0–1 |
| `.policy.benchmark_minimum_ndcg` | 0.90 | 0–1 |

### Gap Analyzer y SubQuery

| Clave | Default | Rango/relación |
|---|---:|---|
| `deep.gaps.enabled` | `true` | bool |
| `.max_subqueries` | 2 | 0–2; 0 desactiva ejecución aunque `enabled` sea true |
| `.max_cost` | 2 | >0 |
| `.max_provider_calls_per_subquery` | 2 | >0 y además limitado por Budget de superficie |
| `.timeout_ms` | 5000 | >0 |
| `.policy.version` | `"v1"` | exactamente v1 |
| `.policy.minimum_documents` | 3 | >0 |
| `.policy.minimum_unique_domains` | 3 | >0 |
| `.policy.minimum_enriched_ratio` | 0.60 | finito, 0–1 |
| `.policy.minimum_average_evidence` | 0.45 | finito, 0–1 |
| `.policy.difficult_confidence_max` | 0.75 | finito, 0–1 |
| `.policy.difficult_minimum_terms` | 4 | >0 |
| `.policy.max_subqueries` | 2 | >0; el hard limit de ejecución sigue siendo 2 |

## Servidor HTTP/UI/MCP

| Clave | Tipo/default | Rango/relación |
|---|---:|---|
| `server.bind` | IP string / `127.0.0.1` | debe parsear como IPv4/IPv6; remoto exige auth + TLS |
| `server.port` | u16 / 8080 | 1–65535 |
| `server.token_env` | string / `AMATL_SERVER_TOKEN` | no vacío; el valor leído debe tener ≥32 bytes |
| `server.no_auth` | bool / false | true sólo en loopback |
| `server.allowed_hosts` | array / loopbacks | no vacío; authorities exactas, sin `*` |
| `server.allowed_origins` | array / `[]` | cada una HTTP(S), raíz, sin query/fragment; vacío deriva orígenes locales |
| `server.max_body_bytes` | usize / 65536 | 1–1048576 |
| `server.max_header_bytes` | usize / 16384 | 1–65536 |
| `server.request_timeout_ms` | u64 / 30000 | >0 |
| `server.idle_timeout_ms` | u64 / 30000 | >0 |
| `server.rate_limit_per_minute` | u32 / 60 | >0 |
| `server.max_connections` | usize / 64 | 1–10000 |
| `server.tls.cert_path` | string opcional | debe aparecer junto con key |
| `server.tls.key_path` | string opcional | debe aparecer junto con cert |

## Contrato frente a calibración

El **contrato** fija nombres, tipos, enums, forma JSON, versión de política,
invariantes y relaciones (sumas, límites duros y ownership). Cambiarlo puede
exigir migración o incrementar `schema_version`. La **calibración** ajusta dentro
de rangos válidos pesos, thresholds, timeouts, concurrencia, cuotas y tamaños con
evidencia reproducible; no cambia por sí sola `schema_version`.

No calibres para ocultar un error de diseño ni relajes SSRF/exposición remota.
Registra baseline, candidato, corpus/carga, entorno, métricas y decisión. MCP
aplica caps adicionales aunque el TOML configure valores mayores.
