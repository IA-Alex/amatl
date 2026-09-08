Generated test artifact — SearXNG characterization — do not treat as project documentation.

# Hallazgos

## OBSERVATION

- La configuración aislada listó `searxng` como `available`; los otros providers quedaron deshabilitados.
- La consulta general `rust async` seleccionó SearXNG y AMATL la reportó en `providers_used` y `providers_partial`, pero no produjo resultados utilizables. La respuesta completa está en `response-01-rust-async.json`.
- Las consultas con `site:` y con `lang:/region:/after:/filetype:` devolvieron `no_available_provider`, con listas vacías de providers usados/parciales/fallidos y `elapsed_ms: 0`. Por ello no alcanzaron SearXNG.
- El canary de provider para la consulta general falló sin una respuesta utilizable. La interfaz `search --json`, usada para conservar la evidencia estructurada, devolvió el contrato de error completo.

## MEASUREMENT

- Consultas de caracterización ejecutadas: 3, secuenciales, sin reintentos.
- Éxitos: 0/3. Resultados: 0. URLs canónicas únicas: 0. Duplicados confirmados y posibles: 0.
- Una consulta seleccionó SearXNG y tomó 816 ms según `elapsed_ms` de AMATL; las otras dos fueron rechazadas por el router en 0 ms.
- No se observó timeout ni código HTTP/provider expuesto por AMATL en la batería. Los códigos de contrato fueron `no_usable_results` (1) y `no_available_provider` (2).

## INFERENCE

La inspección de `crates/amatl-core/src/providers/searxng.rs` muestra que SearXNG declara `site_filter`, `language`, `region`, `time_range` y `file_filter` como no soportados. Junto con las dos respuestas `no_available_provider`, esto indica que el enrutador excluye el único provider habilitado antes de que sea observable la traducción aproximada/ignorada de filtros del adapter. No es una medición de la instancia SearXNG.

## ERROR

- En sandbox, el canary de referencia falló inmediatamente con error de transporte de provider; se repitió fuera del sandbox para evitar atribuir esa limitación al provider.
- Fuera del sandbox, el canary siguió fallando y la búsqueda equivalente obtuvo `no_usable_results` con SearXNG parcial y cero resultados.

## BLOCKED

- Capturar el cuerpo HTTP original de SearXNG y `unresponsive_engines`: AMATL no lo expone en su CLI. Una llamada directa a SearXNG sería una interfaz distinta y queda fuera de la restricción de usar sólo AMATL.
- Medir canonicalización o deduplicación sobre datos de SearXNG: no hubo resultados que llegaran a esas etapas.
- Pruebas de carga, estrés, concurrencia elevada, reintentos, modificación de configuración o de la instancia: no ejecutadas por las restricciones del encargo.

## Limitaciones

- Muestra pequeña de tres consultas y una sola instancia/ventana temporal.
- `elapsed_ms` es el valor publicado por AMATL; no incluye de manera fiable el tiempo total de proceso observado por el lanzador.
- La respuesta pública no distingue qué motor upstream estuvo no disponible ni expone el cuerpo bruto de SearXNG.
- Los filtros no se pueden caracterizar a nivel de adapter mediante la interfaz normal mientras el router no seleccione el provider.
