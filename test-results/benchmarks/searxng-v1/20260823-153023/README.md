Generated benchmark artifact — AMATL SearXNG Baseline v1 — do not treat as project documentation.

# Baseline SearXNG v1

- Inicio: 2026-08-23T15:30:23-07:00
- Commit AMATL: `48d0a9a24b3365e996a3b5e63eb3792fe70ed57a`
- Versión AMATL: `0.1.0-rc.1`
- Provider único: `searxng` (`searxng-v1`), aprobado en el fixture.
- Interfaz: `target/debug/amatl --config-file test-results/searxng/20260823-145832/amatl-isolated.toml search <query> --json`.
- Configuración de control: persistencia, historial, caché de provider, caché de documentos y telemetría persistente deshabilitadas; concurrencia global/per-provider 1; reintentos 0; timeout provider 20 000 ms; timeout global 45 000 ms.
- Motores configurados observados antes del baseline: habilitados `duckduckgo`, `mojeek`, `qwant`; deshabilitados `brave`, `google cse`, `startpage`, `bing`. La selección efectiva queda en SearXNG porque AMATL no envía `engines` ni `categories`.
- Dataset: 10 consultas congeladas en `dataset.json`, 3 rondas, 30 ejecuciones secuenciales en orden Q01..Q10 por ronda.
- Intervalo constante entre solicitudes: 3 segundos.

## Regla de clasificación

- `PARTIAL_SUCCESS`: al menos un resultado final y `providers_partial` no vacío.
- `SUCCESS`: al menos un resultado final y sin provider parcial.
- `ZERO_RESULTS`: ejecución completada, cero resultados finales y sin error explícito de provider/transporte/parsing/timeout.
- `FAILURE`: error explícito de provider/transporte/parsing/timeout u otra condición de fallo (`status=failure`).

## Observabilidad

La interfaz normal expone `elapsed_ms`, `results`, `status`, `providers_partial` y errores agregados. `http_status`, `searxng_results`, `mapped_items`, `unresponsive_engines_count` y detalles de motores son `NOT_OBSERVABLE` en este baseline: no se instrumentó código.
