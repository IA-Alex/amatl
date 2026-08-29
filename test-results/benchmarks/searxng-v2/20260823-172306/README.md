Generated benchmark artifact — AMATL SearXNG Baseline v2 post-change — do not treat as project documentation.

# Baseline SearXNG v2 — post-change

## Estado

`ABORT BENCHMARK`

- Inicio/precheck: 2026-08-23T17:23:06-07:00.
- Commit AMATL observado: `48d0a9a24b3365e996a3b5e63eb3792fe70ed57a`.
- Versión AMATL declarada por el workspace: `0.1.0-rc.1`.
- Dataset: copia exacta de `../../searxng-v1/20260823-153023/dataset.json`; contiene Q01–Q10.
- Mecanismo de aislamiento disponible: el fixture existente `test-results/searxng/20260823-145832/amatl-isolated.toml` selecciona únicamente `searxng`, deshabilita persistencia/cachés y conserva timeout provider de 20 000 ms, timeout global de 45 000 ms, concurrencia 1 y reintentos 0.
- Configuración viva post-change: **NO CONFIRMADA**. `/etc/searxng/settings.yml` no fue legible desde el entorno de medición y no se detectó listener TCP local en el puerto 8888.

No se compiló AMATL ni se generó tráfico. Por tanto no se modificaron AMATL, la configuración, motores, SearXNG ni Marginalia.

## Criterio de aborto

No se pudo confirmar en estado actual que `duckduckgo`, `mojeek` y `qwant` estén en `disabled: true`, ni que SearXNG esté disponible para atender el fixture aislado. La evidencia histórica del cambio no sustituye esta comprobación viva. Las condiciones esenciales de v1 no pueden reproducirse verificablemente; ejecutar 30 solicitudes produciría un v2 inválido.

No hay métricas v2, estabilidad ni comparación v1↔v2 calculadas.
