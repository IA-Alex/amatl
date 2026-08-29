# Campaña SearXNG: caracterización, diagnóstico y baseline (2026-08-23)

## 1. Propósito

Consolidar auditablemente la campaña del 23 de agosto de 2026 contra SearXNG desde AMATL. Este documento sólo sintetiza evidencia histórica: no incorpora nuevas mediciones ni transforma `UNKNOWN` en hechos.

## 2. Alcance

La campaña cubrió caracterización, diagnóstico de routing y adapter, una repetición limitada, baseline operativo, validación individual de motores, cambio de configuración, auditoría estática y un intento de observación runtime. No evaluó relevancia semántica, calidad de resultados ni Marginalia. Si aparece Marginalia, es sólo la observación incidental `provider_rate_limit`; no se incluye ninguna credencial.

## 3. Entorno y estado evaluado

- AMATL: commit `48d0a9a24b3365e996a3b5e63eb3792fe70ed57a` en los artefactos.
- Provider: `searxng` / adapter `searxng-v1`. AMATL envió `q`, `format=json` y `pageno=1`, sin `engines` ni `categories` para búsquedas generales.
- Corridas principales: secuenciales, sin reintentos, con persistencia, historial, cachés y telemetría persistente deshabilitados.
- **Baseline v1 — pre-change:** anterior a deshabilitar DuckDuckGo, Mojeek y Qwant.
- **Current state — post-change, not yet benchmarked:** posterior al cambio. No existe Baseline SearXNG v2 ni evidencia comparativa de mejora o empeoramiento.

## 4. Metodología

Cada conclusión conserva `evidencia → OBSERVATION → INFERENCE o ROOT_CAUSE → limitación`. `ROOT_CAUSE` sólo se usa con causalidad demostrada por el flujo o código inspeccionado; una asociación es `INFERENCE`. `UNKNOWN`, `BLOCKED` y `NOT_OBSERVABLE` conservan su significado literal.

```text
CONFIGURED ≠ ELIGIBLE ≠ SCHEDULED ≠ EXECUTED ≠ CONTRIBUTOR
```

Una cuenta configurada no demuestra selección, ejecución ni contribución de resultados.

## 5. Cronología de pruebas

| Fase | Inicio | Evidencia primaria | Resultado |
| --- | --- | --- | --- |
| 1. Caracterización | 14:58:32 | `test-results/searxng/20260823-145832/` | 3 consultas; una llegó a SearXNG sin resultados utilizables. |
| 2. Routing | 15:04:42 | `test-results/searxng-diagnostic/20260823-150442/` | Los filtros excluyeron SearXNG antes del adapter. |
| 3. Adapter/mapping | 15:10:35 | `test-results/searxng-diagnostic/20260823-151035-mapping/` | HTTP 200 y `results=0`; sin pérdida posterior observable. |
| 4. Motores/repetición | 15:16:06 | `test-results/searxng-diagnostic/20260823-151606-engines/` | 10 resultados; DuckDuckGo `access denied`. |
| 5. Baseline v1 | 15:30:23 | `test-results/benchmarks/searxng-v1/20260823-153023/` | 30 corridas pre-change. |
| 6. Actividad individual | 15:46:39 | `test-results/searxng-diagnostic/20260823-154639-engine-activity/` | DuckDuckGo falló; Qwant activo; Mojeek `UNKNOWN`. |
| 7. Cambio posterior | 16:17:54 | `test-results/searxng-configuration/20260823-161754-disable-engines/` | Se deshabilitaron sólo tres motores. |
| 8. Inventario/contrato | 16:36:46 | `test-results/searxng-diagnostic/20260823-163646-effective-engines/` | 111 → 18 → hasta 17; `success + 0 results`. |
| 9. Runtime por motor | posterior | `test-results/searxng-diagnostic/20260823-165200-runtime-engines/` | `BLOCKED` antes de SearXNG. |

## 6. Caracterización inicial

**Evidencia.** La batería tuvo tres consultas. `rust async` registró SearXNG en `providers_used` y `providers_partial`, `elapsed_ms=816`, cero resultados y `no_usable_results`. `tokio site:docs.rs` y la consulta con `lang:/region:/after:/filetype:` devolvieron `no_available_provider`, sin provider y con `elapsed_ms=0`.

**OBSERVATION.** Una de tres consultas llegó a SearXNG; dos fueron excluidas antes de una solicitud al provider. La primera acabó en `failure/no_usable_results`.

**INFERENCE.** Los dos casos filtrados no son un fallo de SearXNG.

**Limitación.** Body HTTP y `unresponsive_engines` fueron `NOT_OBSERVABLE`; canonicalización y deduplicación tampoco fueron observables porque no entraron resultados.

## 7. Diagnóstico de routing

**Evidencia.** El diagnóstico registró capabilities falsas para `site_filter`, `language`, `region`, `time_range` y `file_filter`; `AdaptiveRouter::recommend` exige los filtros requeridos antes de seleccionar.

