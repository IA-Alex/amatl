Generated test artifact — SearXNG diagnostic — do not treat as project documentation.

# Hallazgos clasificados

## ROOT_CAUSE — Router

Los dos casos filtrados no alcanzaron SearXNG porque `AdaptiveRouter::recommend` exige `supports_required_filters` antes de seleccionar. SearXNG declara en falso todas las capabilities requeridas por esos filtros. No quedó provider elegible, no hubo intento y el orquestador emitió `no_available_provider` por `attempted.is_empty()`.

## ROOT_CAUSE — `no_usable_results`

La consulta general alcanzó la respuesta HTTP, la conversión UTF-8 y el parseo JSON: por eso SearXNG aparece usado y parcial, no fallido. Su `ProviderResult` tuvo estado `Partial`, lo cual demuestra `unresponsive_engines` no vacío. El resultado final fue vacío y, como no hubo degradaciones de normalización, no se descartó ningún item por el contrato URL. El conjunto de items que el adapter entregó al pipeline fue vacío. La condición exacta `results.is_empty() && !providers_partial.is_empty()` generó `no_usable_results` y `SearchStatus::Failure`.

## ROOT_CAUSE — Canary

El canary reutiliza una búsqueda aislada y falla explícitamente cuando la respuesta tiene `SearchStatus::Failure`. La búsqueda equivalente fue `Failure` por `no_usable_results`; SearXNG sí estaba en `providers_used`. Por tanto, ésa es la causa demostrada del canary FAIL.

## OBSERVATION / MEASUREMENT

- `rust async`: 816 ms publicados, SearXNG usado/parcial, cero resultados y cero degradaciones.
- Los casos con filtros: 0 ms publicados, ningún provider usado y `no_available_provider`.
- No se hicieron solicitudes adicionales en esta fase.

## BLOCKED / sin determinar

No puede determinarse con la salida pública si el cuerpo de SearXNG contenía explícitamente arrays vacíos, campos ausentes que serde convirtió en vectores vacíos, ni por qué sus motores quedaron no responsivos. Tampoco se puede asignar causalidad a un motor upstream concreto.

## Prueba mínima posterior

Incorporar o habilitar una superficie diagnóstica de AMATL estrictamente de solo lectura que, para una única consulta, publique sin secretos: código HTTP, conteos de `results` y `answers` antes/después de mapping, y conteo/nombres saneados de `unresponsive_engines`. Con esa superficie, repetir sólo `rust async` una vez y correlacionar esos conteos con el `SearchResponse`.
