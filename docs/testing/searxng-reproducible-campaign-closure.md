# Cierre formal — Campaña reproducible SearXNG (2026-08-24)

**Fase:** Investigación de infraestructura de benchmark reproducible sobre el
provider SearXNG.
**Estado:** CERRADA.
**Autor del cierre:** israelalexish@gmail.com (vía Claude Code, actuando como
operador de la investigación).
**Fecha de cierre:** 2026-08-23.

## A. Objetivo de la fase

Validar que la infraestructura de benchmark (runner + manifest + dataset +
config snapshot) produce campañas reproducibles y autoverificables contra el
provider SearXNG, y determinar si los fallos observados en campañas
anteriores eran atribuibles a un defecto en AMATL o en el runner.

## B. Qué se validó

- Runner reproducible: `tools/benchmark_plan_runner.py`
  (sha256 `d8d27a53438bd90c93800dd8fad9d5f158f6c81f64d1323cae372300a5903736`).
- Manifest de campaña:
  `test-results/benchmarks/searxng-reproducible/20260824-053003/campaign-manifest.json`
  (schema `amatl-benchmark-runner/2`).
- Config snapshot: `config-snapshot.toml` dentro del mismo directorio de
  campaña, referenciado desde
  `test-results/searxng/20260823-145832/amatl-isolated.toml`
  (sha256 `8b749d4dcb26f3dfeced048acd7e91e57bd48ccee1a2c7244f4555a29e1918c1`).
- Dataset sha256: `92ce0225bfde9f55d7a4abce156d9fc4f4bdab885c1eedf30bfa8b1cc6899b3a`
  (`test-results/benchmarks/searxng-v2/20260823-190535/dataset.json`).
- Binary AMATL sha256: `25fcdf9fc41a712652eb65e1b53e1d2d43a7f659b7481a8f378e5102607a3d1f`
  (`amatl 0.1.0-rc.1`).
- Clasificación autoverificable de cada posición (`runs.jsonl`, un registro
  por posición con status explícito).
- Pacing: 30 posiciones, `inter_request_interval_seconds: 3`, 3 repeticiones
  × 10 queries = 30 (29 intervalos de espera entre posiciones).
- Retries: `retries: 0` — sin reintentos automáticos que enmascaren fallos
  transitorios.
- Aislamiento: config y binario propios de la campaña, sin dependencias
  cruzadas con otras campañas (`v1`, `v2`) del mismo directorio de
  benchmarks.

## C. Resultado de campaña

- Directorio: `test-results/benchmarks/searxng-reproducible/20260824-053003/`.
- 30/30 posiciones ejecutadas.
- 30/30 registros con `status: executor_failure` (verificado directamente
  contra `runs.jsonl`, no solo declarado).
- No hay resultados usables (`no_usable_results`) en ninguna posición.

## D. Diagnóstico

- SearXNG local operativo y accesible en `127.0.0.1:8888` (contenedor
  `searxng`, red `host`, `HTTP 200` en raíz).
- Los motores web generales estaban deshabilitados salvo `duckduckgo`, que
  ya venía habilitado en producción desde el 2026-08-18 (ver comentario en
  `settings.yml`, no fue una activación introducida por esta investigación).
- Se realizó una consulta directa contra SearXNG con `duckduckgo`
  habilitado: `HTTP 200`, `results=0`, motor reportando `access denied`.
- No se ejecutó AMATL después de esa consulta directa fallida.
- No se ejecutó una segunda campaña tras el diagnóstico.
- No se demostró defecto en AMATL ni en el runner: el comportamiento
  observado es consistente con bloqueo/denegación de acceso en el motor
  externo (`duckduckgo`), no con un error de construcción de solicitud,
  parseo o ejecución por parte de AMATL o del runner.

## E. Interpretación

La campaña demuestra la **reproducibilidad del mecanismo experimental**
(runner, manifest, pacing, clasificación autoverificable), **no** la calidad
ni el rendimiento funcional de la búsqueda vía SearXNG. Con los motores
disponibles en el momento de la ejecución, SearXNG no entregó resultados
usables; esto es un dato sobre disponibilidad de motores upstream, no una
conclusión sobre la corrección de AMATL o del runner.

## F. Estado

```
AMATL_BENCHMARK_INFRASTRUCTURE=VALIDATED
SEARXNG_PROVIDER_OPERATIONAL_STATUS=BLOCKED_BY_UPSTREAM_AVAILABILITY
FUNCTIONAL_BASELINE=NOT_ESTABLISHED
HISTORICAL_V1_V2=PROVENANCE_INCOMPLETE
```

## G. Decisión

Se cierra esta fase. No se continuará probando motores externos de forma
secuencial dentro de esta investigación. La reevaluación de SearXNG (y de
qué motores web generales son viables de forma sostenida) queda como
trabajo independiente, fuera de esta fase.

## Rollback aplicado al cierre

Se revirtió exclusivamente la línea `disabled` del motor `duckduckgo` en el
`settings.yml` efectivo del contenedor `searxng` (`false → true`), restaurando
byte a byte el estado capturado en `/tmp/settings_now.yml`
(sha256 `3eae24fa315f4f4b06b769ee582c56d7ea4c2fd0f85d6ef70f2c489e7c916a63`,
verificado exacto tras el rollback). Se reinició el contenedor para aplicar
el cambio. Tras el reinicio: contenedor `Up`, `HTTP 200` en raíz,
`/config` reporta `duckduckgo.enabled=false`. No se ejecutó `/search` ni
ninguna otra consulta después del rollback.

## Preservación de evidencia

El directorio `test-results/benchmarks/searxng-reproducible/20260824-053003/`
no fue modificado ni eliminado. Clasificación asignada:
`REPRODUCIBLE_FAILED_CAMPAIGN`. No se reclasifica como baseline funcional.
Los directorios `searxng-v1` y `searxng-v2` bajo `test-results/benchmarks/`
tampoco fueron modificados.
