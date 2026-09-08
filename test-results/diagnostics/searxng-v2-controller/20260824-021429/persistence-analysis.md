# Análisis de persistencia

## Diseño observado

El runner inspeccionado no abre `runs.jsonl`, no serializa `ExecutionRecord` por posición y no llama `flush()` ni `fsync()`. `execute_plan()` mantiene los registros en `ExecutionState.records` en memoria y sólo devuelve el estado al terminar el bucle. Las funciones `write_*_artifacts()` escriben artefactos completos al final de una operación (con `Path.write_text()`), sin append por posición ni fsync explícito.

Clasificación del runner: **BATCH_AT_END** para sus artefactos; para `runs.jsonl` de la campaña, el escritor no está presente en este archivo, por lo que el mecanismo exacto es **UNKNOWN**.

## Ciclo solicitado

Resultado AMATL → parse: sí, dentro de `AmatlProcessExecutor`; record object → memoria: sí, `_execute_position()` agrega `ExecutionRecord`; serialization → write → flush → fsync por posición: no existe. Escritura final de artefactos: sí, después de la ejecución completa en las funciones `write_*_artifacts()`; apertura previa de `runs.jsonl`: no demostrada; escritura por posición: no demostrada; archivo creado sólo al terminar: sí para los artefactos de este runner.

Una interrupción antes del final perdería los registros mantenidos sólo en memoria. Ese diseño **podría** explicar 9 reportadas/0 durables si el escritor de campaña dependía de un batch final, pero no se puede confirmar porque el escritor de `runs.jsonl` y la excepción/señal que interrumpió la campaña no están en la evidencia local. El `runs.jsonl` presente contiene 30 líneas y por tanto no evidencia cero persistencia de esa campaña.

**PERSISTENCE_LOSS_CAUSE: LIKELY_CAUSE** — pérdida por buffering/batch-at-end es compatible con el código, pero el vínculo causal con la campaña descrita no está demostrado.

## Corrección conceptual mínima (no aplicada)

Ejecutar posición → construir/validar identidad `(benchmark_id, provider, query_id, repetition)` → append exactamente una vez → `flush()` → `fsync()` → marcar completada → siguiente posición. Mantener sin retries, sin resume, sin duplicados, máximo 30 y bloqueo previo del intento #31 y de posiciones ajenas. Persistencia parcial conserva evidencia pero la campaña sigue `INVALID`.

## Validación posterior (no ejecutada)

Test A: interrupción mock tras #9; planned 30, invocations 9, durable 9, unique 9, duplicates 0, retries 0, sequence 1..9, #10 ausente, `INVALID_INTERRUPTED`.

Test B: mock completo; 30 invocations/durable/unique, duplicates 0, missing 0, retries 0, sequence 1..30.

Test C: tras 30, intento #31 rechazado antes del executor; durable permanece 30.
