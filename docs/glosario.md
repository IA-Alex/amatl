# Glosario canónico

Una ausencia se marca `—`: no existe representación en esa frontera. Los nombres
físicos de SQLite pertenecen a caché/telemetría, no a una persistencia completa
del ciclo de entidades.

| Concepto | Tipo Rust | Clave JSON | SQLite | CLI | Log/campo de contexto |
|---|---|---|---|---|---|
| Query | `Query` | `query`; internamente `raw_query`, `normalized_query` | `normalized_query` en caché de provider | argumento `query` | no se registra por defecto |
| Classification | `Classification` | `classification` sólo en contratos internos | `category` en telemetría | sólo debug interno | decisiones de routing, sin texto de consulta |
| SearchPlan | `SearchPlan` | `search_plan` sólo interno/Deep | — | sólo debug interno | `providers_considered`, `providers_selected`, `debug_reasons` |
| ProviderResult | `ProviderResult` | payload interno de adapter/cache | `provider_search_cache.payload` | resumen indirecto | `providers_used`, `providers_failed`, `providers_partial` en respuesta |
| NormalizedResult | `NormalizedResult` | estructura interna | —; la caché conserva `ProviderResult` previo al pipeline | — | degradaciones del pipeline |
| CanonicalResult | `CanonicalResult` | estructura interna | — | — | degradaciones de canonicalization |
| DeduplicatedResult | `DeduplicatedResult` | estructura interna | — | — | métricas de diversidad/routing |
| SearchResult | `SearchResult` | elemento de `results` | no se cachea como salida final | rango, título, `canonical_url` | conteos, no cuerpo completo |
| Document | `Document` | elemento de `documents` en Deep | `document_cache.payload` | URL final y status en `deep` | degradaciones Deep |
| Evidence | `Evidence` | elemento de `evidence` | dentro del DeepResponse; no tabla propia | sólo `--json` | — |
| Evidence v2 | `EvidenceV2` | elemento de `evidence_v2`, con `fragments` y `provenance` | no se persiste aparte; se deriva de Document | sólo `--json` | — |
| Gap | `Gap` | elemento de `gaps` | — | sólo `--json` | — |
| SubQuery | `SubQuery` | elemento de `subqueries` | — | sólo `--json` | stop/degradation de Deep |
| URL reportada | `OriginalUrl` | `original_url` | dentro de payloads | no se imprime separada en modo humano | — |
| URL canónica | `CanonicalUrl` | `canonical_url` | `document_cache.canonical_url` | URL abierta/mostrada en Search | — |
| URL tras fetch | `FinalUrl` | `final_url`, sólo Deep/Document/MCP fetch | dentro del payload documental | modo humano Deep | — |
| Rango | `Rank` | `rank` | dentro de payload | prefijo numérico | — |
| Score de ranking | `RankingScore` | campos de explicación internos/Deep | — | no visible por defecto | no visible en modo normal |
| Estado Search | `SearchStatus` | `status`: `success`, `partial_success`, `failure` | — | `status:` y código de proceso | — |
| Error de provider | `ProviderError` | agregado como `CompositeError` público | outcome de telemetría, sin mensaje secreto | error/degradación | provider y outcome |
| Capabilities | `ProviderCapabilities` | `capabilities` en `/providers` | — | `providers` sólo muestra disponibilidad | — |
| Versión de contrato | `SCHEMA_VERSION` / `schema_version` | `schema_version` string, actual `"1"` | payloads de caché versionados; las filas de telemetría actuales no tienen esa columna y SQLite usa `user_version = 2` | visible en JSON | — |

Nombres conceptuales obligatorios: **Normalization**, **Canonicalization** y
**Deduplication**. Los módulos físicos son `normalize`, `canonical` y `dedupe`.
No usar `Normalizer`, `Canonicalizer` ni `Deduplicator` como nombres alternativos
de componente. `RankingScore` no es “score genérico”; `SearchStatus` no es
“result state”; `FinalUrl` nunca es sinónimo de `CanonicalUrl`.