**ROOT_CAUSE.** Los dos casos filtrados quedaron sin provider elegible y emitieron `no_available_provider` sin intento a SearXNG. La traducción aproximada o ignorada de filtros del adapter no fue alcanzable.

**Limitación.** Esto explica el routing, no el comportamiento upstream de SearXNG.

## 8. Diagnóstico del adapter/mapping

**Evidencia.** Una ejecución instrumentada de `rust async` registró HTTP 200; `results=0`; `answers=0`; `unresponsive_engines=2`; `mapped_items=0`; `final_usable_results=0`; `failure/no_usable_results`. La instrumentación fue retirada y el SHA-256 final coincidió con el previo.

**OBSERVATION.** El primer cero apareció en el vector `results` deserializado de SearXNG; tampoco había respuestas para mapear.

**ROOT_CAUSE (de esta ejecución).** No hubo pérdida en mapping, normalización ni etapas posteriores: el adapter recibió cero resultados y produjo cero items.

**Limitación.** Es `UNKNOWN` por qué SearXNG devolvió cero y por qué informó dos motores no responsivos; no se preservaron nombres, body ni atribución upstream.

## 9. Baseline SearXNG v1 — pre-change

**Evidencia.** Se completaron 30/30 corridas secuenciales: diez consultas, tres rondas, intervalo de tres segundos y sin reintentos.

| Medida | Valor |
| --- | ---: |
| Planned / completed runs | 30 / 30 |
| Success rate | 0.00% |
| Partial-success rate | 53.33% (16/30) |
| Zero-result rate | 0.00% |
| Failure rate | 46.67% (14/30) |
| Results mean / p50 | 5.33 / 10 |
| Latency mean / p50 / p95 | 885.37 ms / 721 ms / 2295 ms |

**OBSERVATION.** Las 16 ejecuciones utilizables fueron todas `partial_success` y devolvieron diez resultados. Las 14 restantes fueron `failure/no_usable_results`; Q09 falló en sus tres repeticiones. `success rate=0%` significa que no hubo `SUCCESS` sin provider parcial, no que ninguna búsqueda produjera resultados.

**INFERENCE.** Bajo la configuración previa, la disponibilidad efectiva fue degradada y variable: Q01–Q07 y Q10 alternaron entre 10 y 0 resultados.

**Limitación.** El baseline no evaluó relevancia semántica. HTTP, conteos internos, mapping, identidades de motores y errores upstream fueron `NOT_OBSERVABLE` desde la interfaz normal.

## 10. Diagnóstico de motores

**Evidencia.** Una repetición limitada anterior devolvió HTTP 200, diez resultados, un motor no responsivo y `partial_success`: DuckDuckGo / `access denied`.

**OBSERVATION.** `results=0` no fue determinista. Los dos motores no responsivos de la ejecución de cero resultados son `UNKNOWN`: sólo se guardó el conteo dos.

| Motor | Clasificación | Evidencia y alcance |
| --- | --- | --- |
| DuckDuckGo | `CONFIRMED_ATTEMPT_FAILED` | HTTP 200 de SearXNG con cero resultados y `access denied`; log: intento POST y HTTP 403. |
| Qwant | `CONFIRMED_ACTIVE` | Consulta selectiva sólo Qwant: 10 resultados. |
| Mojeek | `UNKNOWN` | Selección explícita HTTP 200/0, sin error ni atribución en log. |

**INFERENCE.** La disponibilidad de motores puede contribuir a la parcialidad, pero no hay `ROOT_CAUSE` para el patrón completo 10/0 del baseline.

**Limitación.** `200/0` de Mojeek no prueba funcionamiento ni fallo. La validación selectiva no revela los motores de búsquedas generales del baseline.

## 11. Cambio de configuración posterior

**Evidencia.** En `/etc/searxng/settings.yml`, con backup `/etc/searxng/settings.yml.bak-20260823-161754-disable-engines`, sólo DuckDuckGo, Mojeek y Qwant cambiaron de `disabled: false` a `disabled: true`. La validación efectiva posterior los resolvió deshabilitados.

**OBSERVATION.** El cambio ocurrió después del Baseline v1 y no habilitó ni añadió motores, categorías o cambios de AMATL.

**Conclusión.** No se mezclan métricas pre-change con la configuración post-change: el estado actual aún no fue benchmarked.

**Limitación.** Una consulta normal posterior produjo SearXNG HTTP 200, cero resultados y AMATL `success`; no es un Baseline v2 ni comparación de rendimiento.

## 12. Inventario de motores efectivos

**Evidencia.** La auditoría post-change contó 111 motores configurados y habilitados. Como AMATL no envió categorías ni motores, la selección estática `general` produjo 18 candidatos. Wikidata tuvo INIT HTTP 403 y su processor no se registró.

**OBSERVATION.**

```text
111 configured enabled → 18 general static candidates → at most 17 registrable
```

Los 93 restantes pertenecían exclusivamente a categorías no generales bajo esta clasificación.

