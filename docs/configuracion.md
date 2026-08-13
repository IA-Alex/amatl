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
| `providers`, `timeouts`, `budget` | 0–1 | fuentes, deadline y cuota Search |
| `execution` | 1 | concurrencia/retry |
| `ranking_policy`, `diversity_policy` | 2 | resultado Search |
| `persistence`, `cache.provider_search`, `telemetry` | 3 | estado opcional |
| `search_policy` | 4 | rondas, cobertura y ganancia marginal |
| `deep`, `deep.extractor`, `deep.renderer`, `cache.document` | 5 | enriquecimiento |
| `deep.ranking_v2` | 6 | ranking Deep sujeto a benchmark |
| `deep.gaps` | 7 | déficits y SubQuery |
| `server` | 9 | UI/API/MCP y exposición |

## Providers

| Clave | Tipo/default | Validez y efecto |
|---|---|---|
| `providers.enabled` | `array<string>` / `[]` | Nombres reconocidos: `brave`, `mojeek`, `duckduckgo_html`; desconocidos se ignoran actualmente |
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

### Extractor

| Clave | Tipo/default | Rango validado |
|---|---:|---|
| `deep.extractor.executable` | string / `trafilatura` | sin validación de contenido; ausencia degrada |
| `.version` | string / `trafilatura-cli-v1` | sin validación; participa en caché |
| `.timeout_ms` | u64 / 8000 | >0 |
| `.max_output_bytes` | u64 / 4194304 | >0 |

### Renderer

| Clave | Tipo/default | Rango validado/estado |
|---|---:|---|
| `deep.renderer.enabled` | bool / `false` | aun en true, backend actual queda no disponible |
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
