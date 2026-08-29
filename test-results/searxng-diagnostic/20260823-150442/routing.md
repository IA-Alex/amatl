Generated test artifact — SearXNG diagnostic — do not treat as project documentation.

# Trazabilidad: query → clasificación → capabilities → selección

## Flujo implementado

1. `AmatlService::search_inner` parsea el texto con `parse_query`, construye los providers admitidos por configuración/gobernanza/circuito con `select_providers`, y entrega ambos a `SearchOrchestrator::search` (`crates/amatl-core/src/service.rs:602-642`).
2. `SearchOrchestrator::search` clasifica mediante `classify(&query)` y forma los `ProviderDescriptor` desde `provider.capabilities()` y `provider.availability()` en `adaptive_recommendation` (`crates/amatl-core/src/execution.rs:150-171, 174-190`). La clasificación sólo aporta relevancia/puntuación tras elegibilidad; no elimina por sí misma el provider.
3. `AdaptiveRouter::recommend` descarta un provider no disponible y, antes de puntuarlo, llama a `supports_required_filters` (`crates/amatl-core/src/router.rs:56-74`).
4. La condición de elegibilidad exige simultáneamente: dominio ⇒ `site_filter`; tipo de archivo ⇒ `file_filter`; idioma ⇒ `language`; región ⇒ `region`; cualquier fecha ⇒ `time_range` (`router.rs:172-178`). Si no queda provider elegible, `first_round_providers` queda vacío.
5. `build_search_plan` conserva solamente los providers recomendados que caben en presupuesto (`crates/amatl-core/src/planning.rs:6-43`). Al no haber selección, `attempted` queda vacío.
6. Tras la ronda, `SearchOrchestrator::search` emite `no_available_provider` exactamente si `results.is_empty() && attempted.is_empty()` (`crates/amatl-core/src/execution.rs:373-384`). El texto del mensaje no identifica SearXNG como fuente fallida.

## Capabilities de SearXNG

`SearXngProvider::capabilities` declara `language=false`, `region=false`, `time_range=false`, `site_filter=false` y `file_filter=false` (`crates/amatl-core/src/providers/searxng.rs:97-112`). Su disponibilidad sólo sería `Available` cuando está habilitado y aprobado (`searxng.rs:115-129`). El baseline comprobó esa disponibilidad en la configuración aislada.

## Correlación con los dos casos previos

| Consulta previa | Filtros parseados | Capability requerida y ausente | Resultado observable |
| --- | --- | --- | --- |
| `tokio site:docs.rs` | `domains=[docs.rs]` | `site_filter=false` | `providers_used=[]`, `no_available_provider`, 0 ms |
| `rust lang:es region:MX after:2025-01-01 filetype:pdf` | idioma, región, fecha y tipo de archivo | `language=false`, `region=false`, `time_range=false`, `file_filter=false` | `providers_used=[]`, `no_available_provider`, 0 ms |

## Conclusión de router

`ROOT_CAUSE`: los filtros explícitos no cumplen `supports_required_filters` para la capability declarada por SearXNG. Es una exclusión deliberada del router previa a la solicitud HTTP, no un fallo observable de SearXNG.

La función `translated_query` sí contiene una traducción posterior: añade `site:` al texto y marca idioma/región/fecha/filetype como ignorados (`searxng.rs:158-202`). Esa etapa es inalcanzable para estos casos, porque el router exige capabilities antes de construir la solicitud.
