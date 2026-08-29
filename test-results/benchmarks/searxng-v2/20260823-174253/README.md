Generated benchmark artifact — AMATL SearXNG Baseline v2 post-change — do not treat as project documentation.

# Baseline SearXNG v2 — post-change

## Estado

`NOT_COMPARABLE`

- Inicio: 2026-08-23T17:46:42-07:00.
- Commit AMATL: `48d0a9a24b3365e996a3b5e63eb3792fe70ed57a`.
- Dataset: copia byte-idéntica de `../../searxng-v1/20260823-153023/dataset.json`.
- Fixture: `../../searxng/20260823-145832/amatl-isolated.toml`, con proveedor único `searxng` y Marginalia excluida.
- Diseño planeado: 30 posiciones, Q01–Q10 × 3, secuenciales y con intervalo de 3 s.

## Violación de integridad observada

Se registraron 47 intentos, no 30. Las 30 posiciones planeadas están presentes, pero 17 posiciones tienen un segundo registro: Q01–Q03 R3 y Q04–Q10 R2/R3. Por ello se violaron el número de ejecuciones, el orden y el intervalo requeridos. No se borraron, reemplazaron ni reintentaron registros para ocultar esta condición; no se emitieron más consultas después de detectarla.

Las métricas v2 son descriptivas de los 47 registros observados y no constituyen una réplica válida de v1. La comparación final es `NOT_COMPARABLE` por este confounder material de integridad experimental.

## Límites de medición

No se diagnosticó ni corrigió el estado observado. No se modificaron AMATL, SearXNG, motores, fixture, dataset, timeouts o configuración. Marginalia no aparece en proveedores usados ni fallidos. Tras el experimento, DuckDuckGo, Mojeek y Qwant seguían efectivamente deshabilitados.

Los cuerpos de respuesta no se conservaron: `runs.jsonl` contiene sólo métricas públicas y códigos de error.
