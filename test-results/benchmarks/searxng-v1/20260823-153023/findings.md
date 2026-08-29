Generated benchmark artifact — AMATL SearXNG Baseline v1 — do not treat as project documentation.

# Hallazgos

## Resumen observado

- Se completaron 30/30 ejecuciones secuenciales, sin reintentos y con intervalo de 3 s.
- Clasificación: 16 `PARTIAL_SUCCESS` (53.33%), 14 `FAILURE` (46.67%), 0 `SUCCESS`, 0 `ZERO_RESULTS`.
- Las 16 respuestas utilizables produjeron 10 resultados cada una; total 160, media 5.33, p50 por nearest-rank 10.
- Todos los fallos públicos fueron `no_usable_results`; cada uno tuvo `providers_used=[searxng]` y provider parcial. No se observó error explícito de transporte, parsing o timeout en este conjunto.

## Anomalías con evidencia

- Para la misma consulta, el resultado puede alternar entre 10 resultados y `no_usable_results`: Q01–Q07 y Q10 mostraron rango de resultados 10 entre repeticiones.
- Q09 falló en sus tres repeticiones con cero resultados utilizables; esto describe el comportamiento observado, no una causa upstream.
- Todas las ejecuciones con resultados fueron parciales según AMATL; la interfaz pública del benchmark no expone qué motor ni error upstream produjo dicha parcialidad.

## AMATL frente a SearXNG upstream

- AMATL: aplicó el contrato público de forma consistente: cuando hubo resultados, respondió `partial_success`; cuando no los hubo con provider parcial, emitió `failure/no_usable_results`.
- Upstream/SearXNG: la fuente de la parcialidad y la selección/estado de motores no son observables desde la interfaz normal sin instrumentación, que este baseline no introdujo.

## NOT_OBSERVABLE

`http_status`, `searxng_results`, `mapped_items`, `unresponsive_engines_count`, nombres de motores no responsivos y tipos de error upstream. No se calcularon frecuencias de motores/errores porque no se expusieron en las 30 respuestas normales.

## Limitaciones

- Muestra de 30 ejecuciones, una instancia y una ventana temporal; no permite generalizar fuera de ella.
- La selección efectiva de motores queda en SearXNG y no se expone por AMATL.
- `elapsed_ms` es la latencia publicada por AMATL, no una medición de red descompuesta.
- El baseline mide disponibilidad, volumen, latencia y parcialidad; no evalúa calidad o relevancia semántica.