**Conclusión.** No es correcto describir 111 motores como 111 fuentes participando simultáneamente en una búsqueda AMATL.

**Limitación.** Para los 17 restantes, `SCHEDULED`, `EXECUTED` y `CONTRIBUTOR` son `UNKNOWN`.

## 13. Contrato `success + 0 results`

**Evidencia.** La observación post-change y la auditoría describen que HTTP 200 con `results=[]`, `answers=[]` y `unresponsive_engines=[]` se mapea a `Success`; sin provider failed/partial, degradación ni falta de provider, AMATL permite `success` con cero resultados.

**OBSERVATION.** `success + 0 results` es comportamiento observado e implementado del contrato actual.

**Conclusión.** No contradice el Baseline v1: allí los ceros tuvieron SearXNG parcial (`unresponsive_engines` no vacío) y AMATL emitió `failure/no_usable_results`.

**Limitación.** No se clasifica como bug: no hay especificación registrada que contradiga el contrato.

## 14. Prueba runtime inconclusa

**Evidencia.** La única invocación normal para observar runtime por motor terminó con `SearXNG transport error: provider network request failed` / `provider_unavailable`. La sonda temporal no creó datos de scheduler ni processor.

**BLOCKED.** No alcanzó SearXNG: no observó motores scheduled, started, completed, contributors ni fallos individuales.

**Conclusión.** Es un fallo de transporte `AMATL → SearXNG`, anterior al scheduler/processor; no prueba un fallo de motores SearXNG. Los 17 candidatos mantienen participación runtime `UNKNOWN`.

**Limitación.** No se reintentó.

## 15. Hallazgos demostrados

- Los filtros explícitos pueden excluir SearXNG por capabilities declaradas.
- La ejecución de mapping HTTP 200 tenía cero resultados desde la respuesta deserializada, sin pérdida demostrada posterior en AMATL.
- Hubo variabilidad: una repetición produjo diez resultados y DuckDuckGo reportó `access denied`.
- DuckDuckGo tiene intento fallido confirmado (HTTP 403), Qwant actividad confirmada con diez resultados y Mojeek permanece `UNKNOWN`.
- AMATL puede comunicarse funcionalmente con SearXNG: existen ejecuciones históricas con resultados.
- El Baseline v1 pre-change describe disponibilidad efectiva degradada, no relevancia.

## 16. Elementos no determinados

- La causa upstream de `results=0` y de los dos `unresponsive_engines` de mapping.
- Las identidades de esos dos motores históricos.
- Los motores realmente scheduled, executed o contributors entre los 17 candidatos post-change.
- El efecto comparativo del cambio: no hay Baseline v2.

## 17. Limitaciones

Una instancia, ventana temporal y muestras acotadas no permiten generalización. La latencia publicada por AMATL no descompone la red. Los diagnósticos posteriores aportan evidencia puntual, no causalidad general. Marginalia queda fuera del alcance: `provider_rate_limit` sólo ayuda a interpretar estados globales.

## 18. Estado actual

AMATL ha demostrado comunicación funcional previa con SearXNG y resultados utilizables en algunas ejecuciones. El Baseline v1 es **pre-change**. DuckDuckGo, Mojeek y Qwant están deshabilitados **post-change**. Hay 18 candidatos generales estáticos y hasta 17 registrables, pero su participación runtime es `UNKNOWN` porque la última observación quedó `BLOCKED` antes de SearXNG. No existe Baseline SearXNG v2 ni evidencia para afirmar que la configuración actual mejoró o empeoró SearXNG.

## 19. Referencias a artefactos

- Caracterización: [`test-results/searxng/20260823-145832/`](../../test-results/searxng/20260823-145832/)
- Routing: [`test-results/searxng-diagnostic/20260823-150442/`](../../test-results/searxng-diagnostic/20260823-150442/)
- Mapping: [`test-results/searxng-diagnostic/20260823-151035-mapping/`](../../test-results/searxng-diagnostic/20260823-151035-mapping/)
- Repetición/motores: [`test-results/searxng-diagnostic/20260823-151606-engines/`](../../test-results/searxng-diagnostic/20260823-151606-engines/)
- Baseline v1: [`test-results/benchmarks/searxng-v1/20260823-153023/`](../../test-results/benchmarks/searxng-v1/20260823-153023/)
- Actividad individual: [`test-results/searxng-diagnostic/20260823-154639-engine-activity/`](../../test-results/searxng-diagnostic/20260823-154639-engine-activity/)
- Cambio de configuración: [`test-results/searxng-configuration/20260823-161754-disable-engines/`](../../test-results/searxng-configuration/20260823-161754-disable-engines/)
- Inventario y contrato: [`test-results/searxng-diagnostic/20260823-163646-effective-engines/`](../../test-results/searxng-diagnostic/20260823-163646-effective-engines/)
- Runtime bloqueado: [`test-results/searxng-diagnostic/20260823-165200-runtime-engines/`](../../test-results/searxng-diagnostic/20260823-165200-runtime-engines/)
