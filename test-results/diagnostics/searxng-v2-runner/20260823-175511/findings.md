# Hallazgos

## Evidencia y reconstrucción

`runs.jsonl` contiene una línea de metadatos y 47 registros de intento. Todos
los 47 tienen `exit_code=1`, `classification=FAILURE`, `elapsed_ms=0`,
`final_results=0`, `providers_used=[]`, `providers_failed=["searxng"]` y el
error público `provider_unavailable`. La matriz y la secuencia completa están
normalizadas en `reconstruction.json`; no se copian consultas ni cuerpos de respuesta.

El plan esperado era tres rondas completas Q01..Q10: 30 posiciones. El plan
real, inferido en el orden del JSONL, contiene 47 invocaciones. Primero emite
R1 completa, R2 completa y Q01..Q03-R3. Después intercala Q04..Q10-R3 con una
segunda emisión de su R2 (a +1 s), y finalmente vuelve a emitir Q01..Q10-R3.
Por tanto los duplicados exactos son Q01..Q03-R3 (3) y Q04..Q10-R2/R3 (14):
`3 + (7 x 2) = 17`.

La duplicación se prueba por primera vez en el plan efectivo de invocación y
no en el resultado: cada duplicado posee su propio timestamp, separado por
segundos, y su propia entrada JSONL. La persistencia de JSONL sólo es el
registro de tales invocaciones; no hay evidencia de que haya clonado entradas.

## Procedencia y límite de atribución

La fixture existente es
`test-results/searxng/20260823-145832/amatl-isolated.toml`, con
`max_retries=0`, `global_concurrency=1` y sólo `searxng`. El README de esa
fixture documenta la interfaz `target/debug/amatl --config-file
amatl-isolated.toml search <query> --json`; el preflight de build prueba que
ese binario se construyó antes del benchmark. No existe runner, script,
Make/just task, wrapper, log de shell, archivo de comandos ni entrada Git que
contenga el mecanismo que produjo `20260823-174253`. Por ello no es posible
demostrar un bloque/función responsable ni distinguir con certeza entre un
loop externo, una invocación manual adicional o un plan externo mal generado.

Clasificación: `COMMAND_INVOCATION_DUPLICATION` es el primer nivel probado;
`PLAN_GENERATION_DUPLICATION`, `RUNNER_LOOP_DUPLICATION`,
`RESULT_CAPTURE_DUPLICATION` y `ARTIFACT_APPEND_DUPLICATION` no son
observables. La raíz es `ROOT_CAUSE_UNKNOWN`, no confirmada ni probable.

## Retry

AMATL soporta retry recuperable en `crates/amatl-core/src/execution.rs`, pero
la fixture lo desactiva explícitamente (`max_retries=0`); el metadato del JSONL
también declara `retries=0`. En consecuencia hay `CONFIGURED_RETRY=false` y
`EXECUTED_RETRY=NOT_OBSERVED`. Las 17 entradas son `DUPLICATE_NOT_RETRY`:
son invocaciones CLI separadas, no reintentos internos de un mismo provider
call.

## FAILURE + elapsed_ms=0 y alcance de red

`SearchOrchestrator::search` inicia `Instant::now()` y construye
`SearchResponse.elapsed_ms` con `started.elapsed().as_millis() as u64`
(`crates/amatl-core/src/execution.rs`). Es un valor entero truncado para toda
la orquestación, no la latencia de SearXNG. En estos registros sí es un valor
medido por AMATL, pero inferior a 1 ms, no una latencia útil de proveedor.

La cadena de código prueba: CLI `search` -> `AmatlService::search` ->
`SearchOrchestrator` -> `SearXngProvider::search` ->
`HttpTransport::execute`. La presencia de `providers_failed=["searxng"]` y
`provider_unavailable` corresponde al error de transporte que el adaptador
convierte a `ProviderErrorKind::Unavailable`; no es un rechazo de router ni
un error de parsing/captura. Por ello AMATL, el adaptador y la llamada al
transporte están confirmados. Los artefactos no conservan el log de transporte,
HTTP status o evidencia de recepción: la entrega de una solicitud a SearXNG
es `NOT_OBSERVABLE`; no debe inferirse desde el estado actual del servicio.

## Corrección y dry-run posterior

No se puede proponer una modificación mínima de archivo/función sin un runner
localizable. La corrección mínima conceptual, condicionada a recuperar ese
runner, es generar una lista inmutable de 30 tuplas `(round, query_id)` una
sola vez, validar unicidad/cardinalidad antes de ejecutar y consumir cada
tupla exactamente una vez; el append debe rechazar una posición ya emitida.

El dry-run offline posterior debe no invocar AMATL ni transporte y verificar:

- Expected positions: 30
- Generated commands: 30
- Unique positions: 30
- Duplicates: 0
- Missing positions: 0
- Network requests: 0

No se aplicó corrección ni se ejecutó dry-run en esta auditoría.
